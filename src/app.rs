//! The running island. One UI thread owns the window, renderer and state;
//! background sources queue `Event`s on a channel and post `WM_APP_EVENT` to
//! wake it. Nothing redraws unless an event arrives or an animation runs.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, MA_NOACTIVATE, MSG, PostMessageW,
    PostQuitMessage, SetTimer, TranslateMessage, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_TIMER,
};
use windows::core::w;

use crate::claude::pipe_server::spawn_pipe_server;
use crate::config::{self, Style};
use crate::model::{Effect, Event, View};
use crate::sources::battery::{BatteryMonitor, battery_notice, read_battery};
use crate::sources::fullscreen::is_fullscreen;
use crate::sources::{media, notifications};
use crate::state::IslandState;
use crate::ui::geometry::{ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, shape_for};
use crate::ui::render::Renderer;
use crate::ui::window::IslandWindow;

/// Posted by background sources after queueing an event.
const WM_APP_EVENT: u32 = WM_APP + 1;
/// Lives in the Controls feature of `windows`; the value is fixed by Win32.
const WM_MOUSELEAVE: u32 = 0x02A3;

const TIMER_SYSTEM: usize = 1;
const TIMER_BATTERY: usize = 2;
const TIMER_ANIM: usize = 3;
const TIMER_DEADLINE: usize = 4;
const SYSTEM_MS: u32 = 1000;
const BATTERY_MS: u32 = 30_000;
const FRAME_MS: u32 = 16;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Work that arrived while `APP` was borrowed (a message sent back into the
    /// window procedure mid-handler); done once the outer handler returns.
    static DEFERRED_DRAIN: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_REPOSITION: Cell<bool> = const { Cell::new(false) };
}

pub fn run() {
    if !claim_single_instance() {
        return;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let config = config::load();
    let win = IslandWindow::create(Some(wnd_proc)).expect("create island window");
    let renderer = Renderer::new().expect("create renderer");
    let hwnd = win.hwnd();
    let (tx, rx) = mpsc::channel();

    let rest = shape_for(config.style, ShapeKind::Compact);
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            hwnd,
            win: Some(win),
            renderer,
            style: config.style,
            state: IslandState::new(),
            rx,
            anim: Anim::settled(rest, 0.0),
            drawn: View::Hidden,
            fullscreen: false,
            battery: BatteryMonitor::new(config.battery_threshold),
            tracking_mouse: false,
        });
    });
    with_app(|app| {
        app.check_fullscreen();
        app.check_battery();
        app.draw(Instant::now());
        unsafe {
            SetTimer(Some(hwnd), TIMER_SYSTEM, SYSTEM_MS, None);
            SetTimer(Some(hwnd), TIMER_BATTERY, BATTERY_MS, None);
        }
    });

    // HWND is not Send; the sources get the raw value and rebuild it to post.
    let raw = hwnd.0 as isize;
    let wake = move || unsafe {
        let _ = PostMessageW(
            Some(HWND(raw as *mut c_void)),
            WM_APP_EVENT,
            WPARAM(0),
            LPARAM(0),
        );
    };
    notifications::spawn_poller(tx.clone(), wake, config.muted_apps);
    media::spawn_watcher(tx.clone(), wake);
    spawn_pipe_server(tx, wake);

    unsafe {
        let mut msg = MSG::default();
        // 0 is WM_QUIT and -1 an error; both end the loop.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// False when another island already runs in this session. The mutex handle
/// is never closed, so it is held for the life of the process.
fn claim_single_instance() -> bool {
    unsafe {
        match CreateMutexW(None, false, w!("Local\\DynamicIsland")) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            // Cannot tell; better to run than to show nothing.
            Err(_) => true,
        }
    }
}

struct App {
    hwnd: HWND,
    /// Taken out only while `reposition` runs, so no borrow is held across it.
    win: Option<IslandWindow>,
    renderer: Renderer,
    style: Style,
    state: IslandState,
    rx: Receiver<Event>,
    anim: Anim,
    /// The view frames draw: the current one, or the last visible one while
    /// fading out, so the content fades with the shape.
    drawn: View,
    fullscreen: bool,
    battery: BatteryMonitor,
    tracking_mouse: bool,
}

impl App {
    fn drain(&mut self) {
        let now = Instant::now();
        while let Ok(event) = self.rx.try_recv() {
            self.apply(event, now);
        }
        self.refresh(now);
    }

