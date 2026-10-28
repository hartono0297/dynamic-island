//! The System page's readings (CPU, RAM, fixed drives, the busiest
//! processes, uptime) and the background disk / RAM warnings.
//!
//! Readings run on a parked worker thread (`Sampler`), never on the UI
//! thread: `GetDiskFreeSpaceExW` can block on a spun-down disk and a
//! process walk takes milliseconds. Each answer comes back as
//! `Event::SystemSample`. The arithmetic (CPU deltas, per-app totals, the
//! history, the warnings) is pure and tested here; the Win32 calls are thin.
//! Nothing in this module deletes or changes anything on the machine.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::SystemInformation::{
    GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetProcessTimes, GetSystemTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::HSTRING;

use crate::i18n::{self, Key, Lang, text};
use crate::model::{Event, Notice, NoticeKind};

pub const GIB: u64 = 1024 * 1024 * 1024;
/// The CPU sparkline: 30 points, one per 2 s page sample, so 60 s.
pub const HISTORY_LEN: usize = 30;
/// A history whose last point is older than this starts over, so a page
/// opened again does not join its line to stale data.
pub const HISTORY_STALE: Duration = Duration::from_secs(10);
/// CPU and process times older than this are no baseline: the worker
/// takes a fresh one and samples again `BASELINE_WAIT` later.
pub const BASELINE_MAX_AGE: Duration = Duration::from_secs(10);
const BASELINE_WAIT: Duration = Duration::from_millis(300);
/// RAM must stay above its limit this long before the notice.
pub const RAM_CONFIRM: Duration = Duration::from_secs(60);
/// A drive re-arms once its free space is this far back above the limit,
/// so free space wobbling around it does not raise the notice again.
pub const DISK_HYSTERESIS: u64 = GIB;
/// The busiest processes the page lists.
pub const TOP_PROCESSES: usize = 3;
/// `GetDriveTypeW`'s value for a fixed disk (the constant lives in a
/// feature this crate does not otherwise need).
const DRIVE_FIXED: u32 = 3;
/// The System Idle Process and System: no times worth showing.
const SKIPPED_PIDS: [u32; 2] = [0, 4];

/// `GetSystemTimes` in 100 ns units. `kernel` includes `idle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

impl CpuTimes {
    /// All processor time, busy and idle, summed over the cores.
    pub fn total(&self) -> u64 {
        self.kernel.saturating_add(self.user)
    }
}

