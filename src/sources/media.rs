use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::Result;

use crate::lyrics::timeline_moved;
use crate::model::{Event, MediaCommand, MediaInfo, Timeline};

/// Thumbnails larger than this are dropped rather than held in memory.
const MAX_ART_BYTES: u64 = 2 * 1024 * 1024;

/// What a WinRT event handler reports to the watcher thread. Handlers run on
/// WinRT threads, so they only send one of these and return.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Signal {
    /// The manager's current session, or its set of sessions, changed.
    Session,
    /// The current session's title, artist or thumbnail changed.
    Properties,
    /// The current session's play/pause state changed.
    Playback,
    /// The current session's position, length or seek state changed.
    Timeline,
}

/// The work implied by a burst of signals, coalesced so that a flurry of
/// events costs one rebuild.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Refresh {
    /// Re-subscribe to whichever session is now current.
    session: bool,
    /// Re-read title, artist and thumbnail.
    properties: bool,
    /// Re-read the timeline (cheap: no art).
    timeline: bool,
}

impl Refresh {
    fn add(&mut self, signal: Signal) {
        match signal {
            Signal::Session => {
                self.session = true;
                self.properties = true;
                self.timeline = true;
            }
            // A new track starts a new timeline, whichever event comes first.
            Signal::Properties => {
                self.properties = true;
                self.timeline = true;
            }
            // Play/pause may come with or without a timeline update.
            Signal::Playback | Signal::Timeline => self.timeline = true,
        }
    }
}

/// Remembers the last value sent so that repeated events with identical
/// content are not forwarded.
#[derive(Default)]
struct LastSent(Option<Option<MediaInfo>>);

impl LastSent {
    /// True (and remembers `next`) when `next` differs from what was last sent.
    fn changed(&mut self, next: &Option<MediaInfo>) -> bool {
        if self.0.as_ref() == Some(next) {
            return false;
        }
        self.0 = Some(next.clone());
        true
    }
}

/// Title, artist and thumbnail of the current session; re-read only when the
/// session reports new media properties, not on every playback change.
struct Properties {
    title: String,
    artist: String,
    art: Option<Vec<u8>>,
}

/// Handlers registered on one session; removed when dropped.
struct SessionSubscription {
    session: Session,
    properties_token: Option<i64>,
    playback_token: Option<i64>,
    timeline_token: Option<i64>,
}

impl SessionSubscription {
    fn new(session: Session, signals: &Sender<Signal>) -> Self {
        let s = signals.clone();
        let properties_token = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Properties);
                Ok(())
            }))
            .ok();
        let s = signals.clone();
        let playback_token = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Playback);
                Ok(())
            }))
            .ok();
        let s = signals.clone();
        let timeline_token = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Timeline);
                Ok(())
            }))
            .ok();
        Self {
            session,
            properties_token,
            playback_token,
            timeline_token,
        }
    }
}

