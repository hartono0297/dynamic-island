//! The boxes inside windows (a panel, a toolbar, a picture, a table), found
//! through UI Automation on a short-lived worker thread.
//!
//! The overlay never waits for it: it snaps to the windows at once, and the
//! finer boxes are added when a result arrives. The work is capped: the
//! frontmost `MAX_WINDOWS` windows only, `MAX_ELEMENTS` boxes from each
//! (`MAX_VISITED` elements looked at), and `BUDGET` in all.
//!
//! A window is walked breadth first, one level of one element per call
//! (`FindAllBuildCache` with `TreeScope_Children`), never its whole subtree
//! at once: so the caps limit the work itself, and the large panes near the
//! top of the tree come before the small leaves deep in any one of them.
//! Before every call the budget is checked, the stop flag read, and the UI
//! Automation connection and transaction timeouts set to what is left, so
//! neither one heavy window nor a hung one holds the worker past the budget.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationElement, TreeScope_Children,
    UIA_BoundingRectanglePropertyId, UIA_IsOffscreenPropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::win::TopWindow;
use super::{Candidate, MIN_SNAP, Rect};

/// How many of the frontmost windows are searched.
pub const MAX_WINDOWS: usize = 6;
/// Boxes taken from one window.
pub const MAX_ELEMENTS: usize = 400;
/// Elements looked at in one window, usable as a box or not.
pub const MAX_VISITED: usize = MAX_ELEMENTS * 2;
/// Time for all of it; no call is started after this.
pub const BUDGET: Duration = Duration::from_millis(600);

/// The overlay's `WM_APP + n` for a result: `wparam` is the capture's
/// number, `lparam` a ticket for the boxes (see `take_result`).
pub const WM_APP_UIA: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 41;

/// Results posted and not yet taken, by ticket. The message carries only the
/// ticket, so a stray `WM_APP_UIA` from another process finds nothing to take
/// instead of a pointer to free.
static INBOX: Mutex<Vec<(usize, Vec<Candidate>)>> = Mutex::new(Vec::new());
static NEXT_TICKET: AtomicUsize = AtomicUsize::new(1);

/// Keeps `found` until its ticket is taken.
fn deposit(found: Vec<Candidate>) -> usize {
    let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
    INBOX
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push((ticket, found));
    ticket
}

/// The boxes under `ticket`, once; None for a ticket never issued or taken.
fn withdraw(ticket: usize) -> Option<Vec<Candidate>> {
    let mut inbox = INBOX.lock().unwrap_or_else(|p| p.into_inner());
    let at = inbox.iter().position(|(t, _)| *t == ticket)?;
    Some(inbox.swap_remove(at).1)
}

/// The windows worth searching: the frontmost `MAX_WINDOWS`.
pub fn searched(windows: &[TopWindow]) -> &[TopWindow] {
    &windows[..windows.len().min(MAX_WINDOWS)]
}

/// Whether another call may still be started `elapsed` into the budget.
pub fn may_start(elapsed: Duration) -> bool {
    elapsed < BUDGET
}

/// The connection and transaction timeout for a call started `elapsed`
/// into the budget: what is left of it, in whole milliseconds, at least 1.
pub fn timeout_ms(elapsed: Duration) -> u32 {
    let left = BUDGET.saturating_sub(elapsed).as_millis();
    left.clamp(1, u128::from(u32::MAX)) as u32
}

/// `r` clipped to the window, or None when what is left is smaller than
/// `MIN_SNAP` (it could never be picked). Such an element is not opened
/// either: its children lie inside it.
pub fn usable(window: &TopWindow, r: Rect) -> Option<Rect> {
    let r = r.intersect(&window.rect);
    (r.w >= MIN_SNAP && r.h >= MIN_SNAP).then_some(r)
}

/// The usable boxes of one window (`usable`), at most `MAX_ELEMENTS`.
pub fn candidates_of(window: &TopWindow, rects: impl IntoIterator<Item = Rect>) -> Vec<Candidate> {
    rects
        .into_iter()
        .filter_map(|r| usable(window, r))
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
/// capture is dropped by the overlay). `stop` is checked before every call:
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
            // CUIAutomation8 (Windows 8 and later) gives IUIAutomation2,
            // whose timeouts keep each call inside the budget.
            if let Ok(uia) = unsafe {
                CoCreateInstance::<_, IUIAutomation2>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            } {
                let clock = Clock {
                    uia: &uia,
                    started: Instant::now(),
                    stop: &stop,
                };
                for window in searched(&windows) {
                    let Some(found) = (unsafe { boxes_of(&clock, window) }) else {
                        // Out of time, stopped, or no limit could be set.
                        break;
                    };
                    if !found.is_empty() {
                        post(target, capture, found);
                    }
                }
            }
            if inited {
                unsafe { CoUninitialize() };
            }
        })
        .is_ok()
}

