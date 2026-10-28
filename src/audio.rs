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
    AudioSessionStateActive, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender,
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
/// play. A session whose program is unknown is left out; so is a repeat of
/// the same process id.
pub fn rows(master: (f32, bool), sessions: &[RawSession]) -> Vec<MixerRow> {
    let mut out = vec![MixerRow {
        target: Target::Master,
        name: String::new(),
        level: clamp_level(master.0),
        muted: master.1,
    }];
    let mut seen: Vec<u32> = Vec::new();
    let mut apps: Vec<(bool, MixerRow)> = Vec::new();
    let mut system: Option<MixerRow> = None;
    for s in sessions {
        if seen.contains(&s.pid) && !s.system {
            continue;
        }
        seen.push(s.pid);
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
            // Another process of the same program: it is active if any is;
            // the row shows the first one's level.
            Some((active, row)) => {
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

/// A request to the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    Scan,
    Level(Target, f32),
    Mute(Target, bool),
}

/// The requests that matter out of a batch waiting in the queue, in the
/// order of their last mention: the last level and the last mute per target,
/// and one scan (a drag sends one per pointer move).
pub fn coalesce(batch: Vec<Req>) -> Vec<Req> {
    let same = |a: &Req, b: &Req| match (a, b) {
        (Req::Scan, Req::Scan) => true,
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
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut scan = false;
        for req in coalesce(batch) {
            match req {
                Req::Scan => scan = true,
                Req::Level(target, level) => set(&target, Some(clamp_level(level)), None),
                Req::Mute(target, muted) => set(&target, None, Some(muted)),
            }
        }
        if scan {
            let found = if com { read() } else { None };
            if tx.send(Event::Audio(found)).is_err() {
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
fn sessions(device: &IMMDevice) -> Vec<(IAudioSessionControl, RawSession)> {
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
            let (Ok(c2), Ok(volume)) = (
                control.cast::<IAudioSessionControl2>(),
                control.cast::<ISimpleAudioVolume>(),
            ) else {
                continue;
            };
            let pid = c2.GetProcessId().unwrap_or(0);
            let raw = RawSession {
                pid,
                key: process_key(pid),
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
fn read() -> Option<Vec<MixerRow>> {
    let device = default_device()?;
    let master = unsafe {
        let volume = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        (
            volume.GetMasterVolumeLevelScalar().unwrap_or(0.0),
            volume.GetMute().is_ok_and(|m| m.as_bool()),
        )
    };
    let raw: Vec<RawSession> = sessions(&device).into_iter().map(|(_, s)| s).collect();
    Some(rows(master, &raw))
}

/// Sets a level and/or a mute on `target`.
fn set(target: &Target, level: Option<f32>, muted: Option<bool>) {
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
        for (control, raw) in sessions(&device) {
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
            Req::Scan,
            Req::Level(spotify.clone(), 0.2),
            Req::Level(Target::Master, 0.3),
            Req::Mute(Target::Master, true),
            Req::Scan,
            Req::Mute(Target::Master, false),
        ];
        assert_eq!(
            coalesce(batch),
            vec![
                Req::Level(spotify, 0.2),
                Req::Level(Target::Master, 0.3),
                Req::Scan,
                Req::Mute(Target::Master, false),
            ]
        );
    }
}