/// How busy the CPU was between two readings, 0..=100. A zero or negative
/// span (the same reading twice, a counter that went back) is 0.
pub fn cpu_percent(prev: CpuTimes, cur: CpuTimes) -> f32 {
    let delta = |a: u64, b: u64| b.checked_sub(a);
    let (Some(idle), Some(kernel), Some(user)) = (
        delta(prev.idle, cur.idle),
        delta(prev.kernel, cur.kernel),
        delta(prev.user, cur.user),
    ) else {
        return 0.0;
    };
    let total = kernel.saturating_add(user);
    if total == 0 {
        return 0.0;
    }
    let busy = total.saturating_sub(idle);
    ((busy as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// `part` of `whole` in whole percent (rounded); 0 when `whole` is.
pub fn percent(part: u64, whole: u64) -> u8 {
    if whole == 0 {
        return 0;
    }
    ((part as f64 / whole as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

/// One process's times as the walk read them.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcTimes {
    pub pid: u32,
    /// The exe name, "chrome.exe".
    pub name: String,
    /// Kernel plus user time, 100 ns units.
    pub time: u64,
    /// Working set, bytes.
    pub mem: u64,
}

/// One app on the page: all its processes together.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcUsage {
    /// "Chrome".
    pub name: String,
    /// Share of the whole machine, 0..=100.
    pub cpu: f32,
    pub mem: u64,
}

/// "chrome.exe" → "Chrome": the ".exe" dropped (any case), the first letter
/// in capitals, the rest as it is.
pub fn display_name(exe: &str) -> String {
    let stem = exe
        .len()
        .checked_sub(4)
        .and_then(|i| exe.get(i..).map(|ext| (i, ext)))
        .filter(|(_, ext)| ext.eq_ignore_ascii_case(".exe"))
        .map_or(exe, |(i, _)| &exe[..i]);
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Each app's CPU share since `prev` and its memory now, adding up every
/// process of the same exe (case-insensitive: Chrome's many count as one).
/// A process's share is its time delta over `system_delta` (all cores'
/// time, `CpuTimes::total`). A process new since `prev`, or a pid reused by
/// another exe, has no delta yet and counts 0 % (its memory still counts).
pub fn process_usage(prev: &[ProcTimes], cur: &[ProcTimes], system_delta: u64) -> Vec<ProcUsage> {
    let before: HashMap<u32, &ProcTimes> = prev.iter().map(|p| (p.pid, p)).collect();
    // Keyed by the lowercased exe; the first spelling seen names it.
    let mut apps: HashMap<String, (String, u64, u64)> = HashMap::new();
    for p in cur {
        let delta = before
            .get(&p.pid)
            .filter(|b| b.name.eq_ignore_ascii_case(&p.name))
            .map_or(0, |b| p.time.saturating_sub(b.time));
        let app = apps
            .entry(p.name.to_lowercase())
            .or_insert_with(|| (display_name(&p.name), 0, 0));
        app.1 = app.1.saturating_add(delta);
        app.2 = app.2.saturating_add(p.mem);
    }
    apps.into_values()
        .map(|(name, delta, mem)| {
            let cpu = if system_delta == 0 {
                0.0
            } else {
                ((delta as f64 / system_delta as f64) * 100.0).clamp(0.0, 100.0) as f32
            };
            ProcUsage { name, cpu, mem }
        })
        .collect()
}

/// The `n` busiest: by CPU, then memory, then name.
pub fn top_processes(mut apps: Vec<ProcUsage>, n: usize) -> Vec<ProcUsage> {
    apps.sort_by(|a, b| {
        b.cpu
            .total_cmp(&a.cpu)
            .then(b.mem.cmp(&a.mem))
            .then_with(|| a.name.cmp(&b.name))
    });
    apps.truncate(n);
    apps
}

/// The CPU line under the ring: the last `HISTORY_LEN` page samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuHistory {
    points: Vec<f32>,
    last: Option<Instant>,
}

impl CpuHistory {
    /// Adds a sample taken at `now`; after a gap longer than
    /// `HISTORY_STALE` (the page was closed) the line starts over.
    pub fn push(&mut self, cpu: f32, now: Instant) {
        if self
            .last
            .is_some_and(|t| now.saturating_duration_since(t) > HISTORY_STALE)
        {
            self.points.clear();
        }
        self.points.push(cpu);
        if self.points.len() > HISTORY_LEN {
            let extra = self.points.len() - HISTORY_LEN;
            self.points.drain(..extra);
        }
        self.last = Some(now);
    }

    pub fn points(&self) -> &[f32] {
        &self.points
    }
}

/// A fixed drive and its space, bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub letter: char,
    pub free: u64,
    pub total: u64,
}

impl Drive {
    /// The meter's fill: the used part.
    pub fn used_percent(&self) -> u8 {
        percent(self.total.saturating_sub(self.free), self.total)
    }
}

/// What a reading was for: the open page (everything), or the background
/// warning check (memory and drives only, no process walk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    Page,
    Warn,
}

/// One reading of the machine.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemSample {
    pub kind: SampleKind,
    /// 0..=100; 0 in a `Warn` reading, which measures no CPU.
    pub cpu: f32,
    /// Logical processors.
    pub cores: u32,
    pub ram_used: u64,
    /// 0 when memory could not be read.
    pub ram_total: u64,
    /// Fixed drives, A to Z.
    pub drives: Vec<Drive>,
    /// The `TOP_PROCESSES` busiest apps; empty in a `Warn` reading.
    pub procs: Vec<ProcUsage>,
    pub uptime: Duration,
}

impl SystemSample {
    /// `None` when memory could not be read.
    pub fn ram_percent(&self) -> Option<u8> {
        (self.ram_total > 0).then(|| percent(self.ram_used, self.ram_total))
    }
}

/// Free space under the limit.
pub fn disk_low(free: u64, limit_gb: u32) -> bool {
    free < u64::from(limit_gb) * GIB
}

/// Used memory over the limit.
pub fn ram_high(percent: u8, limit: u8) -> bool {
    percent > limit
}

/// A warning to raise now.
#[derive(Clone, Debug, PartialEq)]
pub enum Alert {
    Disk { letter: char, free: u64 },
    Ram { percent: u8 },
}

/// What one reading decided.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WarnCheck {
    pub alerts: Vec<Alert>,
    /// RAM is above its limit but not yet for `RAM_CONFIRM`: read again
    /// this much later to confirm.
    pub recheck: Option<Duration>,
}