impl Drop for SessionSubscription {
    fn drop(&mut self) {
        if let Some(token) = self.properties_token {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = self.playback_token {
            let _ = self.session.RemovePlaybackInfoChanged(token);
        }
        if let Some(token) = self.timeline_token {
            let _ = self.session.RemoveTimelinePropertiesChanged(token);
        }
    }
}

/// State owned by the watcher thread.
struct Watcher {
    manager: Manager,
    manager_token: Option<i64>,
    /// `SessionsChanged`: a session opened or closed, which can change the
    /// current one without `CurrentSessionChanged` firing.
    sessions_token: Option<i64>,
    signals: Sender<Signal>,
    subscription: Option<SessionSubscription>,
    properties: Option<Properties>,
}

impl Watcher {
    fn new(manager: Manager, signals: Sender<Signal>) -> Self {
        let s = signals.clone();
        let manager_token = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Session);
                Ok(())
            }))
            .ok();
        let s = signals.clone();
        let sessions_token = manager
            .SessionsChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Session);
                Ok(())
            }))
            .ok();
        Self {
            manager,
            manager_token,
            sessions_token,
            signals,
            subscription: None,
            properties: None,
        }
    }

    /// Applies `refresh` and returns what the island should now show.
    fn refresh(&mut self, refresh: Refresh) -> Option<MediaInfo> {
        if refresh.session {
            // Drop the old handlers before subscribing to the new session.
            self.subscription = None;
            self.properties = None;
            self.subscription = self
                .manager
                .GetCurrentSession()
                .ok()
                .map(|session| SessionSubscription::new(session, &self.signals));
        }
        let session = &self.subscription.as_ref()?.session;
        if refresh.properties || self.properties.is_none() {
            // On a failed read keep the previous values for this session.
            if let Ok(properties) = read_properties(session) {
                self.properties = Some(properties);
            }
        }
        let playing = shown_playing(
            session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus()),
        )?;
        let properties = self.properties.as_ref()?;
        Some(MediaInfo {
            title: properties.title.clone(),
            artist: properties.artist.clone(),
            playing,
            art: properties.art.clone(),
        })
    }

    /// The current session's timeline; `None` without a session or when it
    /// reports none.
    fn timeline(&self) -> Option<Timeline> {
        let session = &self.subscription.as_ref()?.session;
        read_timeline(session).ok().flatten()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(token) = self.manager_token {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
        if let Some(token) = self.sessions_token {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
    }
}

/// Whether a session with this playback status is playing; `None` when it is
/// closed or stopped and so should not be shown at all. An unreadable status
/// keeps the session on screen as paused.
fn shown_playing(status: Result<PlaybackStatus>) -> Option<bool> {
    match status {
        Ok(PlaybackStatus::Closed | PlaybackStatus::Stopped) => None,
        Ok(status) => Some(status == PlaybackStatus::Playing),
        Err(_) => Some(false),
    }
}

/// Watches the system media session on a background thread and sends
/// `Event::Media` whenever what is playing changes: once at start, then on
/// session, track and play/pause changes. A changed timeline (a seek, a new
/// track, play/pause) follows as `Event::MediaTimeline`, without re-reading
/// the art; steady playback reported again is not sent (`timeline_moved`).
/// The thread blocks between events.
pub fn spawn_watcher(tx: Sender<Event>, wake: impl Fn() + Send + Sync + 'static) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let Ok(manager) = Manager::RequestAsync().and_then(|op| op.join()) else {
            return;
        };
        let (signals, signal_rx) = mpsc::channel();
        let mut watcher = Watcher::new(manager, signals);
        let mut last = LastSent::default();
        let mut last_timeline: Option<Timeline> = None;
        let mut refresh = Refresh::default();
        refresh.add(Signal::Session);
        loop {
            let info = watcher.refresh(refresh);
            let (present, playing) = (info.is_some(), info.as_ref().is_some_and(|m| m.playing));
            let mut sent = false;
            if last.changed(&info) {
                if tx.send(Event::Media(info)).is_err() {
                    return;
                }
                sent = true;
            }
            if refresh.timeline || sent {
                let timeline = if present { watcher.timeline() } else { None };
                let now = Instant::now();
                if timeline_moved(last_timeline.as_ref(), timeline.as_ref(), playing, now) {
                    if tx.send(Event::MediaTimeline(timeline)).is_err() {
                        return;
                    }
                    last_timeline = timeline;
                    sent = true;
                }
            }
            if sent {
                wake();
            }
            refresh = next_refresh(&signal_rx);
        }
    });
}

/// Blocks for the next signal, then folds in any already queued behind it.
fn next_refresh(signals: &Receiver<Signal>) -> Refresh {
    let mut refresh = Refresh::default();
    // The watcher holds a sender, so `recv` only returns once a signal arrives.
    if let Ok(signal) = signals.recv() {
        refresh.add(signal);
    }
    for signal in signals.try_iter() {
        refresh.add(signal);
    }
    refresh
}

