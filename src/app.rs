//! The running island. One UI thread owns the window, renderer and state;
//! background sources queue `Event`s on a channel and post `WM_APP_EVENT` to
//! wake it. Nothing redraws unless an event arrives or an animation runs.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, MA_NOACTIVATE, MSG,
    PostMessageW, PostQuitMessage, SW_SHOWNORMAL, SetTimer, TranslateMessage, WM_APP, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONUP, WM_TIMER,
};
use windows::core::w;

use crate::claude::install::{self, InstallOutcome};
use crate::claude::pipe_server::spawn_pipe_server;
use crate::config::{self, Config, Style};
use crate::model::{Effect, Event, Notice, NoticeKind, View};
use crate::sources::battery::{BatteryMonitor, battery_notice, read_battery};
use crate::sources::fullscreen::is_fullscreen;
use crate::sources::{media, notifications};
use crate::state::IslandState;
use crate::ui::frame_clock::{FrameClock, WM_APP_FRAME};
use crate::ui::geometry::{ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, shape_for};
use crate::ui::render::Renderer;
use crate::ui::tray::{self, MenuCmd, WM_APP_TRAY};
use crate::ui::window::IslandWindow;

/// Posted by background sources after queueing an event.
const WM_APP_EVENT: u32 = WM_APP + 1;
/// Posted once the user has answered the notification-access prompt.
const WM_APP_ACCESS: u32 = WM_APP + 3;
/// These live in the Controls feature of `windows`; the values are fixed by Win32.
const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// How long the pointer must rest on the island before it expands.
const HOVER_DWELL_MS: u32 = 350;

