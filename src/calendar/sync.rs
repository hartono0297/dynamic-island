//! The calendar sync thread: every Google call runs here, never on the UI
//! thread. It answers `SyncRequest`s with `Event::Calendar` snapshots (and,
//! for saves, `WM_APP_EDITOR_STATUS` to the editor window), keeps the local
//! cache, and re-fetches the last requested range every ten minutes.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use super::{CalStatus, CalendarSnapshot, EventDraft, api, range_for};
use crate::config::{self, Config};
use crate::google::http::Http;
use crate::google::oauth::{self, AuthError};
use crate::model::{CalView, Event};

/// Posted to the event editor after a save: `wparam` 1 = saved, 0 = failed with
/// `lparam` a `Box<String>` raw pointer (Indonesian message) that the receiver
/// frees; `lparam` is 0 on success. (WM_APP + 1..4 are taken by the island.)
pub const WM_APP_EDITOR_STATUS: u32 = WM_APP + 5;

/// Values table: re-fetch the last range every 10 minutes.
const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);
/// A repeat request for the range fetched successfully this recently is skipped.
const REPEAT_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum SyncRequest {
    /// Fetch `[from, to)` and make it the range the timer re-fetches.
    Range {
        from: NaiveDate,
        to: NaiveDate,
    },
    /// Patch `event_id` (in `calendar_id`, default `primary`), or insert a new
    /// event in `primary` when `event_id` is `None`. `reply` is the editor's HWND.
    Save {
        event_id: Option<String>,
        calendar_id: Option<String>,
        draft: EventDraft,
        reply: isize,
    },
    /// Interactive Google login (opens the browser), then a fetch.
    Connect,
    Shutdown,
}

/// `%APPDATA%\DynamicIsland\calendar-cache.json`, next to the config file.
pub fn cache_path() -> PathBuf {
    config::path().with_file_name("calendar-cache.json")
}

/// A missing or malformed cache is `None`.
pub fn load_cache(path: &Path) -> Option<CalendarSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()
}

/// Pretty JSON to a sibling `.tmp` file, then renamed over the cache.
pub fn save_cache(path: &Path, snapshot: &CalendarSnapshot) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(snapshot).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// The snapshot after a fetch of `[from, to)`: fresh events on success; on
/// failure the previous events and range stay, with the status the error means.
pub fn merge_fetch(
    prev: &CalendarSnapshot,
    result: Result<Vec<super::CalEvent>, AuthError>,
    from: NaiveDate,
    to: NaiveDate,
    now: DateTime<Local>,
) -> CalendarSnapshot {
    match result {
        Ok(events) => CalendarSnapshot {
            status: CalStatus::Ok,
            events,
            from,
            to,
            fetched_at: Some(now),
        },
        Err(e) => CalendarSnapshot {
            status: status_for(&e),
            ..prev.clone()
        },
    }
}

fn status_for(e: &AuthError) -> CalStatus {
    match e {
        AuthError::NotConfigured => CalStatus::NotConfigured,
        AuthError::NeedsLogin => CalStatus::NeedsLogin,
        AuthError::Reconnect => CalStatus::Reconnect,
        AuthError::Network(_) => CalStatus::Offline,
    }
}

/// The editor's error line. Messages that start with "HTTP " came from Google;
/// any other network error means the request never got through.
pub fn save_error_message(e: &AuthError) -> String {
    match e {
        AuthError::NotConfigured => "Gagal simpan: Google belum diatur".into(),
        AuthError::NeedsLogin => "Gagal simpan: hubungkan Google dulu".into(),
        AuthError::Reconnect => "Gagal simpan: hubungkan ulang Google".into(),
        AuthError::Network(m) if m.starts_with("HTTP ") => {
            format!("Gagal simpan: Google menolak ({m})")
        }
        AuthError::Network(_) => "Gagal simpan: tidak ada koneksi".into(),
    }
}

/// Starts the sync thread. It sends the cached snapshot right away (status
/// `Offline` until a fetch succeeds); the caller asks for the first range.
pub fn spawn_sync(
    tx: Sender<Event>,
    wake: impl Fn() + Send + 'static,
    cfg: Config,
) -> Sender<SyncRequest> {
    spawn_with(tx, wake, cfg, cache_path()).0
}

fn spawn_with(
    tx: Sender<Event>,
    wake: impl Fn() + Send + 'static,
    cfg: Config,
    cache: PathBuf,
) -> (Sender<SyncRequest>, JoinHandle<()>) {
    let (req_tx, req_rx) = mpsc::channel();
    let handle =
        std::thread::spawn(move || Worker::new(tx, Box::new(wake), cfg, cache).run(req_rx));
    (req_tx, handle)
}

