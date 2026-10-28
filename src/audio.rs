//! The Mixer page's volumes: the default output device's master level and
//! the apps that play sound, through Core Audio on one worker thread (COM
//! stays on it; the UI thread only sends requests and gets rows back). The
//! page asks for a scan every second while it shows and sets a level or a
//! mute as the pointer asks. Everything that can be decided without Windows
//! (levels, the slider's maths, which sessions become rows, which requests
//! can be dropped) is a pure function below the thread.

use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    eMultimedia, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{Interface, PWSTR};

use crate::model::Event;

/// A wheel notch over a row moves the level this much.
pub const WHEEL_STEP: f32 = 0.05;
/// Rows on the page: the master level and this many apps.
pub const MAX_APPS: usize = 4;

/// What a row sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The default output device.
    Master,
    /// Windows' own sounds (the system sounds session).
    System,
    /// Every session of the program with this lower-case exe name.
    App(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixerRow {
    pub target: Target,
    /// The app's name; empty for the master level and the system sounds
    /// (the page names them).
    pub name: String,
    /// 0.0 to 1.0.
    pub level: f32,
    pub muted: bool,
}

// ---- pure helpers -------------------------------------------------------

/// A level kept in 0.0..=1.0; anything that is not a number is 0.
pub fn clamp_level(level: f32) -> f32 {
    if level.is_nan() { 0.0 } else { level.clamp(0.0, 1.0) }
}

/// The level as the whole percent the page shows.
pub fn percent(level: f32) -> u32 {
    (clamp_level(level) * 100.0).round() as u32
}

pub fn from_percent(percent: u32) -> f32 {
    clamp_level(percent as f32 / 100.0)
}

/// The level a press or drag at `x` gives a slider track from `track_x` of
/// width `track_w` (the same unit for all three): the left end is 0, the
/// right end 1, past either end is kept on it. Snaps to a whole percent.
pub fn slider_level(track_x: f32, track_w: f32, x: f32) -> f32 {
    if track_w <= 0.0 {
        return 0.0;
    }
    from_percent(percent((x - track_x) / track_w))
}

/// The level after `notches` wheel notches (positive: up, louder), on the
/// 5 % grid.
pub fn step_level(level: f32, notches: i32) -> f32 {
    let grid = (clamp_level(level) / WHEEL_STEP).round() as i32;
    let steps = (1.0 / WHEEL_STEP).round() as i32;
    let next = (grid + notches).clamp(0, steps);
    clamp_level(next as f32 * WHEEL_STEP)
}

/// One wheel notch over the music, in whole percent points.
pub const SCROLL_POINTS_PER_NOTCH: i32 = 2;

/// The percent points a wheel delta adds up to (120 a notch, positive up:
/// louder), keeping what does not make a point yet in `sum`, so a touchpad's
/// small deltas still turn the volume.
pub fn wheel_points(sum: &mut i32, delta: i32) -> i32 {
    wheel_units(sum, delta, 120 / SCROLL_POINTS_PER_NOTCH)
}

/// Whole wheel notches (120 each, positive up) a delta adds up to, keeping
/// the rest in `sum`: a Mixer row steps 5 % a notch, and a touchpad's small
/// deltas make a step once they add up to a notch.
pub fn wheel_notches(sum: &mut i32, delta: i32) -> i32 {
    wheel_units(sum, delta, 120)
}

/// `sum` plus `delta` cut into whole units of `per_unit`; the rest stays.
fn wheel_units(sum: &mut i32, delta: i32, per_unit: i32) -> i32 {
    *sum = sum.saturating_add(delta);
    let units = *sum / per_unit;
    *sum -= units * per_unit;
    units
}

/// `level` moved by `points` percent points, kept in 0..=100 %.
pub fn move_points(level: f32, points: i32) -> f32 {
    let now = percent(level) as i32;
    from_percent((now + points).clamp(0, 100) as u32)
}

/// "spotify" for `C:\Users\x\AppData\Roaming\Spotify\Spotify.exe`: the file
/// name without its extension, lower case. Empty for an empty path.
pub fn exe_key(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    stem.to_lowercase()
}

/// A program's name to show: its key with the first letter upper case.
pub fn display_name(key: &str) -> String {
    let mut chars = key.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One audio session as Core Audio reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct RawSession {
    pub pid: u32,
    /// `exe_key` of its process; empty when it could not be read.
    pub key: String,
    pub system: bool,
    pub active: bool,
    pub level: f32,
    pub muted: bool,
}

/// The page's rows: the master level first, then the apps (one row per
/// program, all its sessions set together; active ones first, then by
/// name), at most `MAX_APPS` of them, then the system sounds when they
/// play. A session whose program is unknown is left out. A program with
/// several sessions (one process can have more than one) is active if any
/// is, muted only if all are, and shows its first active session's level.
pub fn rows(master: (f32, bool), sessions: &[RawSession]) -> Vec<MixerRow> {
    let mut out = vec![MixerRow {
        target: Target::Master,
        name: String::new(),
        level: clamp_level(master.0),
        muted: master.1,
    }];
    let mut apps: Vec<(bool, MixerRow)> = Vec::new();
    let mut system: Option<MixerRow> = None;
    for s in sessions {
        if s.system {
            if s.active {
                system = Some(MixerRow {
                    target: Target::System,
                    name: String::new(),
                    level: clamp_level(s.level),
                    muted: s.muted,
                });
            }
            continue;
        }
        if s.key.is_empty() {
            continue;
        }
        let target = Target::App(s.key.clone());
        match apps.iter_mut().find(|(_, r)| r.target == target) {
            // Another session of the same program: a playing session's
            // level wins over a silent one's.
            Some((active, row)) => {
                if s.active && !*active {
                    row.level = clamp_level(s.level);
                }
                *active |= s.active;
                row.muted &= s.muted;
            }
            None => apps.push((
                s.active,
                MixerRow {
                    target,
                    name: display_name(&s.key),
                    level: clamp_level(s.level),
                    muted: s.muted,
                },
            )),
        }
    }
    apps.sort_by(|(a_on, a), (b_on, b)| b_on.cmp(a_on).then_with(|| a.name.cmp(&b.name)));
    out.extend(apps.into_iter().take(MAX_APPS).map(|(_, r)| r));
    out.extend(system);
    out
}

/// A fresh reading's rows in the order the page already shows (`shown`), so
/// a row does not move under the pointer between a scan and a click: rows
/// still there keep their place with the new values, new ones come after
/// them in the reading's order, gone ones drop. The master stays first and
/// the system sounds last.
pub fn keep_order(shown: &[MixerRow], fresh: Vec<MixerRow>) -> Vec<MixerRow> {
    let place = |row: &MixerRow| {
        shown
            .iter()
            .position(|s| s.target == row.target)
            .unwrap_or(usize::MAX)
    };
    let band = |row: &MixerRow| match row.target {
        Target::Master => 0,
        Target::App(_) => 1,
        Target::System => 2,
    };
    let mut out: Vec<(usize, MixerRow)> = fresh.into_iter().enumerate().collect();
    // New rows (no place yet) keep the reading's order among themselves.
    out.sort_by_key(|(i, row)| (band(row), place(row), *i));
    out.into_iter().map(|(_, row)| row).collect()
}

/// A reading asked for with request number `asked` is older than the last
/// level or mute the page sent (request number `last_set`): it may still
/// hold the value from before the change, so it does not replace the rows.
pub fn reading_is_stale(asked: u64, last_set: u64) -> bool {
    asked < last_set
}

/// Program names by process id, so a scan every second does not open every
/// process again. Only names that were read are kept, and only for the
/// processes the last scan saw (a gone process's id may be reused).
#[derive(Default)]
pub struct Names(std::collections::HashMap<u32, String>);

impl Names {
    /// `pid`'s `exe_key`, asking `read` only the first time; an empty
    /// answer (a protected or starting process) is asked again next time.
    pub fn key(&mut self, pid: u32, read: impl FnOnce(u32) -> String) -> String {
        if let Some(key) = self.0.get(&pid) {
            return key.clone();
        }
        let key = read(pid);
        if !key.is_empty() {
            self.0.insert(pid, key.clone());
        }
        key
    }

    /// Forgets every process not in `seen`.
    pub fn keep_only(&mut self, seen: &[u32]) {
        self.0.retain(|pid, _| seen.contains(pid));
    }
}

/// A request to the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    /// A reading, numbered so its answer can be told from an older one's.
    Scan(u64),
    Level(Target, f32),
    Mute(Target, bool),
}

/// The requests that matter out of a batch waiting in the queue, in the
/// order of their last mention: the last level and the last mute per target,
/// and one scan (a drag sends one per pointer move).
pub fn coalesce(batch: Vec<Req>) -> Vec<Req> {
    let same = |a: &Req, b: &Req| match (a, b) {
        (Req::Scan(_), Req::Scan(_)) => true,
        (Req::Level(x, _), Req::Level(y, _)) | (Req::Mute(x, _), Req::Mute(y, _)) => x == y,
        _ => false,
    };
    let mut out: Vec<Req> = Vec::new();
    for req in batch {
        out.retain(|r| !same(r, &req));
        out.push(req);
    }
    out
}

// ---- the worker thread --------------------------------------------------

/// The thread's handle. Dropping it ends the thread.
pub struct Audio {
    requests: Sender<Req>,
}

impl Audio {
    /// `None` when the thread could not start.
    pub fn spawn(tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> Option<Self> {
        let (requests, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || serve(&rx, &tx, wake));
        match spawned {
            Ok(_) => Some(Self { requests }),
            Err(error) => {
                eprintln!("vale: no audio thread: {error}");
                None
            }
        }
    }

    /// False when the thread is gone.
    pub fn request(&self, req: Req) -> bool {
        self.requests.send(req).is_ok()
    }
}

fn serve(rx: &Receiver<Req>, tx: &Sender<Event>, wake: impl Fn()) {
    // Without COM there is nothing to read: say "no device" to each scan.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mut names = Names::default();
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut scan = None;
        for req in coalesce(batch) {
            match req {
                Req::Scan(asked) => scan = Some(asked),
                Req::Level(target, level) => {
                    set(&target, Some(clamp_level(level)), None, &mut names);
                }
                Req::Mute(target, muted) => set(&target, None, Some(muted), &mut names),
            }
        }
        if let Some(asked) = scan {
            let found = if com { read(&mut names) } else { None };
            if tx.send(Event::Audio(asked, found)).is_err() {
                break;
            }
            wake();
        }
    }
    if com {
        unsafe { CoUninitialize() };
    }
}

fn default_device() -> Option<IMMDevice> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()
    }
}