/// Decides the disk and RAM notices, once per crossing, like the battery's
/// monitor. Disk: per drive, as soon as free space is under the limit;
/// re-armed once it is `DISK_HYSTERESIS` back above it. RAM: only after
/// staying above the limit for `RAM_CONFIRM` (a reading at or under it
/// starts the minute over); re-armed at or under the limit.
#[derive(Clone, Debug, PartialEq)]
pub struct WarnMonitor {
    disk_gb: u32,
    ram_percent: u8,
    disk_alerted: Vec<char>,
    ram_above_since: Option<Instant>,
    ram_alerted: bool,
}

impl WarnMonitor {
    pub fn new(disk_gb: u32, ram_percent: u8) -> Self {
        Self {
            disk_gb,
            ram_percent,
            disk_alerted: Vec::new(),
            ram_above_since: None,
            ram_alerted: false,
        }
    }

    /// New limits from the settings window; alerts already raised stay
    /// raised until their crossing ends.
    pub fn set_limits(&mut self, disk_gb: u32, ram_percent: u8) {
        self.disk_gb = disk_gb;
        self.ram_percent = ram_percent;
    }

    /// One reading: `drives` as read, `ram` the used percent (`None` when
    /// unknown, which changes nothing about RAM).
    pub fn update(&mut self, drives: &[Drive], ram: Option<u8>, now: Instant) -> WarnCheck {
        let mut check = WarnCheck::default();
        let limit = u64::from(self.disk_gb) * GIB;
        for d in drives {
            let alerted = self.disk_alerted.contains(&d.letter);
            if d.free < limit {
                if !alerted {
                    self.disk_alerted.push(d.letter);
                    check.alerts.push(Alert::Disk {
                        letter: d.letter,
                        free: d.free,
                    });
                }
            } else if alerted && d.free > limit.saturating_add(DISK_HYSTERESIS) {
                self.disk_alerted.retain(|l| *l != d.letter);
            }
        }
        if let Some(pct) = ram {
            if ram_high(pct, self.ram_percent) {
                let since = *self.ram_above_since.get_or_insert(now);
                let above = now.saturating_duration_since(since);
                if !self.ram_alerted {
                    if above >= RAM_CONFIRM {
                        self.ram_alerted = true;
                        check.alerts.push(Alert::Ram { percent: pct });
                    } else {
                        check.recheck = Some(RAM_CONFIRM - above);
                    }
                }
            } else {
                self.ram_above_since = None;
                self.ram_alerted = false;
            }
        }
        check
    }
}

/// The notice for `alert`, from the island (Bata looks low).
pub fn alert_notice(lang: Lang, alert: &Alert) -> Notice {
    let (key, title, body) = match alert {
        Alert::Disk { letter, free } => (
            format!("system-disk-{letter}"),
            i18n::disk_low_title(lang, *letter, *free),
            text(lang, Key::SystemDiskBody),
        ),
        Alert::Ram { percent } => (
            "system-ram".to_string(),
            i18n::ram_high_title(lang, *percent),
            text(lang, Key::SystemRamBody),
        ),
    };
    Notice {
        key,
        source: text(lang, Key::System).to_string(),
        title,
        body: body.to_string(),
        kind: NoticeKind::SystemLow,
    }
}

/// Whether the worker must take a fresh baseline before it can give a CPU
/// figure: none yet, or one older than `BASELINE_MAX_AGE`.
pub fn needs_baseline(taken: Option<Instant>, now: Instant) -> bool {
    taken.is_none_or(|t| now.saturating_duration_since(t) > BASELINE_MAX_AGE)
}

// ---- Win32 readers ----