/// Hands `found` to the overlay. A window that is gone takes nothing: the
/// boxes are taken back and freed here.
fn post(target: isize, capture: usize, found: Vec<Candidate>) {
    let ticket = deposit(found);
    let sent = unsafe {
        PostMessageW(
            Some(HWND(target as *mut _)),
            WM_APP_UIA,
            WPARAM(capture),
            LPARAM(ticket as isize),
        )
    };
    if sent.is_err() {
        drop(withdraw(ticket));
    }
}

/// The result `post` deposited for a `WM_APP_UIA` message's `lparam`, taken
/// once (the overlay frees it by dropping it). Anything else gives None.
pub fn take_result(lparam: LPARAM) -> Option<Vec<Candidate>> {
    usize::try_from(lparam.0).ok().and_then(withdraw)
}

/// The budget the worker shares across every call.
struct Clock<'a> {
    uia: &'a IUIAutomation2,
    started: Instant,
    stop: &'a std::sync::atomic::AtomicBool,
}

impl Clock<'_> {
    /// Before a cross-process call: false when the budget is spent, the
    /// overlay has closed, or the timeouts could not be set to what is left
    /// (without a limit one call could outlast the budget).
    fn go(&self) -> bool {
        let elapsed = self.started.elapsed();
        if !may_start(elapsed) || self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        let left = timeout_ms(elapsed);
        unsafe {
            self.uia.SetConnectionTimeout(left).is_ok()
                && self.uia.SetTransactionTimeout(left).is_ok()
        }
    }
}

/// The on-screen elements of `window`, breadth first, with only their
/// bounding rectangles cached: one call per opened element, each limited by
/// `clock`. Stops at `MAX_ELEMENTS` usable boxes or `MAX_VISITED` elements.
/// What was found before the budget ran out is kept. None when the window
/// could not even be started (no time left, stopped, no cache request),
/// which ends the worker.
unsafe fn boxes_of(clock: &Clock, window: &TopWindow) -> Option<Vec<Candidate>> {
    let uia = clock.uia;
    unsafe {
        if !clock.go() {
            return None;
        }
        let Ok(root) = uia.ElementFromHandle(HWND(window.hwnd as *mut _)) else {
            // Closed since the list was made: the next window.
            return Some(Vec::new());
        };
        let cache = uia.CreateCacheRequest().ok()?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
        let on_screen = uia
            .CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))
            .ok()?;
        let mut rects: Vec<Rect> = Vec::new();
        let mut visited = 0usize;
        let mut open: VecDeque<IUIAutomationElement> = VecDeque::from([root]);
        'walk: while let Some(parent) = open.pop_front() {
            if !clock.go() {
                break;
            }
            let Ok(children) = parent.FindAllBuildCache(TreeScope_Children, &on_screen, &cache)
            else {
                continue;
            };
            let n = children.Length().unwrap_or(0).max(0);
            for i in 0..n {
                if visited >= MAX_VISITED || rects.len() >= MAX_ELEMENTS {
                    break 'walk;
                }
                visited += 1;
                let Ok(element) = children.GetElement(i) else {
                    continue;
                };
                let Ok(r) = element.CachedBoundingRectangle() else {
                    continue;
                };
                let r = Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);
                if let Some(r) = usable(window, r) {
                    rects.push(r);
                    open.push_back(element);
                }
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
    fn each_window_gets_only_what_is_left_of_the_budget() {
        assert_eq!(timeout_ms(Duration::ZERO), 600);
        assert_eq!(timeout_ms(Duration::from_millis(450)), 150);
        // Never zero, and never past the budget.
        assert_eq!(timeout_ms(Duration::from_millis(600)), 1);
        assert_eq!(timeout_ms(Duration::from_secs(5)), 1);
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
    fn only_a_posted_result_is_taken_and_only_once() {
        let w = top(0);
        let boxes = candidates_of(&w, [Rect::new(10, 10, 100, 50)]);
        let ticket = deposit(boxes.clone());
        // A stray message's lparam is not a ticket.
        assert!(take_result(LPARAM(0)).is_none());
        assert!(take_result(LPARAM(-1)).is_none());
        assert!(take_result(LPARAM(0x7fff_0000)).is_none());
        assert_eq!(take_result(LPARAM(ticket as isize)), Some(boxes));
        assert!(take_result(LPARAM(ticket as isize)).is_none());
    }

    #[test]
    fn a_box_too_small_or_outside_is_not_usable() {
        let w = top(0);
        assert_eq!(
            usable(&w, Rect::new(-10, -10, 50, 50)),
            Some(Rect::new(0, 0, 40, 40))
        );
        assert_eq!(usable(&w, Rect::new(500, 0, 50, 50)), None);
        assert_eq!(usable(&w, Rect::new(0, 0, MIN_SNAP - 1, 100)), None);
        assert!(MAX_VISITED >= MAX_ELEMENTS);
    }

    #[test]
    fn a_window_gives_at_most_four_hundred_boxes() {
        let w = top(0);
        let many = (0..1000).map(|i| Rect::new(i % 300, 0, 40, 40));
        assert_eq!(candidates_of(&w, many).len(), MAX_ELEMENTS);
    }
}