struct Worker {
    tx: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
    cfg: Config,
    cache: PathBuf,
    /// Created on first use; WinRT setup failures are retried next time.
    http: Option<Http>,
    snapshot: CalendarSnapshot,
    /// The last requested range, re-fetched by the timer.
    range: Option<(NaiveDate, NaiveDate)>,
    last_ok: Option<((NaiveDate, NaiveDate), Instant)>,
    next_refresh: Option<Instant>,
}

impl Worker {
    fn new(tx: Sender<Event>, wake: Box<dyn Fn() + Send>, cfg: Config, cache: PathBuf) -> Self {
        let status = if configured(&cfg) {
            CalStatus::Offline
        } else {
            CalStatus::NotConfigured
        };
        let snapshot = load_cache(&cache).map_or_else(
            || CalendarSnapshot::empty(status, Local::now().date_naive()),
            |cached| CalendarSnapshot { status, ..cached },
        );
        Worker {
            tx,
            wake,
            cfg,
            cache,
            http: None,
            snapshot,
            range: None,
            last_ok: None,
            next_refresh: None,
        }
    }

    fn run(mut self, rx: Receiver<SyncRequest>) {
        self.emit();
        loop {
            let first = match self.next_refresh {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(req) => req,
                    Err(RecvTimeoutError::Timeout) => {
                        self.refresh(true);
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(req) => req,
                    Err(_) => return,
                },
            };
            // Everything queued meanwhile is handled as one batch, so a burst of
            // navigation fetches only the latest range.
            let batch: Vec<SyncRequest> = std::iter::once(first).chain(rx.try_iter()).collect();
            if !self.handle(batch) {
                return;
            }
        }
    }

    /// False on `Shutdown`.
    fn handle(&mut self, batch: Vec<SyncRequest>) -> bool {
        // `Some(force)` when the batch ends with a fetch.
        let mut fetch: Option<bool> = None;
        for req in batch {
            match req {
                SyncRequest::Range { from, to } => {
                    self.range = Some((from, to));
                    fetch = Some(fetch.unwrap_or(false));
                }
                SyncRequest::Save {
                    event_id,
                    calendar_id,
                    draft,
                    reply,
                } => {
                    let result = self.save(event_id.as_deref(), calendar_id.as_deref(), &draft);
                    post_status(reply, result.map_err(|e| save_error_message(&e)));
                    fetch = Some(true);
                }
                SyncRequest::Connect => {
                    if self.connect() {
                        fetch = Some(true);
                    }
                }
                SyncRequest::Shutdown => return false,
            }
        }
        if let Some(force) = fetch {
            self.refresh(force);
        }
        true
    }

    /// Fetches the last requested range (this month before any request).
    fn refresh(&mut self, force: bool) {
        if !configured(&self.cfg) {
            self.set_status(CalStatus::NotConfigured);
            return;
        }
        let range = self
            .range
            .unwrap_or_else(|| range_for(CalView::Month, Local::now().date_naive()));
        let recent = self
            .last_ok
            .is_some_and(|(r, at)| r == range && at.elapsed() < REPEAT_WINDOW);
        if recent && !force {
            return;
        }
        let result = ensure_http(&mut self.http).and_then(|http| {
            let token = oauth::access_token(http, &self.cfg)?;
            api::fetch_range(http, &token, range.0, range.1)
        });
        let ok = result.is_ok();
        self.snapshot = merge_fetch(&self.snapshot, result, range.0, range.1, Local::now());
        if ok {
            // A failed write only costs the offline view after a restart.
            let _ = save_cache(&self.cache, &self.snapshot);
            self.last_ok = Some((range, Instant::now()));
        }
        self.next_refresh = Some(Instant::now() + REFRESH_EVERY);
        self.emit();
    }

    fn save(
        &mut self,
        event_id: Option<&str>,
        calendar_id: Option<&str>,
        draft: &EventDraft,
    ) -> Result<(), AuthError> {
        if !configured(&self.cfg) {
            return Err(AuthError::NotConfigured);
        }
        let http = ensure_http(&mut self.http)?;
        let token = oauth::access_token(http, &self.cfg)?;
        match event_id {
            Some(id) => api::patch_event(http, &token, calendar_id.unwrap_or("primary"), id, draft),
            None => api::insert_event(http, &token, draft),
        }
    }

