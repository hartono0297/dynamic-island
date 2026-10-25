//! The running island. One UI thread owns the window, renderer and state;
//! background sources queue `Event`s on a channel and post `WM_APP_EVENT` to
//! wake it. Nothing redraws unless an event arrives, an animation runs, or a
//! visible live element (equalizer, pomodoro countdown) needs its next frame.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindow, KillTimer,
    MA_NOACTIVATE, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, SW_SHOWNORMAL,
    SetTimer, TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_ENDSESSION, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP,
    WM_TIMER,
};
use windows::core::w;

use crate::calendar::sync::{SyncHandle, SyncRequest, post_status, save_failed_notice, spawn_sync};
use crate::calendar::{CalEvent, CalStatus, CalendarSnapshot, draft_from, new_draft};
use crate::claude::install::{self, InstallOutcome};
use crate::claude::pipe_server::spawn_pipe_server;
use crate::config::{self, Config, OnDisk, Repair, Style};
use crate::i18n::{self, Key, tr};
use crate::model::{Effect, Event, Notice, NoticeKind, Page, View};
use crate::notes::NotesStore;
use crate::pomodoro::PomoStatus;
use crate::sources::battery::{BatteryMonitor, battery_notice, read_battery};
use crate::sources::fullscreen::is_fullscreen;
use crate::sources::{media, notifications};
use crate::state::IslandState;
use crate::ui::editor::{self, EditorKind, EditorResult, WM_APP_EDITOR_RESULT};
use crate::ui::frame_clock::{FrameClock, WM_APP_FRAME};
use crate::ui::geometry::{ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, shape_for};
use crate::ui::render::{Frame, Renderer};
use crate::ui::tray::{self, MenuCmd, WM_APP_TRAY};
use crate::ui::window::IslandWindow;

/// Posted by background sources after queueing an event.
const WM_APP_EVENT: u32 = WM_APP + 1;
/// Posted once the user has answered the notification-access prompt.
const WM_APP_ACCESS: u32 = WM_APP + 3;
/// These live in the Controls feature of `windows`; the values are fixed by Win32.
const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// How long the pointer must rest on the island before it expands. Long
/// enough that a pointer crossing the notch on its way elsewhere doesn't
/// open it, short enough that a deliberate hover feels immediate.
const HOVER_DWELL_MS: u32 = 120;

