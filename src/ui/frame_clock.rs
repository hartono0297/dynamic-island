//! Vsync-paced ticks for animations. A helper thread waits on `DwmFlush`,
//! which returns once the compositor has presented its next frame, and posts
//! `WM_APP_FRAME` to the island. The thread exists only while an animation
//! runs, so an idle island has no frame thread at all.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the island once per compositor frame while a `FrameClock` runs.
pub const WM_APP_FRAME: u32 = WM_APP + 4;

/// Used when `DwmFlush` fails (composition off, or the session is not on a
/// display): roughly one 60 Hz frame.
const FALLBACK_FRAME: Duration = Duration::from_millis(16);

struct Shared {
    running: AtomicBool,
    /// A tick is posted and not yet handled; while set, no further tick is
    /// posted, so a slow frame never leaves a backlog of stale ticks.
    pending: AtomicBool,
}

pub struct FrameClock {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl FrameClock {
    /// Starts posting `WM_APP_FRAME` to `hwnd` after every compositor frame.
    pub fn start(hwnd: isize) -> Self {
        let shared = Arc::new(Shared {
            running: AtomicBool::new(true),
            pending: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("di-frame-clock".into())
            .spawn(move || tick_loop(hwnd, &worker))
            .ok();
        Self { shared, thread }
    }

    /// The window is handling a tick; the next compositor frame may post another.
    pub fn ack(&self) {
        self.shared.pending.store(false, Ordering::Release);
    }

    /// Ends the ticks; the thread exits after at most one more frame wait.
    pub fn stop(&self) {
        self.shared.running.store(false, Ordering::Release);
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn tick_loop(hwnd: isize, shared: &Shared) {
    let hwnd = HWND(hwnd as *mut c_void);
    while shared.running.load(Ordering::Acquire) {
        if unsafe { DwmFlush() }.is_err() {
            thread::sleep(FALLBACK_FRAME);
        }
        if !shared.running.load(Ordering::Acquire) {
            break;
        }
        if shared.pending.swap(true, Ordering::AcqRel) {
            continue;
        }
        let posted = unsafe { PostMessageW(Some(hwnd), WM_APP_FRAME, WPARAM(0), LPARAM(0)) };
        if posted.is_err() {
            // The window is gone; nobody is left to tick.
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, MSG, PM_REMOVE, PeekMessageW,
        WINDOW_EX_STYLE, WINDOW_STYLE,
    };
    use windows::core::w;

    /// A message-only window owned by the test thread, so ticks land in its queue.
    fn message_window() -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                None,
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .expect("message-only window")
        }
    }

    /// Removes and counts the queued ticks.
    fn take_ticks(hwnd: HWND) -> usize {
        let mut msg = MSG::default();
        let mut n = 0;
        while unsafe { PeekMessageW(&mut msg, Some(hwnd), WM_APP_FRAME, WM_APP_FRAME, PM_REMOVE) }
            .as_bool()
        {
            n += 1;
        }
        n
    }

    #[test]
    fn ticks_once_until_acked() {
        let hwnd = message_window();
        let clock = FrameClock::start(hwnd.0 as isize);
        // Several compositor frames pass, but the first tick was never handled.
        thread::sleep(Duration::from_millis(100));
        assert_eq!(take_ticks(hwnd), 1);
        clock.ack();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(take_ticks(hwnd), 1);
        drop(clock);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn drop_joins_the_thread_within_a_few_frames() {
        let hwnd = message_window();
        let clock = FrameClock::start(hwnd.0 as isize);
        thread::sleep(Duration::from_millis(50));
        assert!(clock.thread.as_ref().is_some_and(|t| !t.is_finished()));
        let started = Instant::now();
        drop(clock);
        assert!(started.elapsed() < Duration::from_millis(200));
        // Nothing ticks once the clock is gone.
        take_ticks(hwnd);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(take_ticks(hwnd), 0);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