/// Sends a transport command to the current media session on a short-lived
/// thread, so the caller never waits on WinRT. Errors are ignored.
pub fn send_command(cmd: MediaCommand) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _ = try_send_command(cmd);
    });
}

fn try_send_command(cmd: MediaCommand) -> Result<bool> {
    let session = Manager::RequestAsync()?.join()?.GetCurrentSession()?;
    let op = match cmd {
        MediaCommand::Prev => session.TrySkipPreviousAsync()?,
        MediaCommand::PlayPause => session.TryTogglePlayPauseAsync()?,
        MediaCommand::Next => session.TrySkipNextAsync()?,
    };
    op.join()
}

fn read_properties(session: &Session) -> Result<Properties> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let art = props
        .Thumbnail()
        .ok()
        .and_then(|thumbnail| read_thumbnail(&thumbnail).ok().flatten());
    Ok(Properties {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        art,
    })
}

/// 100 ns ticks between 1601-01-01 (WinRT `DateTime`) and 1970-01-01.
const TICKS_TO_UNIX: i64 = 116_444_736_000_000_000;

/// The session's timeline, positions relative to its start. `None` when it
/// reports no length and no position (players without timeline support).
fn read_timeline(session: &Session) -> Result<Option<Timeline>> {
    let t = session.GetTimelineProperties()?;
    let start = t.StartTime()?.Duration;
    let ms = |ticks: i64| u64::try_from(ticks.saturating_sub(start) / 10_000).unwrap_or(0);
    let position_ms = ms(t.Position()?.Duration);
    let end_ms = ms(t.EndTime()?.Duration);
    if position_ms == 0 && end_ms == 0 {
        return Ok(None);
    }
    let updated_ticks = t.LastUpdatedTime()?.UniversalTime;
    let age = Duration::from_millis(age_ms(now_ticks(), updated_ticks));
    Ok(Some(Timeline {
        position_ms,
        end_ms,
        updated: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
    }))
}

/// The wall clock as WinRT `DateTime` ticks.
fn now_ticks() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    TICKS_TO_UNIX.saturating_add(i64::try_from(since.as_nanos() / 100).unwrap_or(i64::MAX))
}

/// How long ago `updated` was, in ms: 0 when unset or in the future, at most
/// a day (a stale stamp must not throw the position hours ahead).
fn age_ms(now_ticks: i64, updated_ticks: i64) -> u64 {
    if updated_ticks <= 0 {
        return 0;
    }
    let ms = now_ticks.saturating_sub(updated_ticks) / 10_000;
    u64::try_from(ms).unwrap_or(0).min(86_400_000)
}

