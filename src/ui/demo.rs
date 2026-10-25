//! `--demo` (debug builds only): cycles through every view, 2 s each, Pill
//! then Notch, with long text and fake calendar, notes and pomodoro data, so
//! the window and renderer can be checked by eye and by screen capture. Runs
//! until the process is killed.
//!
//! Environment knobs for screenshots:
//! - `DI_DEMO_STEP=n` holds step `n` (see `SCENES`; Notch adds `SCENES.len()`).
//! - `DI_DEMO_T=0.5` freezes the morph into that step at this progress.
//! - `DI_DEMO_SLIDE=0.4` draws the panel mid page-slide.

use std::time::{Duration, Instant};

use chrono::{Days, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, MA_NOACTIVATE, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
    TranslateMessage, WM_DESTROY, WM_MOUSEACTIVATE, WM_QUIT,
};

use super::geometry::{ANIM_MS, ShapeKind, ease_out_back, lerp_shape, shape_for};
use super::render::{Frame, Renderer};
use super::window::IslandWindow;
use crate::calendar::{CalEvent, CalStatus, CalendarSnapshot, EventTime};
use crate::config::Style;
use crate::model::{CalView, MediaInfo, Notice, NoticeKind, Page, View};
use crate::notes::Note;
use crate::pomodoro::{PomoSettings, Pomodoro};

const PHASE: Duration = Duration::from_millis(2000);

#[derive(Clone, Copy, PartialEq)]
enum Pomo {
    Idle,
    Running,
    Paused,
    Ready,
}

#[derive(Clone, Copy)]
struct Scene {
    name: &'static str,
    kind: ShapeKind,
    view: View0,
    pomo: Pomo,
    cal_view: CalView,
    status: CalStatus,
    day_page: usize,
    notes: usize,
    strip: bool,
}

/// `View` without its payloads, so scenes can be `const`.
#[derive(Clone, Copy, PartialEq)]
enum View0 {
    Idle,
    MediaCompact,
    PomodoroCompact,
    Panel(Page),
}

const BASE: Scene = Scene {
    name: "",
    kind: ShapeKind::Panel,
    view: View0::Idle,
    pomo: Pomo::Idle,
    cal_view: CalView::Month,
    status: CalStatus::Ok,
    day_page: 0,
    notes: 5,
    strip: false,
};

const fn panel(name: &'static str, page: Page) -> Scene {
    Scene {
        name,
        view: View0::Panel(page),
        ..BASE
    }
}

const SCENES: [Scene; 15] = [
    Scene {
        name: "idle",
        kind: ShapeKind::Idle,
        ..BASE
    },
    Scene {
        name: "pomo-running",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Running,
        ..BASE
    },
    Scene {
        name: "pomo-paused",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Paused,
        ..BASE
    },
    Scene {
        name: "pomo-ready",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Ready,
        ..BASE
    },
    Scene {
        name: "media-compact",
        kind: ShapeKind::MediaCompact,
        view: View0::MediaCompact,
        ..BASE
    },
    panel("cal-month", Page::Calendar),
    Scene {
        cal_view: CalView::Week,
        ..panel("cal-week", Page::Calendar)
    },
    Scene {
        cal_view: CalView::Day,
        ..panel("cal-day", Page::Calendar)
    },
    Scene {
        status: CalStatus::NeedsLogin,
        ..panel("cal-needslogin", Page::Calendar)
    },
    Scene {
        cal_view: CalView::Day,
        status: CalStatus::Offline,
        day_page: 1,
        ..panel("cal-day-offline-p1", Page::Calendar)
    },
    Scene {
        pomo: Pomo::Running,
        ..panel("pomodoro", Page::Pomodoro)
    },
    panel("notes-5", Page::Notes),
    Scene {
        notes: 30,
        ..panel("notes-30", Page::Notes)
    },
    panel("media", Page::Media),
    Scene {
        strip: true,
        ..panel("strip", Page::Calendar)
    },
];

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}