fn filetime(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

pub fn read_cpu_times() -> Option<CpuTimes> {
    let (mut idle, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    Some(CpuTimes {
        idle: filetime(idle),
        kernel: filetime(kernel),
        user: filetime(user),
    })
}

/// Used and total physical memory, bytes.
pub fn read_memory() -> Option<(u64, u64)> {
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some((
        m.ullTotalPhys.saturating_sub(m.ullAvailPhys),
        m.ullTotalPhys,
    ))
}

/// Fixed drives only: removable, network and CD drives are skipped (and
/// never woken).
pub fn read_drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    (0u8..26)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let letter = char::from(b'A' + i);
            let root = HSTRING::from(format!("{letter}:\\"));
            if unsafe { GetDriveTypeW(&root) } != DRIVE_FIXED {
                return None;
            }
            let (mut free, mut total) = (0u64, 0u64);
            unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) }.ok()?;
            (total > 0).then_some(Drive {
                letter,
                free,
                total,
            })
        })
        .collect()
}

pub fn read_uptime() -> Duration {
    Duration::from_millis(unsafe { GetTickCount64() })
}

pub fn read_cores() -> u32 {
    unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1)
}

/// Every process that opens with limited query rights, with its times and
/// working set. Ones that refuse (protected, gone meanwhile) are skipped;
/// every handle is closed.
pub fn read_processes() -> Vec<ProcTimes> {
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        if !SKIPPED_PIDS.contains(&entry.th32ProcessID)
            && let Some(p) = read_process(&entry)
        {
            out.push(p);
        }
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    out
}

fn read_process(entry: &PROCESSENTRY32W) -> Option<ProcTimes> {
    let len = entry
        .szExeFile
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(entry.szExeFile.len());
    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
    let pid = entry.th32ProcessID;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    let times =
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }.ok();
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    let mem_ok = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) }.as_bool();
    unsafe {
        let _ = CloseHandle(handle);
    }
    times?;
    Some(ProcTimes {
        pid,
        name,
        time: filetime(kernel).saturating_add(filetime(user)),
        mem: if mem_ok {
            counters.WorkingSetSize as u64
        } else {
            0
        },
    })
}

// ---- The worker ----

/// CPU and process times kept between page samples.
struct Baseline {
    cpu: CpuTimes,
    procs: Vec<ProcTimes>,
    at: Instant,
}

fn take_baseline() -> Baseline {
    Baseline {
        cpu: read_cpu_times().unwrap_or_default(),
        procs: read_processes(),
        at: Instant::now(),
    }
}

/// The reading thread. It waits on its channel and answers each request
/// in order; dropping the `Sampler` ends it.
pub struct Sampler {
    requests: Sender<SampleKind>,
}