const TIMER_SYSTEM: usize = 1;
const TIMER_BATTERY: usize = 2;
const TIMER_DEADLINE: usize = 4;
const SYSTEM_MS: u32 = 1000;
const BATTERY_MS: u32 = 30_000;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Work that arrived while `APP` was borrowed (a message sent back into the
    /// window procedure mid-handler); done once the outer handler returns.
    static DEFERRED_DRAIN: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_REPOSITION: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_FRAME: Cell<bool> = const { Cell::new(false) };
    /// The tray menu's modal loop is running; a second click must not nest another.
    static MENU_OPEN: Cell<bool> = const { Cell::new(false) };
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

    let battery = BatteryMonitor::new(config.battery_threshold);
    let _ = tray::set_autostart(config.start_with_windows);

    let rest = shape_for(config.style, ShapeKind::Compact);
    let pomodoro = config.pomodoro;
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            hwnd,
            win: Some(win),
            renderer,
            config,
            state: IslandState::new(chrono::Local::now().date_naive(), pomodoro),
            rx,
            tx: tx.clone(),
            anim: Anim::settled(rest, 0.0),
            clock: None,
            frame_log: FrameLog::from_env(),
            drawn: View::Hidden,
            fullscreen: false,
            battery,
            hover: HoverTracking::default(),
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

    let wake = poster(hwnd, WM_APP_EVENT);
    with_app(App::spawn_notifications);
    media::spawn_watcher(tx.clone(), wake);
    spawn_pipe_server(tx, wake);

    // Fails when Explorer's taskbar is not up yet; `TaskbarCreated` retries.
    let _ = tray::add_tray(hwnd);
    request_access_then_onboard(hwnd);

    unsafe {
        let mut msg = MSG::default();
        // 0 is WM_QUIT and -1 an error; both end the loop.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Posts `msg` to the island from any thread. HWND is not Send, so the
/// closure keeps the raw value and rebuilds it to post.
fn poster(hwnd: HWND, msg: u32) -> impl Fn() + Send + Copy + 'static {
    let raw = hwnd.0 as isize;
    move || unsafe {
        let _ = PostMessageW(Some(HWND(raw as *mut c_void)), msg, WPARAM(0), LPARAM(0));
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
    config: Config,
    state: IslandState,
    rx: Receiver<Event>,
    /// Kept to restart the notification poller once access is granted.
    tx: Sender<Event>,
    anim: Anim,
    /// Paces animation frames; exists only while `anim` runs.
    clock: Option<FrameClock>,
    frame_log: FrameLog,
    /// The view frames draw: the current one, or the last visible one while
    /// fading out, so the content fades with the shape.
    drawn: View,
    fullscreen: bool,
    battery: BatteryMonitor,
    hover: HoverTracking,
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
                // Task 10 wires these to the sound, config, Google login,
                // calendar sync and editor windows.
                Effect::PlaySound
                | Effect::SavePomodoro(_)
                | Effect::ConnectGoogle
                | Effect::RefreshCalendar { .. }
                | Effect::OpenNoteEditor(_)
                | Effect::OpenEventEditor { .. } => {}
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
            // Task 7/8 give idle, pomodoro and the panel their own shapes.
            View::Idle | View::MediaCompact | View::PomodoroCompact => Some(ShapeKind::Compact),
            View::Panel(_) => Some(ShapeKind::MediaExpanded),
            View::Notice(_) => Some(ShapeKind::Notice),
        };
        let target = kind.map(|kind| shape_for(self.config.style, kind));
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
            self.frame_log.start(now);
            if self.clock.is_none() {
                self.clock = Some(FrameClock::start(self.hwnd.0 as isize));
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
            TIMER_DEADLINE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_DEADLINE);
                }
                self.apply_now(Event::Tick);
            }
            _ => {}
        }
    }

    /// One frame-clock tick: draws the animation at the current time.
    fn animate(&mut self) {
        let Some(clock) = &self.clock else {
            // A tick posted just before the clock stopped.
            return;
        };
        clock.ack();
        let now = Instant::now();
        let done = self.anim.progress(now) >= 1.0;
        // The resting t = 1 frame is always drawn before the clock stops, so a
        // fade-out never leaves faint pixels that still catch clicks.
        self.draw(now);
        self.frame_log.frame(now);
        if done {
            self.anim.running = false;
            self.frame_log.finish();
            // Dropping joins the frame thread, which wakes within one frame.
            self.clock = None;
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

    /// Arms hover and leave tracking when the pointer enters; the island
    /// expands only once `WM_MOUSEHOVER` reports a dwell.
    fn on_mouse_move(&mut self) {
        if !self.hover.needs_arming() {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_HOVER | TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: HOVER_DWELL_MS,
        };
        if unsafe { TrackMouseEvent(&mut track) }.is_ok() {
            self.hover.armed();
        }
    }

    fn on_mouse_hover(&mut self) {
        if let Some(event) = self.hover.on_hover() {
            self.apply_now(event);
        }
    }

    fn on_mouse_leave(&mut self) {
        if let Some(event) = self.hover.on_leave() {
            self.apply_now(event);
        }
    }

    fn on_click(&mut self, lparam: LPARAM) {
        // Client coordinates in physical pixels, signed 16-bit each.
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        if let Some(hit) = self.renderer.hit_test(x, y) {
            self.apply_now(Event::Click(hit));
        }
    }

    /// Starts the notification poller; it exits at once without listener access.
    fn spawn_notifications(&mut self) {
        notifications::spawn_poller(
            self.tx.clone(),
            poster(self.hwnd, WM_APP_EVENT),
            self.config.muted_apps.clone(),
        );
    }

    /// Saving can only fail on a broken profile; the setting still holds for
    /// this run and there is nowhere better to report it.
    fn save_config(&self) {
        let _ = self.config.save();
    }

    fn set_style(&mut self, style: Style) {
        if self.config.style == style {
            return;
        }
        self.config.style = style;
        self.save_config();
        let now = Instant::now();
        if self.state.view() == View::Hidden {
            // Nothing to morph; rest on the new shape so the next appearance grows from it.
            self.anim = Anim::settled(shape_for(style, ShapeKind::Compact), 0.0);
            self.draw(now);
        }
        // Visible: morphs from the old style's shape to the new one.
        self.refresh(now);
    }

    fn toggle_autostart(&mut self) {
        let enabled = !self.config.start_with_windows;
        if tray::set_autostart(enabled).is_ok() {
            self.config.start_with_windows = enabled;
            self.save_config();
        }
    }

    fn finish_onboarding(&mut self) {
        self.config.onboarding_done = true;
        self.save_config();
    }

    fn draw(&mut self, now: Instant) {
        let Some(win) = &self.win else { return };
        let (shape, opacity) = self.anim.frame(now);
        let media = self.state.media();
        if let Err(error) =
            self.renderer
                .draw(win, self.config.style, shape, &self.drawn, media, opacity)
        {
            eprintln!("dynamic-island: draw failed: {error}");
        }
    }
}

/// Pointer tracking for hover-to-expand. `TrackMouseEvent` is armed on the
/// first move inside the window; `WM_MOUSEHOVER` (after `HOVER_DWELL_MS`
/// without leaving the hover rectangle) expands, so a pointer just passing
/// over does not. `WM_MOUSELEAVE` ends tracking, so the next move re-arms it.
#[derive(Default)]
struct HoverTracking {
    /// Hover and leave tracking is armed for the pointer's current visit.
    armed: bool,
    /// `Hover(true)` was sent for this visit.
    hovered: bool,
}

impl HoverTracking {
    fn needs_arming(&self) -> bool {
        !self.armed
    }

    fn armed(&mut self) {
        self.armed = true;
    }

    /// `Hover(true)` the first time the pointer dwells during a visit.
    fn on_hover(&mut self) -> Option<Event> {
        (!std::mem::replace(&mut self.hovered, true)).then_some(Event::Hover(true))
    }

    /// `Hover(false)` when this visit expanded the island.
    fn on_leave(&mut self) -> Option<Event> {
        self.armed = false;
        std::mem::take(&mut self.hovered).then_some(Event::Hover(false))
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

/// Frame intervals for measuring smoothness, on when `DI_FRAME_LOG=1`. Each
/// animation's intervals (ms, the first one measured from its start) are kept
/// in memory and appended to `%TEMP%\di-frames.log` under a marker line once
/// it ends, so logging adds no file I/O between frames.
struct FrameLog {
    path: Option<PathBuf>,
    /// Time of the previous frame while an animation is being logged.
    last: Option<Instant>,
    intervals: Vec<f32>,
}

impl FrameLog {
    fn from_env() -> Self {
        let on = std::env::var_os("DI_FRAME_LOG").is_some_and(|v| v == "1");
        Self {
            path: on.then(|| std::env::temp_dir().join("di-frames.log")),
            last: None,
            intervals: Vec::new(),
        }
    }

    /// An animation starts (or retargets, which ends the previous one).
    fn start(&mut self, now: Instant) {
        if self.path.is_some() {
            self.finish();
            self.last = Some(now);
        }
    }

    fn frame(&mut self, now: Instant) {
        let Some(last) = self.last else { return };
        let ms = now.saturating_duration_since(last).as_secs_f32() * 1000.0;
        self.intervals.push(ms);
        self.last = Some(now);
    }

    /// Appends the current animation's intervals; a write failure only loses the log.
    fn finish(&mut self) {
        let (Some(path), Some(_)) = (&self.path, self.last.take()) else {
            return;
        };
        let mut text = String::from("# animation\n");
        for ms in self.intervals.drain(..) {
            let _ = writeln!(text, "{ms:.2}");
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

/// A temporary notice from the island itself.
fn island_notice(key: &str, title: &str, body: &str) -> Notice {
    Notice {
        key: format!("dynamic-island-{key}"),
        source: "Dynamic Island".into(),
        title: title.into(),
        body: body.into(),
        kind: NoticeKind::App { app_id: None },
    }
}

fn dnd_notice() -> Notice {
    island_notice(
        "dnd",
        "Matiin popup Windows",
        "Nyalain Do Not Disturb: Settings → System → Notifications",
    )
}

fn hook_notice(outcome: InstallOutcome) -> Notice {
    const SETTINGS: &str = r"~\.claude\settings.json";
    match outcome {
        InstallOutcome::Installed => island_notice("hooks", "Hook Claude Code terpasang", SETTINGS),
        InstallOutcome::AlreadyInstalled => {
            island_notice("hooks", "Hook sudah terpasang", SETTINGS)
        }
        InstallOutcome::Error(e) => island_notice("hooks", "Gagal pasang hook", &e),
    }
}

/// Every start: asks for notification access when it is missing (a new
/// package identity starts without it, even after onboarding). On first run
/// the Do Not Disturb hint follows once the user has answered.
fn request_access_then_onboard(hwnd: HWND) {
    let asking = !notifications::access_allowed()
        && notifications::ask_access(poster(hwnd, WM_APP_ACCESS)).is_ok();
    if !asking {
        onboard_if_needed();
    }
}

/// The access prompt was answered. The poller started at launch exited
/// without access, so start it again if access is now granted.
fn on_access_answered() {
    if notifications::access_allowed() {
        with_app(App::spawn_notifications);
    }
    onboard_if_needed();
}

/// Shows the Do Not Disturb hint and Settings page once, on first run.
fn onboard_if_needed() {
    if with_app(|app| app.config.onboarding_done) == Some(false) {
        show_dnd_help();
        with_app(App::finish_onboarding);
    }
}

/// The Do Not Disturb hint as a notice, plus the Settings page it points to.
/// Settings opens outside `with_app`: the shell may pump messages meanwhile.
fn show_dnd_help() {
    with_app(|app| app.apply_now(Event::Notice(dnd_notice())));
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:notifications"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Shows the tray menu and runs the choice. The menu's modal loop dispatches
/// timer and mouse messages to `wnd_proc`, so no `APP` borrow is held across it.
fn open_menu(hwnd: HWND) {
    if MENU_OPEN.get() {
        return;
    }
    let Some(config) = with_app(|app| app.config.clone()) else {
        return;
    };
    MENU_OPEN.set(true);
    let cmd = tray::show_menu(hwnd, &config);
    MENU_OPEN.set(false);
    match cmd {
        Some(MenuCmd::StylePill) => {
            with_app(|app| app.set_style(Style::Pill));
        }
        Some(MenuCmd::StyleNotch) => {
            with_app(|app| app.set_style(Style::Notch));
        }
        Some(MenuCmd::ToggleAutostart) => {
            with_app(App::toggle_autostart);
        }
        Some(MenuCmd::ShowDndHelp) => show_dnd_help(),
        Some(MenuCmd::InstallClaudeHooks) => {
            let notice = hook_notice(install::install_for_current_exe());
            with_app(|app| app.apply_now(Event::Notice(notice)));
        }
        Some(MenuCmd::Exit) => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        None => {}
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
    if DEFERRED_FRAME.take() {
        with_app(App::animate);
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
        WM_APP_FRAME => {
            // The clock posts nothing more until this tick is handled, so it must not be lost.
            if with_app(App::animate).is_none() {
                DEFERRED_FRAME.set(true);
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
        WM_MOUSEHOVER => {
            with_app(App::on_mouse_hover);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(App::on_mouse_leave);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            // High word of wParam: signed delta, +120 per notch away from the user.
            let delta = (wparam.0 >> 16) as u16 as i16;
            with_app(|app| app.apply_now(Event::Wheel(i32::from(delta))));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_app(|app| app.on_click(lparam));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            open_menu(hwnd);
            LRESULT(0)
        }
        WM_APP_TRAY => {
            if tray::is_menu_click(lparam) {
                open_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_APP_ACCESS => {
            on_access_answered();
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            reposition();
            LRESULT(0)
        }
        WM_DESTROY => {
            tray::remove_tray(hwnd);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if tray::is_taskbar_created(msg) => {
            let _ = tray::add_tray(hwnd);
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
    fn frame_log_appends_intervals_per_animation() {
        let path = std::env::temp_dir().join(format!("di-frames-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut log = FrameLog {
            path: Some(path.clone()),
            last: None,
            intervals: Vec::new(),
        };
        let t0 = Instant::now();
        // A frame outside any animation is not logged.
        log.frame(t0);
        log.start(t0);
        log.frame(t0 + Duration::from_millis(10));
        log.frame(t0 + Duration::from_millis(26));
        // A retarget ends the running animation's entry.
        log.start(t0 + Duration::from_millis(30));
        log.frame(t0 + Duration::from_millis(37));
        log.finish();
        log.finish();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            text, "# animation\n10.00\n16.00\n# animation\n7.00\n",
            "{text:?}"
        );
    }

    #[test]
    fn frame_log_off_records_nothing() {
        let mut log = FrameLog {
            path: None,
            last: None,
            intervals: Vec::new(),
        };
        let t0 = Instant::now();
        log.start(t0);
        log.frame(t0 + Duration::from_millis(10));
        assert!(log.last.is_none() && log.intervals.is_empty());
    }

    #[test]
    fn island_notices_are_temporary_and_open_nothing() {
        for notice in [
            dnd_notice(),
            hook_notice(InstallOutcome::Installed),
            hook_notice(InstallOutcome::AlreadyInstalled),
            hook_notice(InstallOutcome::Error("x".into())),
        ] {
            assert_eq!(notice.source, "Dynamic Island");
            assert_eq!(notice.kind, NoticeKind::App { app_id: None });
        }
    }

    #[test]
    fn hook_notice_titles() {
        assert_eq!(
            hook_notice(InstallOutcome::Installed).title,
            "Hook Claude Code terpasang"
        );
        assert_eq!(
            hook_notice(InstallOutcome::AlreadyInstalled).title,
            "Hook sudah terpasang"
        );
        let failed = hook_notice(InstallOutcome::Error("rusak".into()));
        assert_eq!(failed.title, "Gagal pasang hook");
        assert_eq!(failed.body, "rusak");
    }

    #[test]
    fn dnd_notice_text() {
        let n = dnd_notice();
        assert_eq!(n.title, "Matiin popup Windows");
        assert_eq!(
            n.body,
            "Nyalain Do Not Disturb: Settings → System → Notifications"
        );
    }

    #[test]
    fn hover_needs_a_dwell_not_just_a_move() {
        let mut h = HoverTracking::default();
        // Entering arms tracking once; further moves do not re-arm or expand.
        assert!(h.needs_arming());
        h.armed();
        assert!(!h.needs_arming());
        // Leaving without a dwell never sent Hover(true), so nothing to undo.
        assert_eq!(h.on_leave(), None);
        assert!(h.needs_arming());
    }

    #[test]
    fn dwell_expands_once_and_leave_collapses() {
        let mut h = HoverTracking::default();
        h.armed();
        assert_eq!(h.on_hover(), Some(Event::Hover(true)));
        assert_eq!(h.on_hover(), None);
        assert!(!h.needs_arming());
        assert_eq!(h.on_leave(), Some(Event::Hover(false)));
        // Tracking ends on leave; the next move re-arms it.
        assert!(h.needs_arming());
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