    fn apply(&mut self, event: Event, now: Instant) {
        // A session without metadata yet has nothing to show.
        let event = match event {
            Event::Media(Some(m)) if m.title.is_empty() => Event::Media(None),
            other => other,
        };
        for effect in self.state.apply(event, now) {
            match effect {
                Effect::OpenApp(app_id) => notifications::open_app(&app_id),
                Effect::Media(cmd) => media::send_command(cmd),
            }
        }
    }

    fn apply_now(&mut self, event: Event) {
        let now = Instant::now();
        self.apply(event, now);
        self.refresh(now);
    }

    /// Retargets the animation at the state's view and re-arms the notice deadline.
    fn refresh(&mut self, now: Instant) {
        let view = self.state.view();
        let kind = match &view {
            View::Hidden => None,
            View::MediaCompact => Some(ShapeKind::Compact),
            View::MediaExpanded => Some(ShapeKind::MediaExpanded),
            View::Notice(_) => Some(ShapeKind::Notice),
        };
        let target = kind.map(|kind| shape_for(self.style, kind));
        let retarget = match target {
            Some(shape) => self.anim.to != shape || self.anim.to_opacity != 1.0,
            None => self.anim.to_opacity != 0.0,
        };
        if view != View::Hidden {
            self.drawn = view;
        }
        if retarget {
            let (shape, opacity) = self.anim.frame(now);
            // Hidden keeps the shape where it is and only fades it out.
            let (to, to_opacity) = target.map_or((shape, 0.0), |s| (s, 1.0));
            self.anim = Anim {
                from: shape,
                from_opacity: opacity,
                to,
                to_opacity,
                start: now,
                running: true,
            };
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_ANIM, FRAME_MS, None);
            }
        } else if !self.anim.running {
            // Same shape, but the content (track, notice text) may have changed.
            self.draw(now);
        }
        self.arm_deadline(now);
    }

    fn arm_deadline(&self, now: Instant) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_DEADLINE);
            if let Some(deadline) = self.state.next_deadline() {
                // Round up so the timer never fires before the notice expires.
                let wait = deadline.saturating_duration_since(now) + Duration::from_millis(1);
                let ms = wait.as_millis().min(u32::MAX as u128) as u32;
                SetTimer(Some(self.hwnd), TIMER_DEADLINE, ms, None);
            }
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_SYSTEM => self.check_fullscreen(),
            TIMER_BATTERY => self.check_battery(),
            TIMER_ANIM => self.animate(),
            TIMER_DEADLINE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_DEADLINE);
                }
                self.apply_now(Event::Tick);
            }
            _ => {}
        }
    }

    fn animate(&mut self) {
        let now = Instant::now();
        let done = self.anim.progress(now) >= 1.0;
        // The resting t = 1 frame is always drawn before the timer stops, so a
        // fade-out never leaves faint pixels that still catch clicks.
        self.draw(now);
        if done {
            self.anim.running = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_ANIM);
            }
        }
    }

    fn check_fullscreen(&mut self) {
        let fullscreen = is_fullscreen();
        if fullscreen != self.fullscreen {
            self.fullscreen = fullscreen;
            self.apply_now(Event::Fullscreen(fullscreen));
        }
    }

    fn check_battery(&mut self) {
        if let Some((percent, charging)) = read_battery()
            && self.battery.update(percent, charging)
        {
            self.apply_now(Event::Notice(battery_notice(percent)));
        }
    }

    fn on_mouse_move(&mut self) {
        if self.tracking_mouse {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        self.tracking_mouse = unsafe { TrackMouseEvent(&mut track) }.is_ok();
        self.apply_now(Event::Hover(true));
    }

    fn on_mouse_leave(&mut self) {
        self.tracking_mouse = false;
        self.apply_now(Event::Hover(false));
    }

    fn on_click(&mut self, lparam: LPARAM) {
        // Client coordinates in physical pixels, signed 16-bit each.
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        if let Some(hit) = self.renderer.hit_test(x, y) {
            self.apply_now(Event::Click(hit));
        }
    }

    fn draw(&mut self, now: Instant) {
        let Some(win) = &self.win else { return };
        let (shape, opacity) = self.anim.frame(now);
        let media = self.state.media();
        if let Err(error) = self
            .renderer
            .draw(win, self.style, shape, &self.drawn, media, opacity)
        {
            eprintln!("dynamic-island: draw failed: {error}");
        }
    }
}