    /// Runs the browser login. True when it succeeded and a fetch should follow;
    /// on failure the snapshot shows why.
    fn connect(&mut self) -> bool {
        if !configured(&self.cfg) {
            self.set_status(CalStatus::NotConfigured);
            return false;
        }
        let result = ensure_http(&mut self.http).and_then(|http| {
            oauth::login(
                http,
                &self.cfg.google_client_id,
                &self.cfg.google_client_secret,
            )
        });
        match result {
            Ok(_) => true,
            Err(e) => {
                self.set_status(status_for(&e));
                false
            }
        }
    }

    fn set_status(&mut self, status: CalStatus) {
        self.snapshot.status = status;
        self.emit();
    }

    fn emit(&self) {
        if self.tx.send(Event::Calendar(self.snapshot.clone())).is_ok() {
            (self.wake)();
        }
    }
}

fn configured(cfg: &Config) -> bool {
    !cfg.google_client_id.trim().is_empty()
}

fn ensure_http(slot: &mut Option<Http>) -> Result<&Http, AuthError> {
    if slot.is_none() {
        *slot = Some(Http::new().map_err(|e| AuthError::Network(e.message()))?);
    }
    Ok(slot.as_ref().expect("just set"))
}

/// Tells the editor `reply` how the save went; a 0 handle means nobody waits.
fn post_status(reply: isize, result: Result<(), String>) {
    if reply == 0 {
        return;
    }
    let (wparam, lparam) = match result {
        Ok(()) => (1, 0),
        Err(message) => (0, Box::into_raw(Box::new(message)) as isize),
    };
    let hwnd = HWND(reply as *mut core::ffi::c_void);
    let posted = unsafe {
        PostMessageW(
            Some(hwnd),
            WM_APP_EDITOR_STATUS,
            WPARAM(wparam),
            LPARAM(lparam),
        )
    };
    if posted.is_err() && lparam != 0 {
        // The editor is gone; nobody else will free the message.
        drop(unsafe { Box::from_raw(lparam as *mut String) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{CalEvent, EventTime};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// A fresh directory under the system temp dir; never `%APPDATA%`.
    fn temp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dynamic-island-cal-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> CalendarSnapshot {
        CalendarSnapshot {
            status: CalStatus::Ok,
            events: vec![
                CalEvent {
                    id: "a".into(),
                    calendar_id: "primary".into(),
                    title: "Libur".into(),
                    start: EventTime::AllDay(date(2026, 10, 26)),
                    end: EventTime::AllDay(date(2026, 10, 29)),
                    description: String::new(),
                    color: Some(0x4F8CFF),
                    editable: false,
                },
                CalEvent {
                    id: "b".into(),
                    calendar_id: "team".into(),
                    title: "Rapat \"mingguan\"".into(),
                    start: EventTime::At(
                        DateTime::parse_from_rfc3339("2026-10-25T10:00:00-07:00")
                            .unwrap()
                            .with_timezone(&Local),
                    ),
                    end: EventTime::At(
                        DateTime::parse_from_rfc3339("2026-10-25T11:00:00-07:00")
                            .unwrap()
                            .with_timezone(&Local),
                    ),
                    description: "baris 1\nbaris 2".into(),
                    color: None,
                    editable: true,
                },
            ],
            from: date(2026, 9, 28),
            to: date(2026, 11, 9),
            fetched_at: Some(Local::now()),
        }
    }

    #[test]
    fn cache_round_trip() {
        let dir = temp_dir();
        let path = dir.join("calendar-cache.json");
        assert_eq!(load_cache(&path), None, "missing file");
        let snap = sample();
        save_cache(&path, &snap).unwrap();
        let loaded = load_cache(&path);
        let leftover_tmp = dir.join("calendar-cache.json.tmp").exists();
        std::fs::write(&path, "{ not json").unwrap();
        let corrupt = load_cache(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(loaded, Some(snap));
        assert!(!leftover_tmp);
        assert_eq!(corrupt, None);
    }

    #[test]
    fn fetch_error_keeps_cached_events_offline() {
        let prev = sample();
        let now = Local::now();
        let merged = merge_fetch(
            &prev,
            Err(AuthError::Network("no route".into())),
            date(2026, 10, 19),
            date(2026, 10, 26),
            now,
        );
        assert_eq!(merged.status, CalStatus::Offline);
        assert_eq!(merged.events, prev.events);
        assert_eq!((merged.from, merged.to), (prev.from, prev.to));
        assert_eq!(merged.fetched_at, prev.fetched_at);

        for (err, status) in [
            (AuthError::Reconnect, CalStatus::Reconnect),
            (AuthError::NeedsLogin, CalStatus::NeedsLogin),
            (AuthError::NotConfigured, CalStatus::NotConfigured),
        ] {
            let merged = merge_fetch(&prev, Err(err), prev.from, prev.to, now);
            assert_eq!(merged.status, status);
            assert_eq!(merged.events, prev.events);
        }
    }

    #[test]
    fn fetch_success_replaces_events_and_range() {
        let prev = sample();
        let now = Local::now();
        let merged = merge_fetch(
            &prev,
            Ok(vec![]),
            date(2026, 10, 19),
            date(2026, 10, 26),
            now,
        );
        assert_eq!(
            merged,
            CalendarSnapshot {
                status: CalStatus::Ok,
                events: vec![],
                from: date(2026, 10, 19),
                to: date(2026, 10, 26),
                fetched_at: Some(now),
            }
        );
    }

    #[test]
    fn save_errors_are_indonesian() {
        assert_eq!(
            save_error_message(&AuthError::Network(
                "The server name cannot be resolved".into()
            )),
            "Gagal simpan: tidak ada koneksi"
        );
        assert_eq!(
            save_error_message(&AuthError::Network("HTTP 403: Forbidden".into())),
            "Gagal simpan: Google menolak (HTTP 403: Forbidden)"
        );
        assert_eq!(
            save_error_message(&AuthError::Reconnect),
            "Gagal simpan: hubungkan ulang Google"
        );
    }

    fn worker(cache: PathBuf) -> (Worker, Receiver<Event>, Arc<AtomicUsize>) {
        let (tx, rx) = mpsc::channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let wake = Box::new(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        // Empty client id: the worker must answer without calling Google.
        (Worker::new(tx, wake, Config::default(), cache), rx, wakes)
    }

    fn statuses(rx: &Receiver<Event>) -> Vec<CalStatus> {
        rx.try_iter()
            .map(|e| match e {
                Event::Calendar(s) => s.status,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn queued_ranges_coalesce_to_the_latest() {
        let dir = temp_dir();
        let (mut w, rx, wakes) = worker(dir.join("calendar-cache.json"));
        let range = |d| SyncRequest::Range {
            from: date(2026, 10, d),
            to: date(2026, 10, d + 1),
        };
        assert!(w.handle(vec![range(1), range(2), range(3)]));
        assert_eq!(w.range, Some((date(2026, 10, 3), date(2026, 10, 4))));
        assert_eq!(statuses(&rx), [CalStatus::NotConfigured], "one answer");
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(!w.handle(vec![range(5), SyncRequest::Shutdown]));
        assert!(!dir.exists(), "nothing written while not configured");
    }

    #[test]
    fn not_configured_thread_answers_without_google() {
        let dir = temp_dir();
        let cache = dir.join("calendar-cache.json");
        save_cache(&cache, &sample()).unwrap();
        let before = std::fs::read(&cache).unwrap();

        let (tx, rx) = mpsc::channel();
        let (req, handle) = spawn_with(tx, || {}, Config::default(), cache.clone());
        let next = || match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Calendar(s) => s,
            other => panic!("unexpected {other:?}"),
        };
        let first = next();
        assert_eq!(first.status, CalStatus::NotConfigured);
        assert_eq!(first.events, sample().events, "cache loaded at start");

        req.send(SyncRequest::Range {
            from: date(2026, 10, 19),
            to: date(2026, 10, 26),
        })
        .unwrap();
        assert_eq!(next().status, CalStatus::NotConfigured);
        req.send(SyncRequest::Connect).unwrap();
        assert_eq!(next().status, CalStatus::NotConfigured);
        req.send(SyncRequest::Save {
            event_id: None,
            calendar_id: None,
            draft: crate::calendar::draft_from(&sample().events[1]),
            reply: 0,
        })
        .unwrap();
        assert_eq!(next().status, CalStatus::NotConfigured);
        req.send(SyncRequest::Shutdown).unwrap();
        handle.join().unwrap();
        let after = std::fs::read(&cache).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(before, after, "cache untouched");
    }

    #[test]
    fn configured_worker_starts_offline_from_cache() {
        let dir = temp_dir();
        let cache = dir.join("calendar-cache.json");
        save_cache(&cache, &sample()).unwrap();
        let (tx, _rx) = mpsc::channel();
        let cfg = Config {
            google_client_id: "id.apps.googleusercontent.com".into(),
            ..Config::default()
        };
        let w = Worker::new(tx, Box::new(|| {}), cfg, cache);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(w.snapshot.status, CalStatus::Offline);
        assert_eq!(w.snapshot.events, sample().events);
    }
}
