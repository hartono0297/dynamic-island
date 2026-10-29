//! The boxes inside windows (a panel, a toolbar, a picture, a table), found
//! through UI Automation on a short-lived worker thread.
//!
//! The overlay never waits for it: it snaps to the windows at once, and the
//! finer boxes are added when a result arrives. The work is capped: the
//! frontmost `MAX_WINDOWS` windows only, `MAX_ELEMENTS` boxes from each, and
//! `BUDGET` in all, checked between windows (a window that answers slowly
//! costs its own time, never the overlay's).

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_BoundingRectanglePropertyId,
    UIA_IsOffscreenPropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::win::TopWindow;
use super::{Candidate, MIN_SNAP, Rect};

/// How many of the frontmost windows are searched.
pub const MAX_WINDOWS: usize = 6;
/// Boxes taken from one window.
pub const MAX_ELEMENTS: usize = 400;
/// Time for all of it; no new window is started after this.
pub const BUDGET: Duration = Duration::from_millis(600);

/// The overlay's `WM_APP + n` for a result: `wparam` is the capture's
/// number, `lparam` a `Box<Vec<Candidate>>` (see `take_result`).
pub const WM_APP_UIA: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 41;

/// The windows worth searching: the frontmost `MAX_WINDOWS`.
pub fn searched(windows: &[TopWindow]) -> &[TopWindow] {
    &windows[..windows.len().min(MAX_WINDOWS)]
}

/// Whether another window may still be started `elapsed` into the budget.
pub fn may_start(elapsed: Duration) -> bool {
    elapsed < BUDGET
}

/// The usable boxes of one window: clipped to the window, and not smaller
/// than `MIN_SNAP` (those could never be picked), at most `MAX_ELEMENTS`.
pub fn candidates_of(window: &TopWindow, rects: impl IntoIterator<Item = Rect>) -> Vec<Candidate> {
    rects
        .into_iter()
        .map(|r| r.intersect(&window.rect))
        .filter(|r| r.w >= MIN_SNAP && r.h >= MIN_SNAP)
        .take(MAX_ELEMENTS)
        .map(|rect| Candidate {
            rect,
            z: window.z,
            depth: 1,
        })
        .collect()
}

/// Starts the worker. Each window's boxes are posted to `overlay` as soon
/// as that window is done, tagged with `capture` (a result for an older
/// capture is dropped by the overlay). `stop` is checked between windows:
/// the overlay raises it when it closes. False when the thread could not
/// start.
pub fn spawn(
    overlay: HWND,
    capture: usize,
    windows: Vec<TopWindow>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    // The HWND is not Send; the thread gets it as a number.
    let target = overlay.0 as isize;
    std::thread::Builder::new()
        .name("capture-uia".into())
        .spawn(move || {
            let inited = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let started = Instant::now();
            if let Some(uia) = unsafe {
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            }
            .ok()
            {
                for window in searched(&windows) {
                    if !may_start(started.elapsed())
                        || stop.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        break;
                    }
                    let found = unsafe { boxes_of(&uia, window) }.unwrap_or_default();
                    if found.is_empty() {
                        continue;
                    }
                    post(target, capture, found);
                }
            }
            if inited {
                unsafe { CoUninitialize() };
            }
        })
        .is_ok()
}

/// Hands `found` to the overlay. A window that is gone takes nothing: the
/// box comes back and is freed here.
fn post(target: isize, capture: usize, found: Vec<Candidate>) {
    let raw = Box::into_raw(Box::new(found));
    let sent = unsafe {
        PostMessageW(
            Some(HWND(target as *mut _)),
            WM_APP_UIA,
            WPARAM(capture),
            LPARAM(raw as isize),
        )
    };
    if sent.is_err() {
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// The result `post` made, taken back once (the overlay frees it by
/// dropping it).
///
/// # Safety
/// `lparam` must be the `LPARAM` of a `WM_APP_UIA` message, and each
/// message may be taken once.
pub unsafe fn take_result(lparam: LPARAM) -> Option<Vec<Candidate>> {
    let raw = lparam.0 as *mut Vec<Candidate>;
    (!raw.is_null()).then(|| *unsafe { Box::from_raw(raw) })
}

/// Every on-screen descendant of `window`'s top-level element, with only its
/// bounding rectangle cached (one cross-process call for the lot).
unsafe fn boxes_of(uia: &IUIAutomation, window: &TopWindow) -> Option<Vec<Candidate>> {
    unsafe {
        let root = uia
            .ElementFromHandle(HWND(window.hwnd as *mut _))
            .ok()?;
        let cache = uia.CreateCacheRequest().ok()?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
        let on_screen = uia
            .CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))
            .ok()?;
        let all = root
            .FindAllBuildCache(TreeScope_Descendants, &on_screen, &cache)
            .ok()?;
        let n = all.Length().ok()?.max(0) as usize;
        let mut rects = Vec::with_capacity(n.min(MAX_ELEMENTS * 2));
        for i in 0..n.min(MAX_ELEMENTS * 2) {
            let Ok(element) = all.GetElement(i as i32) else {
                continue;
            };
            if let Ok(r) = element.CachedBoundingRectangle() {
                rects.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
            }
        }
        Some(candidates_of(window, rects))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(z: u32) -> TopWindow {
        TopWindow {
            hwnd: 0,
            rect: Rect::new(0, 0, 400, 300),
            z,
        }
    }

    #[test]
    fn only_the_frontmost_windows_are_searched() {
        let many: Vec<TopWindow> = (0..10).map(top).collect();
        let s = searched(&many);
        assert_eq!(s.len(), MAX_WINDOWS);
        assert_eq!(s[0].z, 0);
        assert_eq!(searched(&many[..2]).len(), 2);
        assert!(searched(&[]).is_empty());
    }

    #[test]
    fn the_budget_stops_new_windows_not_old_ones() {
        assert!(may_start(Duration::ZERO));
        assert!(may_start(Duration::from_millis(599)));
        assert!(!may_start(Duration::from_millis(600)));
    }

    #[test]
    fn boxes_are_clipped_to_their_window_and_tiny_ones_dropped() {
        let w = top(2);
        let found = candidates_of(
            &w,
            [
                Rect::new(10, 10, 100, 50),
                // Hangs out of the window: clipped.
                Rect::new(350, 250, 200, 200),
                // Entirely outside, and an icon's hit box.
                Rect::new(900, 900, 50, 50),
                Rect::new(20, 20, 8, 8),
            ],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].rect, Rect::new(10, 10, 100, 50));
        assert_eq!(found[1].rect, Rect::new(350, 250, 50, 50));
        assert!(found.iter().all(|c| c.z == 2 && c.depth == 1));
    }

    #[test]
    fn a_window_gives_at_most_four_hundred_boxes() {
        let w = top(0);
        let many = (0..1000).map(|i| Rect::new(i % 300, 0, 40, 40));
        assert_eq!(candidates_of(&w, many).len(), MAX_ELEMENTS);
    }
}