/// Shape and opacity morphing from `from` to `to` over `ANIM_MS`.
#[derive(Clone, Copy)]
struct Anim {
    from: ShapeRect,
    from_opacity: f32,
    to: ShapeRect,
    to_opacity: f32,
    start: Instant,
    running: bool,
}

impl Anim {
    fn settled(shape: ShapeRect, opacity: f32) -> Self {
        Self {
            from: shape,
            from_opacity: opacity,
            to: shape,
            to_opacity: opacity,
            start: Instant::now(),
            running: false,
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        if !self.running {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        (elapsed / ANIM_MS as f32).min(1.0)
    }

    fn frame(&self, now: Instant) -> (ShapeRect, f32) {
        let t = self.progress(now);
        if t >= 1.0 {
            return (self.to, self.to_opacity);
        }
        let shape = lerp_shape(self.from, self.to, ease_out_back(t));
        let opacity = self.from_opacity + (self.to_opacity - self.from_opacity) * t;
        (shape, opacity)
    }
}

/// Runs `f` on the app, or returns `None` when it does not exist yet or is
/// already borrowed by a handler further up the stack.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut app = cell.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

/// Re-centers and resizes for the primary monitor's current DPI, then redraws.
/// `IslandWindow::reposition` calls `SetWindowPos`, which can send
/// `WM_DPICHANGED` straight back here; the window is taken out of `App` for
/// the call, so no borrow is held and the nested call finds it gone and returns.
fn reposition() {
    let Some(taken) = with_app(|app| app.win.take()) else {
        // Arrived mid-handler: redo once the handler returns.
        DEFERRED_REPOSITION.set(true);
        return;
    };
    let Some(mut win) = taken else {
        // Nested inside reposition itself, which already reads the new placement.
        return;
    };
    win.reposition();
    with_app(move |app| {
        app.win = Some(win);
        app.draw(Instant::now());
    });
}

/// Runs work that `DEFERRED_*` postponed, once no handler holds the app.
fn run_deferred() {
    if APP.with(|cell| cell.try_borrow_mut().is_err()) {
        return;
    }
    if DEFERRED_REPOSITION.take() {
        reposition();
    }
    if DEFERRED_DRAIN.take() {
        with_app(App::drain);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = match msg {
        WM_APP_EVENT => {
            if with_app(App::drain).is_none() {
                DEFERRED_DRAIN.set(true);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_app(App::on_mouse_move);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(App::on_mouse_leave);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_app(|app| app.on_click(lparam));
            LRESULT(0)
        }
        // WM_RBUTTONUP opens the tray menu once it exists (Task 11).
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            reposition();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    };
    run_deferred();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn morph(start: Instant) -> Anim {
        Anim {
            from: shape_for(Style::Pill, ShapeKind::Compact),
            from_opacity: 0.0,
            to: shape_for(Style::Pill, ShapeKind::Notice),
            to_opacity: 1.0,
            start,
            running: true,
        }
    }

    #[test]
    fn settled_anim_rests_on_its_shape() {
        let shape = shape_for(Style::Notch, ShapeKind::MediaExpanded);
        let anim = Anim::settled(shape, 0.0);
        assert_eq!(anim.progress(Instant::now()), 1.0);
        assert_eq!(anim.frame(Instant::now()), (shape, 0.0));
    }

    #[test]
    fn anim_starts_at_from() {
        let t0 = Instant::now();
        let anim = morph(t0);
        assert_eq!(anim.frame(t0), (anim.from, 0.0));
    }

    #[test]
    fn anim_ends_exactly_on_target() {
        let t0 = Instant::now();
        let anim = morph(t0);
        let end = t0 + Duration::from_millis(ANIM_MS);
        assert_eq!(anim.progress(end), 1.0);
        assert_eq!(anim.frame(end), (anim.to, 1.0));
        assert_eq!(anim.frame(end + Duration::from_secs(5)), (anim.to, 1.0));
    }

    #[test]
    fn anim_midway_fades_linearly() {
        let t0 = Instant::now();
        let anim = morph(t0);
        let (_, opacity) = anim.frame(t0 + Duration::from_millis(ANIM_MS / 2));
        assert!((opacity - 0.5).abs() < 0.01, "opacity {opacity}");
    }
}