const TIMER_SYSTEM: usize = 1;
const TIMER_BATTERY: usize = 2;
const TIMER_DEADLINE: usize = 4;
/// Equalizer frames, only while the compact media view shows a playing track.
const TIMER_EQ: usize = 5;
/// Pomodoro countdown redraws, only while it runs and is on screen.
const TIMER_POMO: usize = 6;
/// Date rollover check.
const TIMER_DAY: usize = 7;
const SYSTEM_MS: u32 = 1000;
const BATTERY_MS: u32 = 30_000;
/// Values table: ~30 fps equalizer.
const EQ_MS: u32 = 33;
const DAY_MS: u32 = 60_000;
/// Values table: page slide duration (ease-out).
const SLIDE_MS: f32 = 220.0;
/// Values table: the open calendar re-asks for its range once data is this old.
const CALENDAR_FRESH: Duration = Duration::from_secs(60);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Work that arrived while `APP` was borrowed (a message sent back into the
    /// window procedure mid-handler); done once the outer handler returns.
    static DEFERRED_DRAIN: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_REPOSITION: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_FRAME: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_RESULTS: RefCell<Vec<EditorResult>> = const { RefCell::new(Vec::new()) };
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
    i18n::set_lang(config.language);
    let win = IslandWindow::create(Some(wnd_proc)).expect("create island window");
    let renderer = Renderer::new().expect("create renderer");
    let hwnd = win.hwnd();
    let (tx, rx) = mpsc::channel();

    let battery = BatteryMonitor::new(config.battery_threshold);
    let _ = tray::set_autostart(config.start_with_windows);

    let rest = shape_for(config.style, ShapeKind::Idle);
    let pomodoro = config.pomodoro;
    let today = Local::now().date_naive();
    let notes = NotesStore::open(NotesStore::default_path());
    // Sends the cached calendar at once; the first range is asked for below.
    let sync = spawn_sync(tx.clone(), poster(hwnd, WM_APP_EVENT), config.clone());
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            hwnd,
            win: Some(win),
            renderer,
            config,
            state: IslandState::new(today, pomodoro),
            calendar: CalendarSnapshot::empty(CalStatus::NotConfigured, today),
            notes,
            sync: Some(sync),
            calendar_asked: None,
            editor: None,
            rx,
            tx: tx.clone(),
            anim: Anim::settled(rest, 0.0),
            slide: None,
            clock: None,
            frame_log: FrameLog::from_env(),
            drawn: View::Hidden,
            fullscreen: false,
            battery,
            hover: HoverTracking::default(),
            eq_timer: false,
            pomo_timer: false,
            config_warned: false,
        });
    });
    with_app(|app| {
        let now = Instant::now();
        app.check_fullscreen();
        app.check_battery();
        app.draw(now);
        app.request_calendar(app.state.cal_range(), now);
        // An empty notes list would otherwise look like the notes were deleted.
        if let Some(notice) = notes_unreadable_notice(&app.notes) {
            app.apply_now(Event::Notice(notice));
        }
        unsafe {
            SetTimer(Some(hwnd), TIMER_SYSTEM, SYSTEM_MS, None);
            SetTimer(Some(hwnd), TIMER_BATTERY, BATTERY_MS, None);
            SetTimer(Some(hwnd), TIMER_DAY, DAY_MS, None);
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
    /// The latest calendar data from the sync thread.
    calendar: CalendarSnapshot,
    notes: NotesStore,
    /// `None` once shut down.
    sync: Option<SyncHandle>,
    /// The range last asked of the sync thread, and when.
    calendar_asked: Option<((NaiveDate, NaiveDate), Instant)>,
    /// The open note or event editor (at most one).
    editor: Option<HWND>,
    rx: Receiver<Event>,
    /// Kept to restart the notification poller once access is granted.
    tx: Sender<Event>,
    anim: Anim,
    /// A panel page switch in progress.
    slide: Option<Slide>,
    /// Paces animation frames; exists only while `anim` or `slide` runs.
    clock: Option<FrameClock>,
    frame_log: FrameLog,
    /// The view frames draw: the current one, or the last visible one while
    /// fading out, so the content fades with the shape.
    drawn: View,
    fullscreen: bool,
    battery: BatteryMonitor,
    hover: HoverTracking,
    /// `TIMER_EQ` / `TIMER_POMO` are set.
    eq_timer: bool,
    pomo_timer: bool,
    /// The invalid-config notice was shown this run.
    config_warned: bool,
}

impl App {
    fn drain(&mut self) {
        let now = Instant::now();
        while let Ok(event) = self.rx.try_recv() {
            self.apply(event, now);
        }
        self.refresh(now);
    }

    /// Runs `event` through the reducer and performs its effects; events an
    /// effect produces (an editor opened, a notice) follow in order.
    fn apply(&mut self, event: Event, now: Instant) {
        let mut queue = VecDeque::from([event]);
        while let Some(event) = queue.pop_front() {
            let event = match event {
                // A session without metadata yet has nothing to show.
                Event::Media(Some(m)) if m.title.is_empty() => Event::Media(None),
                // Only `App` keeps the calendar; the reducer has no use for it.
                Event::Calendar(snapshot) => {
                    self.calendar = snapshot;
                    continue;
                }
                other => other,
            };
            let page = self.state.page();
            for effect in self.state.apply(event, now) {
                queue.extend(self.perform(effect, now));
            }
            self.follow_page(page, now);
        }
    }

    fn perform(&mut self, effect: Effect, now: Instant) -> Option<Event> {
        match effect {
            Effect::OpenApp(app_id) => notifications::open_app(&app_id),
            Effect::Media(cmd) => media::send_command(cmd),
            Effect::PlaySound => play_phase_sound(),
            Effect::SavePomodoro(settings) => {
                self.config.pomodoro = settings;
                self.save_config();
            }
            Effect::ConnectGoogle => return self.connect_google(),
            Effect::RefreshCalendar { from, to } => self.request_calendar((from, to), now),
            Effect::OpenNoteEditor(id) => {
                return self.open_editor(note_editor_kind(&self.notes, id.as_deref()));
            }
            Effect::OpenEventEditor { event_id, date } => {
                let kind = event_editor_kind(&self.calendar.events, event_id.as_deref(), date)?;
                return self.open_editor(kind);
            }
        }
        None
    }

    /// Slides in the page the wheel just switched to.
    fn follow_page(&mut self, before: Page, now: Instant) {
        let after = self.state.page();
        if after == before || !matches!(self.state.view(), View::Panel(_)) {
            return;
        }
        let index = |p: Page| Page::ALL.iter().position(|q| *q == p).unwrap_or(0);
        // A later page comes in from the right.
        let from = if index(after) > index(before) {
            1.0
        } else {
            -1.0
        };
        self.slide = Some(Slide { from, start: now });
        self.ensure_clock();
    }

    /// Without a client ID there is nothing to log in to; the notice says
    /// where to set it. config.json is read again first, so an ID added while
    /// the island runs is used without a restart. A second click while the
    /// browser login waits is ignored.
    fn connect_google(&mut self) -> Option<Event> {
        let file_ok = self.reload_config();
        match connect_action(file_ok, &self.config.google_client_id) {
            // The ID may well be in there; say so rather than "fill it in".
            ConnectAction::InvalidConfig => Some(Event::Notice(invalid_config_notice(None))),
            ConnectAction::NoClientId => Some(Event::Notice(client_id_notice())),
            ConnectAction::Login => {
                if let Some(sync) = &self.sync {
                    sync.connect();
                }
                None
            }
        }
    }

    /// Asks the sync thread for `range` unless `needs_fetch` says the data is fresh.
    fn request_calendar(&mut self, range: (NaiveDate, NaiveDate), now: Instant) {
        let snap = &self.calendar;
        let fetched_age = snap
            .fetched_at
            .filter(|_| snap.status == CalStatus::Ok && (snap.from, snap.to) == range)
            .and_then(|at| (Local::now() - at).to_std().ok());
        if !needs_fetch(self.calendar_asked, fetched_age, range, now) {
            return;
        }
        let (from, to) = range;
        if self
            .sync
            .as_ref()
            .is_some_and(|sync| sync.send(SyncRequest::Range { from, to }))
        {
            self.calendar_asked = Some((range, now));
        }
    }

    /// Opens an editor under the panel. If one is already open, Windows
    /// brings that one forward instead and nothing else changes.
    fn open_editor(&mut self, kind: EditorKind) -> Option<Event> {
        let win = self.win.as_ref()?;
        let panel = shape_for(self.config.style, ShapeKind::Panel);
        let anchor = win.to_screen(panel.x + panel.w / 2.0, panel.y + panel.h);
        match editor::open_editor(self.hwnd, anchor, kind) {
            Ok(hwnd) if self.editor == Some(hwnd) => None,
            Ok(hwnd) => {
                self.editor = Some(hwnd);
                // Keeps the panel open while the pointer is over the editor.
                Some(Event::EditorOpen(true))
            }
            Err(error) => {
                eprintln!("dynamic-island: cannot open the editor: {error}");
                None
            }
        }
    }

    fn on_editor_result(&mut self, result: EditorResult) {
        let now = Instant::now();
        let closes = result.closes_editor();
        match result {
            EditorResult::NoteSaved { id, text } => self.save_note(id.as_deref(), &text),
            EditorResult::NoteDeleted { id } => {
                if self.notes.delete(&id) {
                    self.persist_notes(now);
                }
            }
            EditorResult::EventSave {
                event_id,
                calendar_id,
                draft,
            } => {
                let reply = self.editor.map_or(0, |hwnd| hwnd.0 as isize);
                let request = SyncRequest::Save {
                    event_id,
                    calendar_id,
                    draft,
                    reply,
                };
                if !self.sync.as_ref().is_some_and(|sync| sync.send(request)) {
                    // No sync thread to answer: the editor must not wait forever.
                    let failed = Err(i18n::save_failed_because(
                        i18n::lang(),
                        tr(Key::ReasonSyncStopped),
                    ));
                    if let Some(message) = post_status(reply, failed) {
                        self.apply(Event::Notice(save_failed_notice(message)), now);
                    }
                }
            }
            EditorResult::Closed => {}
        }
        if closes {
            self.editor = None;
            self.apply(Event::EditorOpen(false), now);
        }
        self.refresh(now);
    }

    /// An untouched note is not saved: `upsert` would still move it to the top.
    fn save_note(&mut self, id: Option<&str>, text: &str) {
        if !note_changed(&self.notes, id, text) {
            return;
        }
        // The note's id from here on is the one `upsert` returns (a new note, or
        // one deleted meanwhile, gets a fresh one; `None` = emptied and deleted).
        // The editor is gone, so nothing else holds an id to update.
        let _id = self.notes.upsert(id, text, Local::now().timestamp());
        self.persist_notes(Instant::now());
    }

    fn persist_notes(&mut self, now: Instant) {
        if let Err(error) = self.notes.save() {
            let notice = island_notice("notes", tr(Key::NotesSaveFailed), &error.to_string());
            self.apply(Event::Notice(notice), now);
        }
    }

    /// The editor died without its last result (never expected): the panel
    /// must not stay pinned open.
    fn check_editor(&mut self) {
        if let Some(hwnd) = self.editor
            && !unsafe { IsWindow(Some(hwnd)) }.as_bool()
        {
            self.editor = None;
            self.apply_now(Event::EditorOpen(false));
        }
    }

    fn check_date(&mut self) {
        let today = Local::now().date_naive();
        if today != self.state.today() {
            self.apply_now(Event::Today(today));
        }
    }

    fn stop_sync(&mut self) {
        if let Some(sync) = self.sync.take() {
            sync.send(SyncRequest::Shutdown);
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
            View::Idle => Some(ShapeKind::Idle),
            View::MediaCompact => Some(ShapeKind::MediaCompact),
            View::PomodoroCompact => Some(ShapeKind::PomodoroCompact),
            View::Panel(_) => Some(ShapeKind::Panel),
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
            self.ensure_clock();
        } else if !self.animating() {
            // Same shape, but the content (track, notice text) may have changed.
            self.draw(now);
        }
        self.arm_deadline(now);
        self.sync_timers(now);
    }

    fn animating(&self) -> bool {
        self.anim.running || self.slide.is_some()
    }

    fn ensure_clock(&mut self) {
        if self.clock.is_none() {
            self.clock = Some(FrameClock::start(self.hwnd.0 as isize));
        }
    }

    /// Runs the equalizer and countdown timers only while what they animate
    /// is on screen and moving.
    fn sync_timers(&mut self, now: Instant) {
        let view = self.state.view();
        let eq = view == View::MediaCompact && self.state.media().is_some_and(|m| m.playing);
        if eq != self.eq_timer {
            self.eq_timer = eq;
            unsafe {
                if eq {
                    SetTimer(Some(self.hwnd), TIMER_EQ, EQ_MS, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER_EQ);
                }
            }
        }
        let shows_countdown = matches!(view, View::PomodoroCompact | View::Panel(Page::Pomodoro));
        let running = matches!(self.state.pomodoro().status(), PomoStatus::Running { .. });
        let pomo = shows_countdown && running;
        if pomo != self.pomo_timer {
            self.pomo_timer = pomo;
            if pomo {
                self.arm_pomo(now);
            } else {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_POMO);
                }
            }
        }
    }

    /// One-shot, re-armed after each redraw, so the shown second ticks over on time.
    fn arm_pomo(&self, now: Instant) {
        let wait = ms_to_next_second(self.state.pomodoro().remaining(now));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_POMO, wait, None);
        }
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
            TIMER_SYSTEM => {
                self.check_fullscreen();
                self.check_editor();
            }
            TIMER_BATTERY => self.check_battery(),
            TIMER_DEADLINE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_DEADLINE);
                }
                self.apply_now(Event::Tick);
            }
            // While an animation runs, the frame clock draws anyway.
            TIMER_EQ => {
                if !self.animating() {
                    self.draw(Instant::now());
                }
            }
            TIMER_POMO => {
                let now = Instant::now();
                if !self.animating() {
                    self.draw(now);
                }
                if self.pomo_timer {
                    self.arm_pomo(now);
                }
            }
            TIMER_DAY => self.check_date(),
            _ => {}
        }
    }

    /// One frame-clock tick: draws the shape morph and page slide at the current time.
    fn animate(&mut self) {
        let Some(clock) = &self.clock else {
            // A tick posted just before the clock stopped.
            return;
        };
        clock.ack();
        let now = Instant::now();
        let shape_done = self.anim.progress(now) >= 1.0;
        let slide_done = self.slide.is_none_or(|s| s.progress(now) >= 1.0);
        // The resting t = 1 frame is always drawn before the clock stops, so a
        // fade-out never leaves faint pixels that still catch clicks, and the
        // settled page registers its click regions.
        self.draw(now);
        self.frame_log.frame(now);
        if shape_done && self.anim.running {
            self.anim.running = false;
            self.frame_log.finish();
        }
        if slide_done {
            self.slide = None;
        }
        if !self.animating() {
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

    /// Writes the island's own settings into config.json as it is now, so
    /// fields edited by hand meanwhile survive (and are picked up). An invalid
    /// file is backed up first (`config::save_merged_at`); the first time this
    /// run, a notice says so. The setting holds for this run either way.
    fn save_config(&mut self) {
        let out = config::save_merged(&self.config);
        self.adopt_config(out.config);
        if out.repair != Repair::None && !self.config_warned {
            self.config_warned = true;
            self.apply_now(Event::Notice(invalid_config_notice(Some(&out.repair))));
        }
    }

    /// Picks up hand edits to config.json. False when the file exists but is
    /// not a valid config; it then changes nothing (only saves back it up).
    fn reload_config(&mut self) -> bool {
        match config::read_at(&config::path()) {
            OnDisk::Valid(disk) => {
                let merged = config::merge_owned(disk, &self.config);
                self.adopt_config(merged);
                true
            }
            OnDisk::Missing => true,
            OnDisk::Invalid => false,
        }
    }

    /// The sync thread gets a changed Google client before any later request.
    fn adopt_config(&mut self, cfg: Config) {
        let google = |c: &Config| (c.google_client_id.clone(), c.google_client_secret.clone());
        let changed = google(&cfg) != google(&self.config);
        self.config = cfg;
        i18n::set_lang(self.config.language);
        if changed && let Some(sync) = &self.sync {
            sync.send(SyncRequest::Config(self.config.clone()));
            // The next calendar view asks again, now with the new client.
            self.calendar_asked = None;
        }
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
            self.anim = Anim::settled(shape_for(style, ShapeKind::Idle), 0.0);
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
        let state = &self.state;
        let frame = Frame {
            view: &self.drawn,
            media: state.media(),
            pomodoro: state.pomodoro(),
            now,
            today: state.today(),
            page: state.page(),
            strip: state.waiting_strip(),
            cal_view: state.cal_view(),
            cal_anchor: state.cal_anchor(),
            cal_selected: state.cal_selected(),
            calendar: &self.calendar,
            notes: self.notes.list(),
            notes_page: state.notes_page(),
            day_page: state.day_page(),
            page_slide: self.slide.map_or(0.0, |s| s.offset(now)),
        };
        if let Err(error) = self
            .renderer
            .draw(win, self.config.style, shape, &frame, opacity)
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

/// A panel page switch: `page_slide` eases from `from` (±1) to 0 over `SLIDE_MS`.
#[derive(Clone, Copy)]
struct Slide {
    from: f32,
    start: Instant,
}

impl Slide {
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        (elapsed / SLIDE_MS).min(1.0)
    }

    fn offset(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        // Cubic ease-out: fast start, gentle landing, no overshoot past the page.
        self.from * (1.0 - t).powi(3)
    }
}

/// Whether the calendar should ask the sync thread for `range`. A different
/// range than the last one asked (navigation, or the panel opening on another
/// range) always asks. The same range asks again only once both the request
/// and the data shown for it (`fetched_age`, `None` if it is not that range or
/// the fetch failed) are older than `CALENDAR_FRESH`.
fn needs_fetch(
    asked: Option<((NaiveDate, NaiveDate), Instant)>,
    fetched_age: Option<Duration>,
    range: (NaiveDate, NaiveDate),
    now: Instant,
) -> bool {
    match asked {
        Some((last, at)) if last == range => {
            now.saturating_duration_since(at) >= CALENDAR_FRESH
                && fetched_age.is_none_or(|age| age >= CALENDAR_FRESH)
        }
        _ => true,
    }
}

/// Milliseconds until the countdown's shown second changes (it rounds up, so
/// at a whole second), plus 1 ms so the redraw lands after the change.
fn ms_to_next_second(remaining: Duration) -> u32 {
    match (remaining.as_millis() % 1000) as u32 {
        0 => 1001,
        ms => ms + 1,
    }
}

/// False when saving would change nothing: an existing note with the same
/// text, or an empty new one.
fn note_changed(notes: &NotesStore, id: Option<&str>, text: &str) -> bool {
    match id.and_then(|id| notes.get(id)) {
        Some(note) => note.text != text,
        None => !text.trim().is_empty(),
    }
}

/// The note editor for `id`; an unknown id (deleted meanwhile) opens a new note.
fn note_editor_kind(notes: &NotesStore, id: Option<&str>) -> EditorKind {
    let note = id.and_then(|id| notes.get(id));
    EditorKind::Note {
        id: note.map(|n| n.id.clone()),
        text: note.map(|n| n.text.clone()).unwrap_or_default(),
    }
}

/// The event editor for `event_id` (read-only unless the event is editable),
/// or a new event on `date`. `None` when the event is no longer in the data.
fn event_editor_kind(
    events: &[CalEvent],
    event_id: Option<&str>,
    date: NaiveDate,
) -> Option<EditorKind> {
    let Some(id) = event_id else {
        return Some(EditorKind::Event {
            event_id: None,
            calendar_id: None,
            draft: new_draft(date),
            read_only: false,
        });
    };
    let ev = events.iter().find(|e| e.id == id)?;
    Some(EditorKind::Event {
        event_id: Some(ev.id.clone()),
        calendar_id: Some(ev.calendar_id.clone()),
        draft: draft_from(ev),
        read_only: !ev.editable,
    })
}

/// Values table: Windows' default notification sound, once, without blocking;
/// silent if the sound scheme has none.
fn play_phase_sound() {
    unsafe {
        let _ = PlaySoundW(
            w!("Notification.Default"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
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

/// At startup: notes.json exists but could not be read or parsed.
fn notes_unreadable_notice(notes: &NotesStore) -> Option<Notice> {
    notes.load_failed().then(|| {
        island_notice(
            "notes-unreadable",
            tr(Key::NotesUnreadable),
            tr(Key::NotesUnreadableBody),
        )
    })
}

/// What "Hubungkan Google" does, given whether config.json is valid (or
/// absent) and the client ID in effect.
#[derive(Debug, PartialEq)]
enum ConnectAction {
    InvalidConfig,
    NoClientId,
    Login,
}

fn connect_action(file_ok: bool, client_id: &str) -> ConnectAction {
    if !file_ok {
        ConnectAction::InvalidConfig
    } else if client_id.trim().is_empty() {
        ConnectAction::NoClientId
    } else {
        ConnectAction::Login
    }
}

/// config.json does not parse: after a save (`repair`, which says where it
/// went), or on "Hubungkan Google" (`None`, nothing moved yet).
fn invalid_config_notice(repair: Option<&Repair>) -> Notice {
    let body = match repair {
        Some(Repair::BackedUp(path)) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            i18n::config_copied_to(i18n::lang(), &name)
        }
        Some(Repair::Blocked) => tr(Key::ConfigNotCopied).into(),
        Some(Repair::None) | None => tr(Key::ConfigCheck).into(),
    };
    island_notice("config-invalid", tr(Key::ConfigInvalid), &body)
}

/// "Hubungkan Google" without a client ID in `config.json`.
fn client_id_notice() -> Notice {
    Notice {
        key: "calendar-client-id".into(),
        source: tr(Key::SourceCalendar).into(),
        title: tr(Key::ClientIdMissing).into(),
        body: tr(Key::SeeReadme).into(),
        kind: NoticeKind::App { app_id: None },
    }
}

fn dnd_notice() -> Notice {
    island_notice("dnd", tr(Key::DndTitle), tr(Key::DndBody))
}

fn hook_notice(outcome: InstallOutcome) -> Notice {
    const SETTINGS: &str = r"~\.claude\settings.json";
    match outcome {
        InstallOutcome::Installed => island_notice("hooks", tr(Key::HooksInstalled), SETTINGS),
        InstallOutcome::AlreadyInstalled => {
            island_notice("hooks", tr(Key::HooksAlreadyInstalled), SETTINGS)
        }
        InstallOutcome::Error(e) => island_notice("hooks", tr(Key::HooksFailed), &e),
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
        Some(MenuCmd::Exit) => shutdown(hwnd),
        None => {}
    }
}

/// Closes the island. An open editor is closed first and its last result
/// (an edited note) handled here: once the island is gone, nobody reads it.
fn shutdown(hwnd: HWND) {
    flush_editor();
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Closes an open editor now and handles its last result synchronously.
/// Called outside `with_app`: destroying the editor sends window messages.
fn flush_editor() {
    if let Some(result) = editor::close_open_editor() {
        deliver_result(result);
    }
}

/// Handles an editor result now, or once the handler holding the app returns.
fn deliver_result(result: EditorResult) {
    let mut pending = Some(result);
    with_app(|app| {
        if let Some(result) = pending.take() {
            app.on_editor_result(result);
        }
    });
    if let Some(result) = pending {
        DEFERRED_RESULTS.with_borrow_mut(|results| results.push(result));
    }
}

/// Editor results still queued for the island (an editor destroyed along
/// with it posts its last one); read before the queue goes with the window.
fn drain_editor_results(hwnd: HWND) {
    let mut msg = MSG::default();
    while unsafe {
        PeekMessageW(
            &mut msg,
            Some(hwnd),
            WM_APP_EDITOR_RESULT,
            WM_APP_EDITOR_RESULT,
            PM_REMOVE,
        )
    }
    .as_bool()
    {
        if let Some(result) = unsafe { editor::take_result(msg.lParam) } {
            deliver_result(result);
        }
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
    for result in DEFERRED_RESULTS.take() {
        deliver_result(result);
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
        WM_APP_EDITOR_RESULT => {
            // Each result is taken exactly once, here.
            if let Some(result) = unsafe { editor::take_result(lparam) } {
                deliver_result(result);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            shutdown(hwnd);
            LRESULT(0)
        }
        // Logoff or shutdown: the process may end right after this returns.
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                flush_editor();
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            reposition();
            LRESULT(0)
        }
        WM_DESTROY => {
            drain_editor_results(hwnd);
            with_app(App::stop_sync);
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
            from: shape_for(Style::Pill, ShapeKind::Idle),
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
        let shape = shape_for(Style::Notch, ShapeKind::Panel);
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

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn calendar_asks_on_start_navigation_and_when_stale() {
        let t0 = Instant::now();
        let month = (date(1), date(31));
        let next = (date(5), date(12));
        let secs = Duration::from_secs;
        // Nothing asked yet (start).
        assert!(needs_fetch(None, None, month, t0));
        // Reopening the panel on the range just asked: wait.
        assert!(!needs_fetch(Some((month, t0)), None, month, t0 + secs(59)));
        // Navigation lands on another range: always ask, however recent.
        assert!(needs_fetch(Some((month, t0)), None, next, t0));
        // ...and back again, since the sync thread now holds the other range.
        assert!(needs_fetch(Some((next, t0)), Some(secs(1)), month, t0));
        // Same range after 60 s: ask, unless the data shown is fresher
        // (the sync thread's own 10-minute refresh).
        assert!(needs_fetch(Some((month, t0)), None, month, t0 + secs(60)));
        assert!(!needs_fetch(
            Some((month, t0)),
            Some(secs(10)),
            month,
            t0 + secs(61)
        ));
        assert!(needs_fetch(
            Some((month, t0)),
            Some(secs(70)),
            month,
            t0 + secs(61)
        ));
    }

    #[test]
    fn countdown_redraws_when_the_shown_second_changes() {
        let ms = Duration::from_millis;
        // 24:59.5 left shows 25:00 until 24:59.0, half a second away.
        assert_eq!(ms_to_next_second(ms(1_499_500)), 501);
        assert_eq!(ms_to_next_second(ms(1_500_000)), 1001);
        assert_eq!(ms_to_next_second(ms(1)), 2);
        assert_eq!(ms_to_next_second(ms(999)), 1000);
    }

    #[test]
    fn page_slide_eases_out_to_rest() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let right = Slide {
            from: 1.0,
            start: t0,
        };
        assert_eq!(right.offset(t0), 1.0);
        let mid = right.offset(ms(110));
        assert!((mid - 0.125).abs() < 1e-3, "half time, eased: {mid}");
        assert!(right.offset(ms(50)) > right.offset(ms(60)));
        assert_eq!(right.progress(ms(220)), 1.0);
        assert_eq!(right.offset(ms(220)), 0.0);
        assert_eq!(right.offset(ms(500)), 0.0);
        let left = Slide {
            from: -1.0,
            start: t0,
        };
        assert_eq!(left.offset(t0), -1.0);
        assert!(left.offset(ms(110)) < 0.0);
    }

    fn temp_notes(name: &str) -> (PathBuf, NotesStore) {
        let dir =
            std::env::temp_dir().join(format!("dynamic-island-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = NotesStore::open(dir.join("notes.json"));
        (dir, store)
    }

    #[test]
    fn only_changed_notes_are_saved() {
        let (dir, mut notes) = temp_notes("changed");
        let id = notes.upsert(None, "beli kopi", 1).unwrap();
        assert!(!note_changed(&notes, Some(&id), "beli kopi"), "untouched");
        assert!(note_changed(&notes, Some(&id), "beli teh"));
        assert!(note_changed(&notes, Some(&id), ""), "emptied: deletes");
        assert!(!note_changed(&notes, None, "  \n"), "empty new note");
        assert!(note_changed(&notes, None, "baru"));
        assert!(note_changed(&notes, Some("n0-gone"), "masih ada"));
        assert!(!dir.exists(), "nothing written");
    }

    #[test]
    fn unreadable_notes_file_is_announced() {
        let (dir, notes) = temp_notes("unreadable-none");
        assert_eq!(notes_unreadable_notice(&notes), None, "missing file");
        assert!(!dir.exists());

        let (dir, _) = temp_notes("unreadable");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.json"), "{ rusak").unwrap();
        let notes = NotesStore::open(dir.join("notes.json"));
        let notice = notes_unreadable_notice(&notes);
        let _ = std::fs::remove_dir_all(&dir);
        let notice = notice.expect("a notice");
        assert_eq!(notice.title, "notes.json tidak terbaca");
        assert_eq!(
            notice.body,
            "Disimpan sebagai notes.json.bak saat menyimpan berikutnya"
        );
    }

    #[test]
    fn note_editor_opens_the_note_or_a_new_one() {
        let (_dir, mut notes) = temp_notes("kind");
        let id = notes.upsert(None, "isi", 1).unwrap();
        assert_eq!(
            note_editor_kind(&notes, Some(&id)),
            EditorKind::Note {
                id: Some(id),
                text: "isi".into()
            }
        );
        let new = EditorKind::Note {
            id: None,
            text: String::new(),
        };
        assert_eq!(note_editor_kind(&notes, None), new);
        assert_eq!(note_editor_kind(&notes, Some("n0-gone")), new);
    }

    #[test]
    fn event_editor_is_read_only_unless_editable() {
        use crate::calendar::EventTime;
        let event = |id: &str, editable| CalEvent {
            id: id.into(),
            calendar_id: "team".into(),
            title: id.into(),
            start: EventTime::AllDay(date(25)),
            end: EventTime::AllDay(date(26)),
            description: String::new(),
            color: None,
            editable,
        };
        let events = [event("rw", true), event("ro", false)];
        let open = |id| match event_editor_kind(&events, id, date(27)) {
            Some(EditorKind::Event {
                event_id,
                calendar_id,
                draft,
                read_only,
            }) => (event_id, calendar_id, draft.date, read_only),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            open(Some("rw")),
            (Some("rw".into()), Some("team".into()), date(25), false)
        );
        assert_eq!(
            open(Some("ro")),
            (Some("ro".into()), Some("team".into()), date(25), true)
        );
        // New: on the selected date, writable, into the primary calendar.
        assert_eq!(open(None), (None, None, date(27), false));
        assert_eq!(event_editor_kind(&events, Some("gone"), date(27)), None);
    }

    #[test]
    fn connect_on_an_invalid_config_says_so() {
        let id = "id.apps.googleusercontent.com";
        // The ID may be in the broken file; "fill in the ID" would mislead.
        assert_eq!(connect_action(false, ""), ConnectAction::InvalidConfig);
        assert_eq!(connect_action(false, id), ConnectAction::InvalidConfig);
        assert_eq!(connect_action(true, "  "), ConnectAction::NoClientId);
        assert_eq!(connect_action(true, id), ConnectAction::Login);
    }

    #[test]
    fn invalid_config_notice_says_where_the_file_went() {
        let bak = PathBuf::from(r"C:\x\DynamicIsland\config.json.bak");
        let saved = invalid_config_notice(Some(&Repair::BackedUp(bak)));
        assert_eq!(saved.title, "config.json tidak valid");
        assert_eq!(saved.body, "Disalin ke config.json.bak — cek lagi isinya");
        let blocked = invalid_config_notice(Some(&Repair::Blocked));
        assert_eq!(blocked.title, "config.json tidak valid");
        assert_eq!(blocked.body, "Tidak bisa disalin — cek lagi isinya");
        let connect = invalid_config_notice(None);
        assert_eq!(connect.title, "config.json tidak valid");
        assert_eq!(connect.kind, NoticeKind::App { app_id: None });
    }

    #[test]
    fn island_notices_in_english() {
        i18n::set_lang(i18n::Lang::En);
        let bak = PathBuf::from(r"C:\x\DynamicIsland\config.json.bak");
        let saved = invalid_config_notice(Some(&Repair::BackedUp(bak)));
        assert_eq!(saved.title, "config.json is invalid");
        assert_eq!(saved.body, "Copied to config.json.bak — check its contents");
        let n = client_id_notice();
        assert_eq!(
            (n.source.as_str(), n.title.as_str()),
            ("Calendar", "Add a Google client ID first")
        );
        assert_eq!(dnd_notice().title, "Turn off Windows pop-ups");
        assert_eq!(
            hook_notice(InstallOutcome::Installed).title,
            "Claude Code hooks installed"
        );
    }

    #[test]
    fn client_id_notice_points_to_the_readme() {
        let n = client_id_notice();
        assert_eq!(n.source, "Kalender");
        assert_eq!(n.title, "Isi Google client ID dulu");
        assert_eq!(n.body, "lihat README");
        assert_eq!(n.kind, NoticeKind::App { app_id: None });
    }

    #[test]
    fn anim_midway_fades_linearly() {
        let t0 = Instant::now();
        let anim = morph(t0);
        let (_, opacity) = anim.frame(t0 + Duration::from_millis(ANIM_MS / 2));
        assert!((opacity - 0.5).abs() < 0.01, "opacity {opacity}");
    }
}
