//! `--demo` (debug builds only): cycles Hidden → compact → expanded → notice
//! every 2 s, Pill then Notch, with long text, so the window and renderer can
//! be checked by eye and by screen capture. Runs until the process is killed.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, MA_NOACTIVATE, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
    TranslateMessage, WM_DESTROY, WM_MOUSEACTIVATE, WM_QUIT,
};

use super::geometry::{ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, shape_for};
use super::render::Renderer;
use super::window::IslandWindow;
use crate::config::Style;
use crate::model::{MediaInfo, Notice, NoticeKind, Page, View};

const PHASE: Duration = Duration::from_millis(2000);
const STEPS: u32 = 8;

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
    println!("demo: 2 s per view, Pill then Notch; kill the process to stop");
    let start = Instant::now();
    // Step whose resting (t = 1) frame was drawn last; frames stop until the next step.
    let mut settled = u32::MAX;
    loop {
        if !pump_messages() {
            return;
        }
        let elapsed = start.elapsed();
        let step = (elapsed.as_millis() / PHASE.as_millis()) as u32 % STEPS;
        let into = elapsed.as_millis() % PHASE.as_millis();
        let t = (into as f32 / ANIM_MS as f32).min(1.0);
        // Redraw only while animating, as the app will, always ending on the resting frame.
        if step != settled {
            if t >= 1.0 {
                settled = step;
            }
            let (style, view, to, to_opacity) = step_target(step);
            let (_, _, from, from_opacity) = step_target((step + STEPS - 1) % STEPS);
            let shape = lerp_shape(from, to, ease_out_back(t));
            let opacity = from_opacity + (to_opacity - from_opacity) * t;
            let shown = matches!(view, View::MediaCompact | View::Panel(_));
            let media = shown.then_some(&media);
            if let Err(error) = renderer.draw(&win, style, shape, &view, media, opacity) {
                eprintln!("demo: draw failed: {error}");
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Style, view, resting shape and opacity of a demo step. A hidden step keeps
/// the previous step's style and shape and fades it out.
fn step_target(step: u32) -> (Style, View, ShapeRect, f32) {
    let style = if step < 4 { Style::Pill } else { Style::Notch };
    let kind = match step % 4 {
        0 => {
            let (style, _, shape, _) = step_target((step + STEPS - 1) % STEPS);
            return (style, View::Hidden, shape, 0.0);
        }
        1 => ShapeKind::MediaCompact,
        2 => ShapeKind::Panel,
        _ => ShapeKind::Notice,
    };
    let view = match kind {
        ShapeKind::Idle => View::Idle,
        ShapeKind::MediaCompact => View::MediaCompact,
        ShapeKind::PomodoroCompact => View::PomodoroCompact,
        ShapeKind::Panel => View::Panel(Page::Media),
        ShapeKind::Notice => View::Notice(demo_notice(style)),
    };
    (style, view, shape_for(style, kind), 1.0)
}

fn demo_notice(style: Style) -> Notice {
    match style {
        Style::Pill => Notice {
            key: "demo-wa".into(),
            source: "WhatsApp".into(),
            title: "Ibu (Grup Keluarga Besar Bani Hartono Sedunia)".into(),
            body: "udah makan belum? jangan lupa minum obat ya nak, nanti malam telepon ibu".into(),
            kind: NoticeKind::App { app_id: None },
        },
        Style::Notch => Notice {
            key: "demo-claude".into(),
            source: "Claude".into(),
            title: "Claude selesai".into(),
            body: "D:\\GitHub\\Dynamic Island with a very long project path that must be trimmed"
                .into(),
            kind: NoticeKind::ClaudeDone {
                session_id: "demo".into(),
            },
        },
    }
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