impl Sampler {
    /// `None` when the thread could not start.
    pub fn spawn(tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> Option<Self> {
        let (requests, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("system-sampler".into())
            .spawn(move || serve(&rx, &tx, wake));
        match spawned {
            Ok(_) => Some(Self { requests }),
            Err(error) => {
                eprintln!("vale: no system sampler: {error}");
                None
            }
        }
    }

    /// False when the thread is gone.
    pub fn request(&self, kind: SampleKind) -> bool {
        self.requests.send(kind).is_ok()
    }
}

fn serve(rx: &Receiver<SampleKind>, tx: &Sender<Event>, wake: impl Fn()) {
    let mut baseline: Option<Baseline> = None;
    while let Ok(kind) = rx.recv() {
        let sample = match kind {
            SampleKind::Page => sample_page(&mut baseline),
            SampleKind::Warn => sample_warn(),
        };
        if tx.send(Event::SystemSample(sample)).is_err() {
            return;
        }
        wake();
    }
}

/// Everything, with the CPU and process figures since the last page
/// sample. Without a recent one it takes a baseline first and waits
/// `BASELINE_WAIT`, so the page shows real numbers at once.
fn sample_page(baseline: &mut Option<Baseline>) -> SystemSample {
    if needs_baseline(baseline.as_ref().map(|b| b.at), Instant::now()) {
        *baseline = Some(take_baseline());
        std::thread::sleep(BASELINE_WAIT);
    }
    let cur = take_baseline();
    let prev = baseline.replace(cur).expect("baseline taken above");
    let cur = baseline.as_ref().expect("just set");
    let delta = cur.cpu.total().saturating_sub(prev.cpu.total());
    let procs = top_processes(process_usage(&prev.procs, &cur.procs, delta), TOP_PROCESSES);
    let (ram_used, ram_total) = read_memory().unwrap_or_default();
    SystemSample {
        kind: SampleKind::Page,
        cpu: cpu_percent(prev.cpu, cur.cpu),
        cores: read_cores(),
        ram_used,
        ram_total,
        drives: read_drives(),
        procs,
        uptime: read_uptime(),
    }
}

/// Memory and drives only, for the warnings.
fn sample_warn() -> SystemSample {
    let (ram_used, ram_total) = read_memory().unwrap_or_default();
    SystemSample {
        kind: SampleKind::Warn,
        cpu: 0.0,
        cores: 0,
        ram_used,
        ram_total,
        drives: read_drives(),
        procs: Vec::new(),
        uptime: read_uptime(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(idle: u64, kernel: u64, user: u64) -> CpuTimes {
        CpuTimes { idle, kernel, user }
    }

    #[test]
    fn cpu_percent_from_deltas() {
        // 1000 units passed (kernel 600, of which 300 idle, user 400):
        // busy 700 of 1000.
        let a = times(1_000, 2_000, 3_000);
        let b = times(1_300, 2_600, 3_400);
        assert!((cpu_percent(a, b) - 70.0).abs() < 0.01);
        // All idle, all busy.
        assert_eq!(cpu_percent(a, times(1_500, 2_500, 3_000)), 0.0);
        assert_eq!(cpu_percent(a, times(1_000, 2_500, 3_500)), 100.0);
        // No time passed, or a counter went back: 0, never NaN or negative.
        assert_eq!(cpu_percent(a, a), 0.0);
        assert_eq!(cpu_percent(b, a), 0.0);
        // Idle larger than the span (a bad reading) is clamped.
        assert_eq!(cpu_percent(a, times(9_000, 2_100, 3_000)), 0.0);
        assert_eq!(b.total(), 6_000);
    }

    #[test]
    fn percents_round_and_guard_zero() {
        assert_eq!(percent(14, 32), 44);
        assert_eq!(percent(1, 0), 0);
        assert_eq!(percent(5, 4), 100);
        let d = Drive {
            letter: 'C',
            free: 47,
            total: 338,
        };
        assert_eq!(d.used_percent(), 86);
        let s = SystemSample {
            kind: SampleKind::Warn,
            cpu: 0.0,
            cores: 0,
            ram_used: 142,
            ram_total: 318,
            drives: vec![],
            procs: vec![],
            uptime: Duration::ZERO,
        };
        assert_eq!(s.ram_percent(), Some(45));
        assert_eq!(
            SystemSample {
                ram_total: 0,
                ..s.clone()
            }
            .ram_percent(),
            None
        );
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name("chrome.exe"), "Chrome");
        assert_eq!(display_name("Code.EXE"), "Code");
        assert_eq!(display_name("acad.exe"), "Acad");
        assert_eq!(display_name("explorer"), "Explorer");
        assert_eq!(display_name("msedgeWebView2.exe"), "MsedgeWebView2");
        assert_eq!(display_name(".exe"), "");
        assert_eq!(display_name("ä.exe"), "Ä");
        assert_eq!(display_name("é"), "É");
    }

    fn proc(pid: u32, name: &str, time: u64, mem: u64) -> ProcTimes {
        ProcTimes {
            pid,
            name: name.into(),
            time,
            mem,
        }
    }

    #[test]
    fn processes_add_up_per_exe_and_sort() {
        let prev = [
            proc(10, "chrome.exe", 100, 0),
            proc(11, "chrome.exe", 200, 0),
            proc(20, "Code.exe", 50, 0),
            proc(30, "acad.exe", 0, 0),
            // Reused below by another exe: no delta.
            proc(40, "old.exe", 0, 0),
        ];
        let cur = [
            proc(10, "chrome.exe", 160, 1_000),
            proc(11, "CHROME.EXE", 260, 2_000),
            proc(12, "chrome.exe", 999, 500),
            proc(20, "Code.exe", 120, 1_400),
            proc(30, "acad.exe", 50, 2_200),
            proc(40, "new.exe", 900, 10),
        ];
        let apps = process_usage(&prev, &cur, 1_000);
        let chrome = apps
            .iter()
            .find(|a| a.name == "Chrome")
            .expect("one Chrome");
        assert_eq!(apps.iter().filter(|a| a.name == "Chrome").count(), 1);
        // 60 + 60 (the new process has no delta yet) of 1000.
        assert!((chrome.cpu - 12.0).abs() < 0.01);
        assert_eq!(chrome.mem, 3_500);
        let new = apps.iter().find(|a| a.name == "New").unwrap();
        assert_eq!(new.cpu, 0.0, "pid reused by another exe");
        let top = top_processes(apps, 3);
        let names: Vec<&str> = top.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Chrome", "Code", "Acad"]);
        assert!((top[1].cpu - 7.0).abs() < 0.01);
        assert!((top[2].cpu - 5.0).abs() < 0.01);
        // No system time: everything 0 %, ordered by memory then name.
        let idle = top_processes(process_usage(&prev, &cur, 0), 10);
        assert!(idle.iter().all(|a| a.cpu == 0.0));
        let names: Vec<&str> = idle.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Chrome", "Acad", "Code", "New"]);
        // Ties on CPU and memory: by name.
        let tie = |n: &str| ProcUsage {
            name: n.into(),
            cpu: 1.0,
            mem: 1,
        };
        let sorted = top_processes(vec![tie("b"), tie("a")], 3);
        assert_eq!(sorted[0].name, "a");
    }

    #[test]
    fn history_keeps_thirty_points_and_restarts_after_a_gap() {
        let t0 = Instant::now();
        let mut h = CpuHistory::default();
        for i in 0..40 {
            h.push(i as f32, t0 + Duration::from_secs(2 * i));
        }
        assert_eq!(h.points().len(), HISTORY_LEN);
        assert_eq!(h.points()[0], 10.0);
        assert_eq!(*h.points().last().unwrap(), 39.0);
        // Within 10 s it continues; after a longer gap it starts over.
        let last = t0 + Duration::from_secs(78);
        h.push(1.0, last + Duration::from_secs(10));
        assert_eq!(h.points().len(), HISTORY_LEN);
        h.push(2.0, last + Duration::from_secs(21));
        assert_eq!(h.points(), [2.0]);
    }

    #[test]
    fn baseline_only_when_missing_or_stale() {
        let t0 = Instant::now();
        assert!(needs_baseline(None, t0));
        assert!(!needs_baseline(Some(t0), t0 + Duration::from_secs(2)));
        assert!(!needs_baseline(Some(t0), t0 + BASELINE_MAX_AGE));
        assert!(needs_baseline(
            Some(t0),
            t0 + BASELINE_MAX_AGE + Duration::from_millis(1)
        ));
    }

    fn drive(letter: char, free_gb: f64) -> Drive {
        Drive {
            letter,
            free: (free_gb * GIB as f64) as u64,
            total: 500 * GIB,
        }
    }

    #[test]
    fn disk_alerts_once_per_drive_and_rearms_above_the_margin() {
        let t0 = Instant::now();
        let mut m = WarnMonitor::new(10, 90);
        assert_eq!(
            m.update(&[drive('C', 12.0)], None, t0),
            WarnCheck::default()
        );
        let low = m.update(&[drive('C', 9.6), drive('D', 33.0)], None, t0);
        assert_eq!(
            low.alerts,
            [Alert::Disk {
                letter: 'C',
                free: (9.6 * GIB as f64) as u64
            }]
        );
        // Still low: once per crossing.
        assert!(m.update(&[drive('C', 8.0)], None, t0).alerts.is_empty());
        // Back above the limit but within the margin: not re-armed.
        assert!(m.update(&[drive('C', 10.5)], None, t0).alerts.is_empty());
        assert_eq!(m.update(&[drive('C', 9.0)], None, t0).alerts.len(), 0);
        // Well above: re-armed; the next drop alerts again.
        m.update(&[drive('C', 11.5)], None, t0);
        assert_eq!(m.update(&[drive('C', 9.0)], None, t0).alerts.len(), 1);
        // Each drive on its own.
        let both = m.update(&[drive('C', 9.0), drive('D', 1.0)], None, t0);
        assert_eq!(
            both.alerts,
            [Alert::Disk {
                letter: 'D',
                free: GIB
            }]
        );
        assert!(disk_low(9 * GIB, 10) && !disk_low(10 * GIB, 10));
    }

    #[test]
    fn ram_alerts_only_after_a_minute_above() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut m = WarnMonitor::new(10, 90);
        assert_eq!(
            m.update(&[], Some(90), s(0)),
            WarnCheck::default(),
            "at the limit"
        );
        // Above: no alert yet, a recheck when the minute is up.
        let first = m.update(&[], Some(93), s(0));
        assert!(first.alerts.is_empty());
        assert_eq!(first.recheck, Some(RAM_CONFIRM));
        let mid = m.update(&[], Some(95), s(40));
        assert!(mid.alerts.is_empty());
        assert_eq!(mid.recheck, Some(Duration::from_secs(20)));
        // A dip resets the minute.
        assert_eq!(m.update(&[], Some(80), s(50)), WarnCheck::default());
        let again = m.update(&[], Some(93), s(55));
        assert_eq!(again.recheck, Some(RAM_CONFIRM));
        assert!(m.update(&[], Some(93), s(100)).alerts.is_empty());
        // A full minute above: once.
        let alert = m.update(&[], Some(94), s(115));
        assert_eq!(alert.alerts, [Alert::Ram { percent: 94 }]);
        assert_eq!(alert.recheck, None);
        assert_eq!(m.update(&[], Some(97), s(400)), WarnCheck::default());
        // Unknown memory changes nothing.
        assert_eq!(m.update(&[], None, s(401)), WarnCheck::default());
        // Down to the limit re-arms; the next crossing needs its minute.
        m.update(&[], Some(90), s(402));
        assert!(m.update(&[], Some(93), s(403)).alerts.is_empty());
        assert_eq!(m.update(&[], Some(93), s(463)).alerts.len(), 1);
        assert!(ram_high(91, 90) && !ram_high(90, 90));
    }

    #[test]
    fn limit_changes_keep_raised_alerts() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut m = WarnMonitor::new(10, 90);
        assert_eq!(m.update(&[drive('C', 9.0)], None, s(0)).alerts.len(), 1);
        m.update(&[], Some(95), s(0));
        assert_eq!(m.update(&[], Some(95), s(60)).alerts.len(), 1);
        // A higher disk limit: C is still the same crossing, no new notice.
        m.set_limits(20, 90);
        assert!(m.update(&[drive('C', 15.0)], None, s(61)).alerts.is_empty());
        // A lower RAM limit while it is raised: nothing new.
        m.set_limits(20, 80);
        assert!(m.update(&[], Some(95), s(62)).alerts.is_empty());
        // A lower disk limit: a drive now above it re-arms past the margin.
        m.set_limits(5, 80);
        assert!(m.update(&[drive('C', 15.0)], None, s(63)).alerts.is_empty());
        assert_eq!(m.update(&[drive('C', 4.0)], None, s(64)).alerts.len(), 1);
        // A higher RAM limit above the reading re-arms it.
        m.set_limits(5, 98);
        m.update(&[], Some(95), s(65));
        m.set_limits(5, 90);
        assert!(m.update(&[], Some(95), s(66)).alerts.is_empty());
        assert_eq!(m.update(&[], Some(95), s(126)).alerts.len(), 1);
    }

    #[test]
    fn notices_name_the_drive_or_the_memory() {
        let disk = alert_notice(
            Lang::Id,
            &Alert::Disk {
                letter: 'C',
                free: (9.6 * GIB as f64) as u64,
            },
        );
        assert_eq!(disk.key, "system-disk-C");
        assert_eq!(disk.source, "Sistem");
        assert_eq!(disk.title, "Disk C: tinggal 9,6 GB");
        assert_eq!(disk.body, "Buka halaman Sistem untuk detailnya");
        assert_eq!(disk.kind, NoticeKind::SystemLow);
        let ram = alert_notice(Lang::En, &Alert::Ram { percent: 93 });
        assert_eq!(ram.key, "system-ram");
        assert_eq!(ram.source, "System");
        assert_eq!(ram.title, "RAM 93% used");
        assert_eq!(ram.body, "Above the limit for over a minute");
    }

    #[test]
    fn readers_run() {
        // Smoke test on the build machine: real figures, sane ranges.
        let a = read_cpu_times().expect("system times");
        assert!(a.kernel >= a.idle);
        let (used, total) = read_memory().expect("memory");
        assert!(total > 0 && used <= total);
        assert!(read_cores() >= 1);
        assert!(read_uptime() > Duration::ZERO);
        for d in read_drives() {
            assert!(d.free <= d.total, "{d:?}");
        }
        let procs = read_processes();
        assert!(
            procs
                .iter()
                .all(|p| !SKIPPED_PIDS.contains(&p.pid) && !p.name.is_empty())
        );
        assert!(procs.iter().any(|p| p.pid == std::process::id()));
    }
}
