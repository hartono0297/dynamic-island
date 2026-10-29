//! The capture overlay: one borderless topmost window over the whole virtual
//! screen, showing the frozen frame dimmed everywhere but the highlight.
//!
//! The pointer snaps to the window under it at once; finer boxes (panels,
//! toolbars) are added when the UI Automation worker answers. A click takes
//! the highlight, a drag draws a rectangle, Enter takes the highlight, a
//! right click or Esc cancels. One overlay at a time. It exists only while
//! in use: no timer, nothing at idle, and the frame is freed when it closes.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    D2DERR_RECREATE_TARGET, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, ID2D1Bitmap};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HWND_TOPMOST, IDC_CROSS,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassExW, SW_SHOWNA,
    SWP_SHOWWINDOW, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, WM_APP, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, w};
use windows_numerics::Vector2;

use super::uia::{WM_APP_UIA, take_result};
use super::win::{self, Frozen, TopWindow};
use super::{Candidate, Rect, crop, drag_rect, is_click, pick, size_label};
use crate::i18n::{Key, tr};
use crate::shot_edit::gfx::{Align, Gfx, Target, bitmap, color, rect_f};
use crate::theme;

const CLASS: windows::core::PCWSTR = w!("ValeSnapOverlay");

/// How much of the picture stays visible outside the highlight.
const DIM: f32 = 0.55;

/// Message from `finish` for demos that quit when the overlay is cancelled.
#[allow(dead_code)]
const _WM: u32 = WM_APP;

struct State {
    hwnd: HWND,
    frame: Frozen,
    gfx: Gfx,
    target: Option<Target>,
    picture: Option<ID2D1Bitmap>,
    cands: Vec<Candidate>,
    /// Which capture the UI Automation results belong to.
    id: usize,
    /// Raised when the overlay closes so the worker stops between windows.
    stop: Arc<AtomicBool>,
    /// Pointer in virtual-screen pixels.
    cursor: (i32, i32),
    hover: Option<Rect>,
    down: Option<(i32, i32)>,
    drag: Option<Rect>,
    accent: u32,
    demo: bool,
}

thread_local! {
    static OVERLAY: RefCell<Option<State>> = const { RefCell::new(None) };
    /// True from the first trigger until the window is up (the grab takes a
    /// moment): a second trigger is ignored.
    static STARTING: Cell<bool> = const { Cell::new(false) };
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// True while an overlay is up or being made.
pub fn active() -> bool {
    STARTING.get() || OVERLAY.with(|c| c.try_borrow().map_or(true, |s| s.is_some()))
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    OVERLAY.with(|c| c.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// Freezes the real screen and puts the overlay up. Ignored while one is
/// already up. The caller (the shortcut runner) is outside the app borrow.
pub fn start() {
    if active() {
        return;
    }
    STARTING.set(true);
    let up = (|| {
        let bounds = win::virtual_screen();
        let windows = win::top_windows(bounds);
        let frame = win::grab_virtual_screen()?;
        let cands: Vec<Candidate> = windows.iter().map(TopWindow::candidate).collect();
        create(frame, cands, Some(windows), false)
    })();
    STARTING.set(false);
    if up.is_none() {
        eprintln!("vale: the capture overlay could not start");
    }
}

/// The overlay over a made-up frame with fixed candidates: no screen grab,
/// no window list, no UI Automation (the `--demo-snap` mode).
#[cfg(debug_assertions)]
pub fn start_demo(frame: Frozen, cands: Vec<Candidate>) -> bool {
    if active() {
        return false;
    }
    create(frame, cands, None, true).is_some()
}

fn register(instance: windows::Win32::Foundation::HINSTANCE) -> windows::core::Result<()> {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            let error = Error::from_thread();
            if error.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
                return Err(error);
            }
        }
    }
    Ok(())
}