pub fn run() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let win = IslandWindow::create(Some(wnd_proc)).expect("create island window");
    let mut renderer = Renderer::new().expect("create renderer");
    let media = MediaInfo {
        title: "Sampai Jadi Debu (Live at Synchronize Festival 2024, Extended Version)".into(),
        artist: "Banda Neira feat. Hindia, Kunto Aji, Nadin Amizah & Friends".into(),
        playing: true,
        art: Some(test_art()),
    };
    let today = Local::now().date_naive();
    let calendar = demo_calendar(today);
    let notes5 = demo_notes(today);
    let notes30 = many_notes(today);
    let strip = Notice {
        key: "demo-wait".into(),
        source: "Claude".into(),
        title: "Claude butuh respon".into(),
        body: "Dynamic Island".into(),
        kind: NoticeKind::ClaudeWaiting {
            session_id: "demo".into(),
        },
    };
    let hold: Option<usize> = env("DI_DEMO_STEP");
    let frozen_t: Option<f32> = env("DI_DEMO_T");
    let slide: f32 = env("DI_DEMO_SLIDE").unwrap_or(0.0);
    let steps = 2 * SCENES.len();
    println!("demo: 2 s per view, Pill then Notch ({steps} steps); kill the process to stop");
    let start = Instant::now();
    let mut settled = usize::MAX;
    loop {
        if !pump_messages() {
            return;
        }
        let elapsed = start.elapsed();
        let (step, into) = match hold {
            Some(n) => (n % steps, elapsed.as_millis()),
            None => (
                (elapsed.as_millis() / PHASE.as_millis()) as usize % steps,
                elapsed.as_millis() % PHASE.as_millis(),
            ),
        };
        let t = frozen_t.unwrap_or((into as f32 / ANIM_MS as f32).min(1.0));
        let (style, scene) = scene(step);
        // The equalizer animates; everything else redraws only while morphing.
        let animated = scene.view == View0::MediaCompact;
        if step != settled || animated {
            if t >= 1.0 {
                if settled != step {
                    println!("demo: step {step} {:?} {}", style, scene.name);
                }
                settled = step;
            }
            let (prev_style, prev) = self::scene((step + steps - 1) % steps);
            let from = shape_for(prev_style, prev.kind);
            let shape = lerp_shape(from, shape_for(style, scene.kind), ease_out_back(t));
            let now = Instant::now();
            let pomodoro = demo_pomodoro(scene.pomo, now);
            let view = match scene.view {
                View0::Idle => View::Idle,
                View0::MediaCompact => View::MediaCompact,
                View0::PomodoroCompact => View::PomodoroCompact,
                View0::Panel(page) => View::Panel(page),
            };
            let page = match scene.view {
                View0::Panel(page) => page,
                _ => Page::Calendar,
            };
            let snapshot = CalendarSnapshot {
                status: scene.status,
                ..calendar.clone()
            };
            let notes: &[Note] = if scene.notes == 30 { &notes30 } else { &notes5 };
            let frame = Frame {
                view: &view,
                media: Some(&media),
                pomodoro: &pomodoro,
                now,
                page,
                strip: scene.strip.then_some(&strip),
                cal_view: scene.cal_view,
                cal_anchor: today,
                cal_selected: today,
                calendar: &snapshot,
                notes,
                notes_page: 0,
                day_page: scene.day_page,
                page_slide: slide,
            };
            if let Err(error) = renderer.draw(&win, style, shape, &frame, 1.0) {
                eprintln!("demo: draw failed: {error}");
            }
        }
        std::thread::sleep(Duration::from_millis(if animated { 33 } else { 16 }));
    }
}

fn scene(step: usize) -> (Style, Scene) {
    let style = if step < SCENES.len() {
        Style::Pill
    } else {
        Style::Notch
    };
    (style, SCENES[step % SCENES.len()])
}

/// Focus session 2 of 4 with 18:42 left (running or paused), or a short break
/// ready to start.
fn demo_pomodoro(which: Pomo, now: Instant) -> Pomodoro {
    let mut p = Pomodoro::new(PomoSettings::default());
    if which == Pomo::Idle {
        return p;
    }
    if which == Pomo::Ready {
        p.skip(now);
        return p;
    }
    // Focus 1 → short break → focus 2.
    p.skip(now);
    p.skip(now);
    let started = now.checked_sub(Duration::from_secs(378)).unwrap_or(now);
    p.start_pause(started);
    if which == Pomo::Paused {
        p.start_pause(now);
    }
    p
}

fn local(d: NaiveDate, h: u32, m: u32) -> EventTime {
    let t = d.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time"));
    EventTime::At(
        Local
            .from_local_datetime(&t)
            .earliest()
            .expect("local time"),
    )
}