/// The program's `exe_key` for a process id; empty when it cannot be read
/// (a protected or gone process).
fn process_key(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            HANDLE(handle.0),
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok {
            exe_key(&String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            String::new()
        }
    }
}

/// Every session of the default device, as the Core Audio reports it.
fn sessions(device: &IMMDevice, names: &mut Names) -> Vec<(IAudioSessionControl, RawSession)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            return out;
        };
        let Ok(list) = manager.GetSessionEnumerator() else {
            return out;
        };
        let count = list.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(control) = list.GetSession(i) else {
                continue;
            };
            // A session its app has closed: nothing plays or can be set.
            if control
                .GetState()
                .is_ok_and(|state| state == AudioSessionStateExpired)
            {
                continue;
            }
            let (Ok(c2), Ok(volume)) = (
                control.cast::<IAudioSessionControl2>(),
                control.cast::<ISimpleAudioVolume>(),
            ) else {
                continue;
            };
            let pid = c2.GetProcessId().unwrap_or(0);
            let raw = RawSession {
                pid,
                key: names.key(pid, process_key),
                // S_OK (0) is yes, S_FALSE (1) is no: both are "ok" to `is_ok`.
                system: c2.IsSystemSoundsSession().0 == 0,
                active: control
                    .GetState()
                    .is_ok_and(|state| state == AudioSessionStateActive),
                level: volume.GetMasterVolume().unwrap_or(1.0),
                muted: volume.GetMute().is_ok_and(|m| m.as_bool()),
            };
            out.push((control, raw));
        }
    }
    out
}