/// The thumbnail's encoded image bytes; `None` when empty or over the size cap.
fn read_thumbnail(thumbnail: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = thumbnail.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if !art_size_ok(size) {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(Some(bytes))
}

fn art_size_ok(size: u64) -> bool {
    size > 0 && size <= MAX_ART_BYTES
}

/// Diagnostic mode for `--probe-media`: prints every media update until Ctrl+C.
#[cfg(debug_assertions)]
pub fn probe() {
    let (tx, rx) = mpsc::channel();
    spawn_watcher(tx, || {});
    println!("Watching media sessions; press Ctrl+C to stop.");
    for event in rx {
        match event {
            Event::Media(None) => println!("None"),
            Event::Media(Some(m)) => println!(
                "{} | {} | playing={} | art={} bytes",
                m.title,
                m.artist,
                m.playing,
                m.art.as_ref().map_or(0, Vec::len)
            ),
            Event::MediaTimeline(Some(t)) => println!(
                "  timeline {} / {} ms, updated {} ms ago",
                t.position_ms,
                t.end_ms,
                t.updated.elapsed().as_millis()
            ),
            Event::MediaTimeline(None) => println!("  no timeline"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str, playing: bool, art: Option<Vec<u8>>) -> Option<MediaInfo> {
        Some(MediaInfo {
            title: title.to_string(),
            artist: "Artist".to_string(),
            playing,
            art,
        })
    }

    #[test]
    fn first_value_is_always_sent_even_none() {
        let mut last = LastSent::default();
        assert!(last.changed(&None));
        assert!(!last.changed(&None));
    }

    #[test]
    fn identical_info_is_not_resent() {
        let mut last = LastSent::default();
        assert!(last.changed(&info("A", true, Some(vec![1, 2]))));
        assert!(!last.changed(&info("A", true, Some(vec![1, 2]))));
    }

    #[test]
    fn play_state_title_and_late_art_count_as_changes() {
        let mut last = LastSent::default();
        assert!(last.changed(&info("A", true, None)));
        assert!(last.changed(&info("A", false, None)));
        assert!(last.changed(&info("B", false, None)));
        assert!(last.changed(&info("B", false, Some(vec![9]))));
        assert!(last.changed(&None));
    }

    #[test]
    fn playback_and_timeline_signals_reread_only_the_timeline() {
        for signal in [Signal::Playback, Signal::Timeline] {
            let mut r = Refresh::default();
            r.add(signal);
            assert_eq!(
                r,
                Refresh {
                    timeline: true,
                    ..Refresh::default()
                },
                "{signal:?}"
            );
        }
    }

    #[test]
    fn new_properties_reread_the_timeline_too() {
        let mut r = Refresh::default();
        r.add(Signal::Properties);
        assert!(r.properties && r.timeline && !r.session);
    }

    #[test]
    fn timeline_age_is_clamped() {
        let now = TICKS_TO_UNIX + 1_000_000_000;
        assert_eq!(age_ms(now, now - 25_000_000), 2_500);
        assert_eq!(age_ms(now, 0), 0, "unset");
        assert_eq!(age_ms(now, now + 10_000_000), 0, "in the future");
        assert_eq!(age_ms(now, 1), 86_400_000, "at most a day");
        // The wall clock is past 2023 in WinRT ticks.
        assert!(now_ticks() > TICKS_TO_UNIX + 1_700_000_000 * 10_000_000);
    }

    #[test]
    fn session_change_implies_property_reread() {
        let mut r = Refresh::default();
        r.add(Signal::Playback);
        r.add(Signal::Session);
        assert_eq!(
            r,
            Refresh {
                session: true,
                properties: true,
                timeline: true
            }
        );
    }

    #[test]
    fn coalesces_queued_signals() {
        let (tx, rx) = mpsc::channel();
        tx.send(Signal::Playback).unwrap();
        tx.send(Signal::Properties).unwrap();
        tx.send(Signal::Playback).unwrap();
        let r = next_refresh(&rx);
        assert_eq!(
            r,
            Refresh {
                session: false,
                properties: true,
                timeline: true
            }
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn closed_or_stopped_session_shows_nothing() {
        assert_eq!(shown_playing(Ok(PlaybackStatus::Closed)), None);
        assert_eq!(shown_playing(Ok(PlaybackStatus::Stopped)), None);
    }

    #[test]
    fn live_session_reports_whether_it_plays() {
        assert_eq!(shown_playing(Ok(PlaybackStatus::Playing)), Some(true));
        for status in [
            PlaybackStatus::Paused,
            PlaybackStatus::Opened,
            PlaybackStatus::Changing,
        ] {
            assert_eq!(shown_playing(Ok(status)), Some(false), "{status:?}");
        }
        // An unreadable status keeps the session on screen, paused.
        assert_eq!(
            shown_playing(Err(windows::core::Error::empty())),
            Some(false)
        );
    }

    #[test]
    fn sessions_changed_resubscribes() {
        let mut r = Refresh::default();
        r.add(Signal::Session);
        assert!(r.session && r.properties);
    }

    #[test]
    fn art_size_cap() {
        assert!(!art_size_ok(0));
        assert!(art_size_ok(1));
        assert!(art_size_ok(MAX_ART_BYTES));
        assert!(!art_size_ok(MAX_ART_BYTES + 1));
    }
}