/// Six events on today: all-day, multi-day all-day, three timed (one past
/// midnight), and one with a very long title.
fn demo_calendar(today: NaiveDate) -> CalendarSnapshot {
    let day = |n: i64| {
        if n >= 0 {
            today + Days::new(n as u64)
        } else {
            today - Days::new(n.unsigned_abs())
        }
    };
    let ev = |id: &str, title: &str, start, end, color| CalEvent {
        id: id.into(),
        calendar_id: "primary".into(),
        title: title.into(),
        start,
        end,
        description: String::new(),
        color,
        editable: true,
    };
    let events = vec![
        ev(
            "bday",
            "Ulang tahun Rani",
            EventTime::AllDay(today),
            EventTime::AllDay(day(1)),
            Some(0xF5A524),
        ),
        ev(
            "expo",
            "Pameran arsitektur Jakarta",
            EventTime::AllDay(day(-1)),
            EventTime::AllDay(day(3)),
            Some(0x9B59B6),
        ),
        ev(
            "review",
            "Review desain klien",
            local(today, 10, 0),
            local(today, 11, 30),
            None,
        ),
        ev(
            "rapat",
            "Rapat koordinasi kontraktor, struktur, MEP dan pemilik proyek gedung kantor",
            local(today, 14, 0),
            local(today, 15, 0),
            None,
        ),
        ev(
            "dinner",
            "Makan malam keluarga",
            local(today, 19, 0),
            local(today, 21, 0),
            Some(0x2ECC71),
        ),
        ev(
            "render",
            "Render final malam",
            local(today, 22, 0),
            local(day(1), 2, 0),
            Some(0xEF5350),
        ),
    ];
    CalendarSnapshot {
        status: CalStatus::Ok,
        events,
        from: day(-30),
        to: day(30),
        fetched_at: Some(Local::now()),
    }
}

fn stamp(t: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map_or(0, |t| t.timestamp())
}

fn demo_notes(today: NaiveDate) -> Vec<Note> {
    let at = |d: NaiveDate, h, m| stamp(d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()));
    let note = |id: &str, text: &str, updated| Note {
        id: id.into(),
        text: text.into(),
        updated,
    };
    vec![
        note(
            "n1",
            "Revisi denah lantai 2 — geser tangga ke timur",
            at(today, 9, 12),
        ),
        note(
            "n2",
            "Beli tinta printer A3",
            at(today - Days::new(1), 16, 0),
        ),
        note(
            "n3",
            "\n  Ide: fasad pakai roster bata ekspos\ndetail menyusul",
            at(today - Days::new(2), 8, 0),
        ),
        note(
            "n4",
            "Catatan rapat: klien minta jendela lebih besar di semua kamar tidur dan ruang keluarga menghadap taman belakang",
            at(today - Days::new(9), 11, 0),
        ),
        note(
            "n5",
            "Perpanjang lisensi software",
            at(today - Days::new(400), 11, 0),
        ),
    ]
}

fn many_notes(today: NaiveDate) -> Vec<Note> {
    let base = stamp(today.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()));
    (0..30)
        .map(|i| Note {
            id: format!("m{i}"),
            text: format!("Catatan nomor {}", i + 1),
            updated: base - i64::from(i) * 7 * 3600,
        })
        .collect()
}

/// A 64×40 orange-to-purple gradient BMP, non-square to exercise the crop.
fn test_art() -> Vec<u8> {
    const W: u32 = 64;
    const H: u32 = 40;
    let row = W * 3; // 192 bytes: already 4-byte aligned
    let pixels = row * H;
    let mut bmp = Vec::with_capacity((54 + pixels) as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + pixels).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(W as i32).to_le_bytes());
    bmp.extend_from_slice(&(H as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0u8; 24]);
    let (from, to) = ([0xD9, 0x86, 0x4A], [0x7A, 0x3B, 0x6B]);
    for y in 0..H {
        for x in 0..W {
            let t = (x + y) as f32 / (W + H - 2) as f32;
            let mix = |i: usize| (from[i] as f32 + (to[i] as f32 - from[i] as f32) * t) as u8;
            bmp.extend_from_slice(&[mix(2), mix(1), mix(0)]);
        }
    }
    bmp
}

/// Dispatches pending messages; false once WM_QUIT arrives.
fn pump_messages() -> bool {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