/// The rows now; `None` without an output device.
fn read(names: &mut Names) -> Option<Vec<MixerRow>> {
    let device = default_device()?;
    let master = unsafe {
        let volume = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        (
            volume.GetMasterVolumeLevelScalar().unwrap_or(0.0),
            volume.GetMute().is_ok_and(|m| m.as_bool()),
        )
    };
    let raw: Vec<RawSession> = sessions(&device, names)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    names.keep_only(&raw.iter().map(|s| s.pid).collect::<Vec<_>>());
    Some(rows(master, &raw))
}

/// Sets a level and/or a mute on `target`.
fn set(target: &Target, level: Option<f32>, muted: Option<bool>, names: &mut Names) {
    let Some(device) = default_device() else {
        return;
    };
    unsafe {
        if *target == Target::Master {
            let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else {
                return;
            };
            if let Some(level) = level {
                let _ = volume.SetMasterVolumeLevelScalar(level, std::ptr::null());
            }
            if let Some(muted) = muted {
                let _ = volume.SetMute(muted, std::ptr::null());
            }
            return;
        }
        for (control, raw) in sessions(&device, names) {
            let hit = match target {
                Target::System => raw.system,
                Target::App(key) => !raw.system && raw.key == *key,
                Target::Master => false,
            };
            if !hit {
                continue;
            }
            let Ok(volume) = control.cast::<ISimpleAudioVolume>() else {
                continue;
            };
            if let Some(level) = level {
                let _ = volume.SetMasterVolume(level, std::ptr::null());
            }
            if let Some(muted) = muted {
                let _ = volume.SetMute(muted, std::ptr::null());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_wheel_adds_up_to_two_percent_points_a_notch() {
        use super::*;
        let mut sum = 0;
        assert_eq!(wheel_points(&mut sum, 120), 2);
        assert_eq!(wheel_points(&mut sum, -120), -2);
        // A touchpad's small deltas keep what is not a point yet.
        let mut sum = 0;
        let total: i32 = (0..12).map(|_| wheel_points(&mut sum, 20)).sum();
        assert_eq!(total, 4, "12 x 20 = two notches");
        assert_eq!(sum, 0);
        let mut sum = 0;
        assert_eq!(wheel_points(&mut sum, 59), 0);
        assert_eq!(wheel_points(&mut sum, 1), 1);
        // Up then down cancels out.
        let mut sum = 0;
        assert_eq!(wheel_points(&mut sum, 30), 0);
        assert_eq!(wheel_points(&mut sum, -30), 0);
        assert_eq!(sum, 0);
    }

    #[test]
    fn the_level_moves_by_points_inside_zero_and_a_hundred() {
        use super::*;
        assert_eq!(percent(move_points(0.50, 2)), 52);
        assert_eq!(percent(move_points(0.51, -2)), 49);
        assert_eq!(percent(move_points(0.99, 5)), 100);
        assert_eq!(percent(move_points(0.01, -5)), 0);
        assert_eq!(percent(move_points(f32::NAN, 3)), 3);
        assert_eq!(move_points(1.0, 100), 1.0);
    }

    use super::*;

    fn raw(pid: u32, key: &str, active: bool, level: f32) -> RawSession {
        RawSession {
            pid,
            key: key.into(),
            system: false,
            active,
            level,
            muted: false,
        }
    }

    #[test]
    fn levels_are_kept_in_range_and_read_as_whole_percent() {
        assert_eq!(clamp_level(1.7), 1.0);
        assert_eq!(clamp_level(-0.2), 0.0);
        assert_eq!(clamp_level(f32::NAN), 0.0);
        assert_eq!(percent(0.624), 62);
        assert_eq!(percent(0.625), 63);
        assert_eq!(percent(2.0), 100);
        assert_eq!(from_percent(40), 0.4);
        assert_eq!(from_percent(900), 1.0);
    }

    #[test]
    fn the_slider_maps_the_pointer_onto_its_track() {
        assert_eq!(slider_level(100.0, 200.0, 100.0), 0.0);
        assert_eq!(slider_level(100.0, 200.0, 200.0), 0.5);
        assert_eq!(slider_level(100.0, 200.0, 300.0), 1.0);
        // Past either end is kept on it.
        assert_eq!(slider_level(100.0, 200.0, 40.0), 0.0);
        assert_eq!(slider_level(100.0, 200.0, 900.0), 1.0);
        // Whole percent; a track without width gives 0.
        assert_eq!(percent(slider_level(0.0, 300.0, 100.0)), 33);
        assert_eq!(slider_level(0.0, 0.0, 10.0), 0.0);
    }

    #[test]
    fn the_wheel_steps_on_the_five_percent_grid_within_range() {
        assert_eq!(percent(step_level(0.5, 1)), 55);
        assert_eq!(percent(step_level(0.5, -2)), 40);
        assert_eq!(percent(step_level(0.62, 1)), 65, "off the grid snaps first");
        assert_eq!(percent(step_level(0.98, 1)), 100);
        assert_eq!(percent(step_level(1.0, 3)), 100);
        assert_eq!(percent(step_level(0.02, -1)), 0);
        assert_eq!(percent(step_level(0.0, -1)), 0);
    }

    #[test]
    fn programs_are_named_by_their_exe() {
        assert_eq!(exe_key(r"C:\Users\x\AppData\Roaming\Spotify\Spotify.exe"), "spotify");
        assert_eq!(exe_key("C:/Program Files/Google/Chrome/chrome.exe"), "chrome");
        assert_eq!(exe_key("msedge"), "msedge");
        assert_eq!(exe_key(""), "");
        assert_eq!(display_name("spotify"), "Spotify");
        assert_eq!(display_name(""), "");
    }

    #[test]
    fn rows_put_the_master_first_and_one_row_per_program() {
        let sessions = [
            raw(10, "chrome", true, 0.45),
            raw(11, "chrome", false, 0.45),
            raw(10, "chrome", true, 0.45),
            raw(20, "spotify", true, 0.8),
            raw(30, "teams", false, 0.2),
            raw(40, "", true, 0.9),
        ];
        let got = rows((0.62, false), &sessions);
        let names: Vec<&str> = got.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["", "Chrome", "Spotify", "Teams"]);
        assert_eq!(got[0].target, Target::Master);
        assert_eq!(got[1].target, Target::App("chrome".into()));
        assert_eq!(got[0].level, 0.62);
    }

    #[test]
    fn rows_list_active_programs_first_cap_the_apps_and_add_system_sounds_when_they_play() {
        let mut sessions: Vec<RawSession> = (0..7)
            .map(|i| raw(100 + i, &format!("app{i}"), i >= 5, 0.5))
            .collect();
        sessions.push(RawSession {
            pid: 5,
            key: String::new(),
            system: true,
            active: true,
            level: 0.3,
            muted: true,
        });
        let got = rows((1.0, true), &sessions);
        assert_eq!(got.len(), 1 + MAX_APPS + 1);
        assert!(got[0].muted);
        // The two active ones lead, the rest by name.
        assert_eq!(got[1].name, "App5");
        assert_eq!(got[2].name, "App6");
        assert_eq!(got[3].name, "App0");
        assert_eq!(got.last().unwrap().target, Target::System);
        // Silent system sounds are not a row.
        sessions.last_mut().unwrap().active = false;
        assert_eq!(rows((1.0, false), &sessions).len(), 1 + MAX_APPS);
    }

    #[test]
    fn requests_are_cut_to_the_last_of_each_kind_per_target() {
        let spotify = Target::App("spotify".into());
        let batch = vec![
            Req::Level(Target::Master, 0.1),
            Req::Scan(1),
            Req::Level(spotify.clone(), 0.2),
            Req::Level(Target::Master, 0.3),
            Req::Mute(Target::Master, true),
            Req::Scan(4),
            Req::Mute(Target::Master, false),
        ];
        assert_eq!(
            coalesce(batch),
            vec![
                Req::Level(spotify, 0.2),
                Req::Level(Target::Master, 0.3),
                Req::Scan(4),
                Req::Mute(Target::Master, false),
            ],
            "the newest scan's number answers"
        );
    }

    #[test]
    fn a_process_with_two_sessions_counts_both_and_shows_the_playing_ones_level() {
        // Before the fix the second session of pid 10 was dropped: the row
        // read as silent at the silent session's level.
        let sessions = [
            raw(10, "spotify", false, 0.3),
            raw(10, "spotify", true, 0.8),
            raw(20, "aimp", false, 0.5),
        ];
        let got = rows((0.5, false), &sessions);
        assert_eq!(got[1].name, "Spotify", "active, so it leads");
        assert_eq!(got[1].level, 0.8);
        // Muted only when every session is.
        let mut sessions = sessions;
        sessions[0].muted = true;
        assert!(!rows((0.5, false), &sessions)[1].muted);
        sessions[1].muted = true;
        assert!(rows((0.5, false), &sessions)[1].muted);
        // No session plays: the first one's level.
        let quiet = [raw(10, "vlc", false, 0.2), raw(11, "vlc", false, 0.9)];
        assert_eq!(rows((0.5, false), &quiet)[1].level, 0.2);
    }

    fn row(target: Target, level: f32) -> MixerRow {
        MixerRow {
            name: match &target {
                Target::App(key) => display_name(key),
                _ => String::new(),
            },
            target,
            level,
            muted: false,
        }
    }

    fn app(key: &str) -> Target {
        Target::App(key.into())
    }

    #[test]
    fn rows_keep_the_order_shown_while_new_ones_append_and_gone_ones_drop() {
        let shown = [
            row(Target::Master, 0.5),
            row(app("chrome"), 0.5),
            row(app("spotify"), 0.5),
            row(app("teams"), 0.5),
            row(Target::System, 0.5),
        ];
        // The next reading sorts Spotify first (it started playing), lost
        // Teams and found Discord and Zoom.
        let fresh = vec![
            row(Target::Master, 0.6),
            row(app("spotify"), 0.9),
            row(app("zoom"), 0.4),
            row(app("chrome"), 0.1),
            row(app("discord"), 0.3),
            row(Target::System, 0.2),
        ];
        let got = keep_order(&shown, fresh);
        let targets: Vec<&Target> = got.iter().map(|r| &r.target).collect();
        assert_eq!(
            targets,
            [
                &Target::Master,
                &app("chrome"),
                &app("spotify"),
                &app("zoom"),
                &app("discord"),
                &Target::System,
            ]
        );
        // The values are the reading's.
        assert_eq!(got[0].level, 0.6);
        assert_eq!(got[1].level, 0.1);
        assert_eq!(got[2].level, 0.9);
        // Nothing shown yet: the reading's own order.
        let fresh = vec![row(Target::Master, 0.5), row(app("b"), 0.5), row(app("a"), 0.5)];
        assert_eq!(keep_order(&[], fresh.clone()), fresh);
    }

    #[test]
    fn a_reading_asked_before_the_last_change_is_stale() {
        // Scan 3 was asked, then a level went out as request 4.
        assert!(reading_is_stale(3, 4));
        // The scan asked after it (5) shows the change.
        assert!(!reading_is_stale(5, 4));
        // Nothing set yet.
        assert!(!reading_is_stale(1, 0));
    }

    #[test]
    fn program_names_are_read_once_per_process_while_it_is_seen() {
        let mut names = Names::default();
        let mut reads = 0;
        let mut read = |pid: u32| {
            reads += 1;
            if pid == 7 { String::new() } else { format!("app{pid}") }
        };
        assert_eq!(names.key(10, &mut read), "app10");
        assert_eq!(names.key(10, &mut read), "app10");
        // A name that could not be read is asked again.
        assert_eq!(names.key(7, &mut read), "");
        assert_eq!(names.key(7, &mut read), "");
        // A process the last scan did not see is forgotten (its id may be
        // reused by another program).
        names.keep_only(&[7]);
        assert_eq!(names.key(10, &mut read), "app10");
        assert_eq!(reads, 4);
    }

    #[test]
    fn the_mixer_wheel_adds_small_deltas_up_to_whole_notches() {
        let mut sum = 0;
        assert_eq!(wheel_notches(&mut sum, 120), 1);
        assert_eq!(wheel_notches(&mut sum, -240), -2);
        assert_eq!(sum, 0);
        // A touchpad: 30 at a time steps once every four messages.
        let steps: Vec<i32> = (0..8).map(|_| wheel_notches(&mut sum, 30)).collect();
        assert_eq!(steps, [0, 0, 0, 1, 0, 0, 0, 1]);
        // Back the other way cancels what was kept.
        let mut sum = 0;
        assert_eq!(wheel_notches(&mut sum, 90), 0);
        assert_eq!(wheel_notches(&mut sum, -90), 0);
        assert_eq!(sum, 0);
    }
}