fn create(
    frame: Frozen,
    cands: Vec<Candidate>,
    for_uia: Option<Vec<TopWindow>>,
    demo: bool,
) -> Option<()> {
    let area = frame.rect();
    let gfx = Gfx::new().ok()?;
    let instance = unsafe { GetModuleHandleW(None) }.ok()?.into();
    register(instance).ok()?;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Vale capture"),
            WS_POPUP,
            area.x,
            area.y,
            area.w,
            area.h,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .ok()?;
    let mut at = POINT::default();
    let _ = unsafe { GetCursorPos(&mut at) };
    let cursor = if demo {
        (area.x + area.w / 2, area.y + area.h / 2)
    } else {
        (at.x, at.y)
    };
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stop = Arc::new(AtomicBool::new(false));
    let hover = pick(&cands, cursor);
    let state = State {
        hwnd,
        frame,
        gfx,
        target: None,
        picture: None,
        cands,
        id,
        stop: stop.clone(),
        cursor,
        hover,
        down: None,
        drag: None,
        accent: theme::current_window().accent,
        demo,
    };
    let stored = OVERLAY.with(|c| match c.try_borrow_mut() {
        Ok(mut slot) if slot.is_none() => {
            *slot = Some(state);
            true
        }
        _ => false,
    });
    if !stored {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return None;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            area.x,
            area.y,
            area.w,
            area.h,
            SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    if let Some(windows) = for_uia {
        // Never waited for: the windows are snappable already.
        super::uia::spawn(hwnd, id, windows, stop);
    }
    Some(())
}

/// Closes the overlay. `choice` is the rectangle to keep (virtual-screen
/// pixels); it is cropped from the frame, the frame is freed, the window
/// closes and the editor opens at once.
fn finish(choice: Option<Rect>) {
    let Some(state) = OVERLAY.with(|c| c.try_borrow_mut().ok().and_then(|mut s| s.take())) else {
        return;
    };
    state.stop.store(true, Ordering::Relaxed);
    let shot = choice.and_then(|r| {
        crop(
            &state.frame.bgra,
            state.frame.stride(),
            state.frame.rect(),
            r,
        )
        .map(|s| (s, r))
    });
    let (hwnd, demo) = (state.hwnd, state.demo);
    // The picture, the target and the frame go now.
    drop(state);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    match shot {
        Some((shot, r)) => crate::shot_edit::window::open(shot, r.center(), demo),
        None if demo => unsafe { PostQuitMessage(0) },
        None => {}
    }
}

/// What a left click takes: the highlight, else the whole monitor under the
/// pointer.
fn click_target(s: &State) -> Rect {
    s.hover
        .unwrap_or_else(|| win::monitor_at(s.cursor).0.intersect(&s.frame.rect()))
}

/// Recomputes the highlight for the pointer; true when what is drawn changed.
fn moved(s: &mut State, cursor: (i32, i32)) -> bool {
    let (old_hover, old_drag, old_cursor) = (s.hover, s.drag, s.cursor);
    s.cursor = cursor;
    match s.down {
        Some(down) if !is_click(down, cursor) => {
            s.drag = Some(drag_rect(down, cursor, s.frame.rect()));
        }
        _ => {
            s.drag = None;
            s.hover = pick(&s.cands, cursor);
        }
    }
    // The hint follows the pointer to another monitor.
    let monitor_changed = win::monitor_at(old_cursor).0 != win::monitor_at(cursor).0;
    s.hover != old_hover || s.drag != old_drag || monitor_changed
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            unsafe {
                if let Ok(cross) = LoadCursorW(None, IDC_CROSS) {
                    SetCursor(Some(cross));
                }
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let changed = with(|s| {
                let origin = s.frame.origin;
                let p = (
                    origin.0 + i32::from(lparam.0 as i16),
                    origin.1 + i32::from((lparam.0 >> 16) as i16),
                );
                moved(s, p)
            });
            if changed == Some(true) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let set = with(|s| {
                let origin = s.frame.origin;
                let p = (
                    origin.0 + i32::from(lparam.0 as i16),
                    origin.1 + i32::from((lparam.0 >> 16) as i16),
                );
                s.cursor = p;
                s.down = Some(p);
            });
            if set.is_some() {
                unsafe {
                    let _ = SetCapture(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            let choice = with(|s| {
                let origin = s.frame.origin;
                let up = (
                    origin.0 + i32::from(lparam.0 as i16),
                    origin.1 + i32::from((lparam.0 >> 16) as i16),
                );
                let down = s.down.take()?;
                s.cursor = up;
                if is_click(down, up) {
                    // Pointing at where it was pressed: the highlight.
                    s.hover = pick(&s.cands, up);
                    Some(click_target(s))
                } else {
                    let r = drag_rect(down, up, s.frame.rect());
                    (!r.is_empty()).then_some(r)
                }
            })
            .flatten();
            if let Some(r) = choice {
                finish(Some(r));
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            finish(None);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 as u16 {
                x if x == VK_ESCAPE.0 => finish(None),
                x if x == VK_RETURN.0 => {
                    if let Some(Some(r)) = with(|s| s.hover) {
                        finish(Some(r));
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_UIA => {
            // Always taken, so its box is freed even for an older capture.
            let found = unsafe { take_result(lparam) };
            let changed = with(|s| {
                let Some(found) = found else { return false };
                if wparam.0 != s.id {
                    return false;
                }
                s.cands.extend(found);
                let was = s.hover;
                if s.down.is_none() {
                    s.hover = pick(&s.cands, s.cursor);
                }
                s.hover != was
            });
            if changed == Some(true) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            // The frozen frame no longer fits the screen.
            finish(None);
            LRESULT(0)
        }
        WM_DESTROY => {
            // Results that were posted but not taken: free their boxes.
            let mut m = MSG::default();
            while unsafe { PeekMessageW(&mut m, Some(hwnd), WM_APP_UIA, WM_APP_UIA, PM_REMOVE) }
                .as_bool()
            {
                drop(unsafe { take_result(m.lParam) });
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    unsafe {
        BeginPaint(hwnd, &mut ps);
    }
    with(|s| {
        if draw(s).is_err() {
            // A lost target: made again on the next paint.
            s.target = None;
            s.picture = None;
            invalidate(hwnd);
        }
    });
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

fn black(a: f32) -> D2D1_COLOR_F {
    color(0, a)
}

fn draw(s: &mut State) -> windows::core::Result<()> {
    let (w, h) = (s.frame.w, s.frame.h);
    if s.target.is_none() {
        s.target = Some(s.gfx.hwnd_target(s.hwnd, (w, h))?);
        s.picture = None;
    }
    let target = s.target.as_ref().expect("made above");
    let rt: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget = &target.rt;
    if s.picture.is_none() {
        s.picture = Some(bitmap(rt, w, h, &s.frame.bgra)?);
    }
    let picture = s.picture.as_ref().expect("made above");
    let brush = &target.brush;
    let origin = s.frame.origin;
    let local = |r: Rect| Rect::new(r.x - origin.0, r.y - origin.1, r.w, r.h);
    let (full_w, full_h) = (w as f32, h as f32);
    let lit = s.drag.or(s.hover).map(local);
    unsafe {
        rt.BeginDraw();
        rt.DrawBitmap(
            picture,
            Some(&rect_f(0.0, 0.0, full_w, full_h)),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
        brush.SetColor(&black(DIM));
        let dim = |x: f32, y: f32, w: f32, h: f32| {
            if w > 0.0 && h > 0.0 {
                rt.FillRectangle(&rect_f(x, y, w, h), brush);
            }
        };
        match lit {
            None => dim(0.0, 0.0, full_w, full_h),
            Some(r) => {
                let (x, y, rw, rh) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
                dim(0.0, 0.0, full_w, y);
                dim(0.0, y + rh, full_w, full_h - y - rh);
                dim(0.0, y, x, rh);
                dim(x + rw, y, full_w - x - rw, rh);
            }
        }
        let (_, dpi) = win::monitor_at(s.cursor);
        let ui = dpi as f32 / 96.0;
        if let Some(r) = lit {
            let (x, y, rw, rh) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            brush.SetColor(&color(s.accent, 1.0));
            // Just outside the highlight, so the picture inside is untouched.
            rt.DrawRectangle(
                &rect_f(x - 1.0 * ui, y - 1.0 * ui, rw + 2.0 * ui, rh + 2.0 * ui),
                brush,
                2.0 * ui,
                None,
            );
            let label = size_label(s.drag.or(s.hover).unwrap_or_default());
            let lw = s.gfx.measure(&label, 12.0 * ui, 500).0 + 16.0 * ui;
            let lh = 22.0 * ui;
            let below = y + rh + 8.0 * ui;
            let ly = if below + lh <= full_h {
                below
            } else {
                (y + 8.0 * ui).max(0.0)
            };
            let lx = x.min(full_w - lw).max(0.0);
            pill(
                rt,
                brush,
                &s.gfx,
                (lx, ly, lw, lh),
                &label,
                12.0 * ui,
                s.accent,
                ui,
            );
        }
        if s.drag.is_none() {
            let (mon, _) = win::monitor_at(s.cursor);
            let mon = local(mon);
            let hint = tr(Key::SnapHint);
            let size = 13.0 * ui;
            let hw = s.gfx.measure(hint, size, 500).0 + 36.0 * ui;
            let hh = 32.0 * ui;
            let hx = mon.x as f32 + (mon.w as f32 - hw) / 2.0;
            let hy = mon.y as f32 + 24.0 * ui;
            pill(rt, brush, &s.gfx, (hx, hy, hw, hh), hint, size, 0, ui);
        }
        let _ = Vector2::default();
        match rt.EndDraw(None, None) {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => Err(e),
            other => other,
        }
    }
}

/// A dark rounded pill with white text (the size label, the hint). `edge`
/// colours its hairline (0 for a plain white one).
#[allow(clippy::too_many_arguments)]
fn pill(
    rt: &windows::Win32::Graphics::Direct2D::ID2D1RenderTarget,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    gfx: &Gfx,
    (x, y, w, h): (f32, f32, f32, f32),
    text: &str,
    size: f32,
    edge: u32,
    ui: f32,
) {
    use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
    use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;
    let rounded = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        radiusX: h / 2.0,
        radiusY: h / 2.0,
    };
    unsafe {
        brush.SetColor(&black(0.82));
        rt.FillRoundedRectangle(&rounded, brush);
        if edge == 0 {
            brush.SetColor(&color(0xFFFFFF, 0.16));
        } else {
            brush.SetColor(&color(edge, 0.9));
        }
        rt.DrawRoundedRectangle(&rounded, brush, 1.0 * ui, None);
    }
    gfx.text_in(
        rt,
        brush,
        text,
        (x, y, w, h),
        size,
        500,
        0xFFFFFF,
        1.0,
        Align::Center,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_overlay_is_up_in_a_test() {
        assert!(!active());
    }

    #[test]
    fn the_dim_leaves_less_than_half_the_picture() {
        assert!((0.5..0.6).contains(&DIM));
    }
}
