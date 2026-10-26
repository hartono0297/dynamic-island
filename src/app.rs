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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM,
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
    WM_SETTINGCHANGE, WM_TIMER,
};
use windows::core::{PCWSTR, w};

use crate::calendar::sync::{SyncHandle, SyncRequest, post_status, save_failed_notice, spawn_sync};
use crate::calendar::{CalEvent, CalStatus, CalendarSnapshot, draft_from, new_draft};
use crate::claude::pipe_server::spawn_pipe_server;
use crate::cli::{self, Builtin, CliEvent, History, RunHandle, Session, SyncStep, WindowSync};
use crate::config::{self, Config, OnDisk, Repair, Style};
use crate::i18n::{self, Key, tr};
use crate::lyrics::{self, DiskCache, LyricsState, LyricsWorker, Query};
use crate::model::{Effect, Event, Hit, Notice, NoticeKind, Page, View};
use crate::notes::NotesStore;
use crate::pomodoro::PomoStatus;
use crate::reminders::{self, Reminder, RemindersStore, due_now, next_due};
use crate::shortcuts;
use crate::sources::battery::{BatteryMonitor, battery_notice, read_battery};
use crate::sources::fullscreen::is_fullscreen;
use crate::sources::{media, notifications};
use crate::state::IslandState;
use crate::theme::{self, Theme};
use crate::ui::cli_window::{self, CliAction, CliInit, WM_APP_CLI_ACTION};
use crate::ui::editor::{self, EditorKind, EditorResult, WM_APP_EDITOR_RESULT};
use crate::ui::frame_clock::{FrameClock, WM_APP_FRAME};
use crate::ui::geometry::{ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, shape_for};
use crate::ui::render::{CliCard, Frame, Renderer};
use crate::ui::settings::{self, Action as SettingsAction, WM_APP_SETTINGS_ACTION};
use crate::ui::tray::{self, MenuCmd, WM_APP_TRAY};
use crate::ui::window::IslandWindow;

/// Posted by background sources after queueing an event.
const WM_APP_EVENT: u32 = WM_APP + 1;
/// Posted once the user has answered the notification-access prompt.
const WM_APP_ACCESS: u32 = WM_APP + 3;
/// These live in the Controls feature of `windows`; the values are fixed by Win32.
const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The system clock or time zone changed.
const WM_TIMECHANGE: u32 = 0x001E;
const WM_POWERBROADCAST: u32 = 0x0218;
/// `WM_POWERBROADCAST` codes for waking from sleep (with or without a user).
const PBT_APMRESUMESUSPEND: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
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
/// Reminders: one-shot at the earliest due time (at most `REMINDER_MAX_MS` away).
const TIMER_REMINDER: usize = 9;
/// Idle clock redraws at each minute boundary, only while it shows.
const TIMER_CLOCK: usize = 8;
/// One-shot to the next lyric line, only while the lyric view shows.
const TIMER_LYRIC: usize = 10;
/// One-shot: a clicked shortcut runs once the panel is out of the way.
const TIMER_SHORTCUT: usize = 11;
const SYSTEM_MS: u32 = 1000;
const BATTERY_MS: u32 = 30_000;
/// Values table: ~30 fps equalizer.
const EQ_MS: u32 = 33;
const DAY_MS: u32 = 60_000;
/// The reminder timer re-arms at least this often, so a changed clock or a
/// sleep is noticed without polling.
const REMINDER_MAX_MS: i64 = 3_600_000;
/// A reminder fired this much after its time (sleep, clock change) says "Terlewat".
const REMINDER_LATE_SECS: i64 = 60;
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
    static DEFERRED_SETTINGS: RefCell<Vec<SettingsAction>> = const { RefCell::new(Vec::new()) };
    static DEFERRED_CLI: RefCell<Vec<CliAction>> = const { RefCell::new(Vec::new()) };
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
    theme::set_system_light(theme::read_system_light());
    theme::set_current(Theme::of(&config));
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
    let reminders = RemindersStore::open(RemindersStore::default_path());
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
            reminders,
            reminder_fired: Vec::new(),
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
            clock_timer: false,
            lyrics: LyricsState::default(),
            lyrics_worker: None,
            lyric_timer: false,
            lyric_fading: false,
            config_warned: false,
            cli: Session::default(),
            cli_run: None,
            cli_runs: 0,
            cli_history: History::load(&cli::history_path()),
            cli_cwd: cli::load_cwd(&cli::cwd_path()),
            cli_shown: WindowSync::default(),
            cli_wake: Arc::new(AtomicBool::new(false)),
            shortcut: None,
        });
    });
    with_app(|app| {
        let now = Instant::now();
        let lyrics_on = app.config.lyrics;
        app.apply(Event::LyricsSetting(lyrics_on), now);
        app.check_fullscreen();
        app.check_battery();
        app.draw(now);
        app.request_calendar(app.state.cal_range(), now);
        // An empty notes list would otherwise look like the notes were deleted.
        if let Some(notice) = notes_unreadable_notice(&app.notes) {
            app.apply_now(Event::Notice(notice));
        }
        if let Some(notice) = reminders_unreadable_notice(&app.reminders) {
            app.apply_now(Event::Notice(notice));
        }
        // Reminders that came due while the island was off ring now.
        app.fire_reminders(true);
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
    reminders: RemindersStore,
    /// Due reminders whose notice was raised and not yet dismissed; the timer
    /// skips them.
    reminder_fired: Vec<String>,
    /// `None` once shut down.
    sync: Option<SyncHandle>,
    /// The range last asked of the sync thread, and when.
    calendar_asked: Option<((NaiveDate, NaiveDate), Instant)>,
    /// The open note, event or reminder editor (at most one).
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
    /// `TIMER_CLOCK` is set.
    clock_timer: bool,
    /// Lyrics looked up, the current track's lines and the line on screen.
    lyrics: LyricsState,
    /// The lyrics thread, started on the first lookup.
    lyrics_worker: Option<LyricsWorker>,
    /// `TIMER_LYRIC` is set.
    lyric_timer: bool,
    /// Two lyric lines are crossfading; the frame clock runs until it ends.
    lyric_fading: bool,
    /// The invalid-config notice was shown this run.
    config_warned: bool,
    /// The CLI's output, last command and running state.
    cli: Session,
    /// The running command, if any.
    cli_run: Option<RunHandle>,
    /// Run ids given out so far.
    cli_runs: u64,
    cli_history: History,
    /// Where commands run; `cd` changes it.
    cli_cwd: PathBuf,
    /// What the CLI window has shown of the output.
    cli_shown: WindowSync,
    /// Set by a run's threads when they post a wake, cleared when `drain`
    /// reads the queue: a burst of output posts one message, not hundreds.
    cli_wake: Arc<AtomicBool>,
    /// A clicked shortcut waiting for `TIMER_SHORTCUT`.
    shortcut: Option<shortcuts::Action>,
}

impl App {
    fn drain(&mut self) {
        let now = Instant::now();
        // Before reading: output queued after this posts a new wake.
        self.cli_wake.store(false, Ordering::SeqCst);
        while let Ok(event) = self.rx.try_recv() {
            self.apply(event, now);
        }
        // Once per burst of command output, not per chunk.
        self.sync_cli_window();
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
                // Lyrics are `App`'s too; `refresh` tells the reducer they exist.
                Event::Lyrics(found) => {
                    self.lyrics.found(found);
                    continue;
                }
                Event::Cli(event) => {
                    self.on_cli_event(event);
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
            Effect::OpenReminderEditor(id) => {
                let now = Local::now().naive_local();
                return self.open_editor(reminder_editor_kind(&self.reminders, id.as_deref(), now));
            }
            Effect::SaveLyrics(on) => {
                self.config.lyrics = on;
                self.save_config();
            }
            Effect::RunShortcut(s) => {
                let action = shortcuts::shortcut_action(s);
                self.shortcut = Some(action);
                // Replaces a pending one: only the last click runs.
                unsafe {
                    SetTimer(
                        Some(self.hwnd),
                        TIMER_SHORTCUT,
                        shortcuts::delay_ms(action),
                        None,
                    );
                }
            }
            Effect::OpenCli => self.open_cli(),
            Effect::ReminderDone(id) => {
                // The reducer already took its notice down.
                self.reminder_fired.retain(|f| *f != id);
                if self
                    .reminders
                    .mark_done_or_advance(&id, Local::now().timestamp())
                {
                    self.persist_reminders(now);
                }
                self.arm_reminders();
            }
        }
        None
    }

    /// Raises a notice (and one sound) for every reminder whose time has come
    /// and that is not showing yet, then re-arms the timer. At startup they all
    /// say "Terlewat"; later only those fired well after their time (sleep).
    fn fire_reminders(&mut self, startup: bool) {
        let now = Instant::now();
        let unix = Local::now().timestamp();
        let mut rang = false;
        for id in due_now(self.reminders.list(), unix) {
            if self.reminder_fired.contains(&id) {
                continue;
            }
            let Some(notice) = self
                .reminders
                .get(&id)
                .map(|r| reminder_notice(r, unix, startup))
            else {
                continue;
            };
            self.reminder_fired.push(id);
            self.apply(Event::Notice(notice), now);
            rang = true;
        }
        if rang {
            play_phase_sound();
            self.refresh(now);
        }
        self.arm_reminders();
    }

    /// One-shot to the earliest reminder not showing yet, capped at
    /// `REMINDER_MAX_MS` so a clock change or sleep is caught within the hour.
    fn arm_reminders(&self) {
        let pending: Vec<Reminder> = self
            .reminders
            .upcoming()
            .iter()
            .filter(|r| !self.reminder_fired.contains(&r.id))
            .cloned()
            .collect();
        let wait = reminder_wait_ms(next_due(&pending), Local::now().timestamp_millis());
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REMINDER);
            if let Some(ms) = wait {
                SetTimer(Some(self.hwnd), TIMER_REMINDER, ms, None);
            }
        }
    }

    /// After an edit or delete: notices of reminders that are no longer due
    /// come down, and the timer follows the new times.
    fn settle_reminders(&mut self, now: Instant) {
        let due = due_now(self.reminders.list(), Local::now().timestamp());
        let (keep, gone): (Vec<String>, Vec<String>) = std::mem::take(&mut self.reminder_fired)
            .into_iter()
            .partition(|id| due.contains(id));
        self.reminder_fired = keep;
        for id in gone {
            self.apply(Event::ReminderCleared(id), now);
        }
        self.arm_reminders();
    }

    fn persist_reminders(&mut self, now: Instant) {
        if let Err(error) = self.reminders.save() {
            let notice = island_notice(
                "reminders",
                tr(Key::RemindersSaveFailed),
                &error.to_string(),
            );
            self.apply(Event::Notice(notice), now);
        }
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
        let anchor = self.under_panel()?;
        // One window under the panel: the CLI window has nothing to lose.
        cli_window::close();
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

    /// Screen point under the panel's bottom centre, where editors and the
    /// CLI window hang.
    fn under_panel(&self) -> Option<POINT> {
        let win = self.win.as_ref()?;
        let panel = shape_for(self.config.style, ShapeKind::Panel);
        Some(win.to_screen(panel.x + panel.w / 2.0, panel.y + panel.h))
    }

    /// Opens the CLI window with everything so far, or brings it forward.
    /// An open editor comes forward instead: it may hold unsaved text.
    fn open_cli(&mut self) {
        if editor::focus_open_editor() {
            return;
        }
        let Some(anchor) = self.under_panel() else {
            return;
        };
        let init = CliInit {
            text: self.cli.output.text(),
            cwd: self.cli_cwd.to_string_lossy().into_owned(),
            shell: self.config.cli_shell,
            running: self.cli.running().is_some(),
            history: self.cli_history.clone(),
        };
        let fresh = !cli_window::is_open();
        match cli_window::open(self.hwnd, anchor, init) {
            Ok(_) if fresh => self.cli_shown.reset(&self.cli.output),
            Ok(_) => {}
            Err(error) => eprintln!("dynamic-island: cannot open the CLI: {error}"),
        }
    }

    fn on_cli_action(&mut self, action: CliAction) {
        match action {
            CliAction::Run(cmd) => self.run_cli(&cmd),
            CliAction::Stop => {
                if let Some(run) = &self.cli_run {
                    run.kill();
                }
            }
            CliAction::Focus(on) => self.apply_now(Event::CliFocus(on)),
            CliAction::Closed => self.apply_now(Event::CliFocus(false)),
        }
    }

    /// A command from the CLI window: a builtin, or a run in the shell.
    fn run_cli(&mut self, cmd: &str) {
        self.cli_history.push(cmd);
        // A history that cannot be written only loses the history.
        let _ = self.cli_history.save(&cli::history_path());
        // While a command runs, `Session` keeps all of this off the card.
        match cli::builtin(cmd) {
            Some(Builtin::Clear) => {
                self.cli.clear();
                self.cli_shown.reset(&self.cli.output);
                cli_window::reset("");
            }
            Some(Builtin::Cd(arg)) => {
                self.cli.begin(cmd);
                match cli::resolve_cd(&self.cli_cwd, arg.as_deref(), &cli::home()) {
                    Ok(dir) => {
                        let _ = cli::save_cwd(&cli::cwd_path(), &dir);
                        self.cli_cwd = dir;
                    }
                    Err(arg) => self.cli.note(cli::cd_error(&arg)),
                }
            }
            None if self.cli.running().is_some() => {
                self.cli.begin(cmd);
                self.cli.note(tr(Key::CliBusy).into());
            }
            None => {
                self.cli.begin(cmd);
                self.cli_runs += 1;
                let id = self.cli_runs;
                let shell = self.config.cli_shell;
                match cli::spawn(id, cmd, &self.cli_cwd, shell, self.cli_sink()) {
                    Ok(run) => {
                        self.cli.started(id);
                        self.cli_run = Some(run);
                    }
                    Err(error) => {
                        let line = i18n::cli_start_failed(i18n::lang(), &error.to_string());
                        self.cli.note(line);
                    }
                }
            }
        }
        self.sync_cli_window();
        self.refresh(Instant::now());
    }

    /// Where a run's threads send their output: the island's queue, waking
    /// it once per burst.
    fn cli_sink(&self) -> cli::Sink {
        let tx = self.tx.clone();
        let wake = poster(self.hwnd, WM_APP_EVENT);
        let pending = self.cli_wake.clone();
        Arc::new(move |event| {
            if tx.send(Event::Cli(event)).is_ok() && !pending.swap(true, Ordering::SeqCst) {
                wake();
            }
        })
    }

    fn on_cli_event(&mut self, event: CliEvent) {
        match event {
            CliEvent::Output { run, lines } => {
                self.cli.lines(run, lines);
            }
            CliEvent::Exit { run, code, killed } => {
                if self.cli.exit(run, code, killed) {
                    // The command is over; what it started may go on.
                    self.cli_run = None;
                }
            }
        }
    }

    /// Brings an open CLI window up to date with the output and status.
    fn sync_cli_window(&mut self) {
        if !cli_window::is_open() {
            return;
        }
        match self.cli_shown.next(&self.cli.output) {
            SyncStep::Nothing => {}
            SyncStep::Append(lines) => cli_window::append(&lines),
            SyncStep::Reset => cli_window::reset(&self.cli.output.text()),
        }
        cli_window::set_status(
            &self.cli_cwd.to_string_lossy(),
            self.config.cli_shell,
            self.cli.running().is_some(),
        );
    }

    /// The shortcut `TIMER_SHORTCUT` was waiting for, taken once.
    fn take_shortcut(&mut self) -> Option<shortcuts::Action> {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SHORTCUT);
        }
        self.shortcut.take()
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
            EditorResult::ReminderSaved {
                id,
                text,
                date,
                time,
                repeat,
            } => {
                let unix = Local::now().timestamp();
                // Keeps the time as set, even on a day a DST change skips it.
                if self
                    .reminders
                    .upsert_local(id.as_deref(), &text, date, time, repeat, unix)
                    .is_some()
                {
                    self.persist_reminders(now);
                    self.settle_reminders(now);
                }
            }
            EditorResult::ReminderDeleted { id } => {
                if self.reminders.delete(&id) {
                    self.persist_reminders(now);
                    self.settle_reminders(now);
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
            // A new day may follow a clock or time-zone change.
            self.fire_reminders(false);
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
        self.sync_lyrics(now);
        self.sync_lyric_line(now);
        let view = self.state.view();
        let kind = match &view {
            View::Hidden => None,
            View::Idle => Some(ShapeKind::Idle),
            View::MediaCompact => Some(ShapeKind::MediaCompact),
            View::MediaLyric => Some(ShapeKind::MediaLyric),
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
        self.sync_settings();
    }

    fn animating(&self) -> bool {
        self.anim.running || self.slide.is_some() || self.lyric_fading
    }

    /// Looks up lyrics for the playing track (memory first, else the worker;
    /// only with the setting on and while it plays) and tells the reducer
    /// whether the current track has them.
    fn sync_lyrics(&mut self, now: Instant) {
        let on = self.state.lyrics_on();
        let query = lyrics::track_query(self.state.media(), self.state.timeline());
        if on {
            let playing = self.state.media().is_some_and(|m| m.playing);
            if let Some(q) = self.lyrics.want(query.as_ref(), playing) {
                self.ask_lyrics(q);
            }
        }
        let ready = on && self.lyrics.ready(query.as_ref());
        if ready != self.state.lyrics_ready() {
            self.apply(Event::LyricsReady(ready), now);
        }
    }

    fn ask_lyrics(&mut self, q: Query) {
        let worker = self.lyrics_worker.get_or_insert_with(|| {
            lyrics::spawn_worker(
                self.tx.clone(),
                poster(self.hwnd, WM_APP_EVENT),
                DiskCache::new(DiskCache::default_dir()),
            )
        });
        if !worker.ask(q) {
            // The thread is gone; the next track starts another.
            self.lyrics_worker = None;
        }
    }

    /// Puts the line for the current position on screen (crossfading from
    /// the last one) and arms `TIMER_LYRIC` for the next change. Outside
    /// the lyric view the timer stops. True when the line changed.
    fn sync_lyric_line(&mut self, now: Instant) -> bool {
        let view = self.state.view();
        let position = match (self.state.timeline(), self.state.media()) {
            (Some(t), Some(m)) if view == View::MediaLyric => {
                Some(lyrics::position_at(t, m.playing, now))
            }
            _ => None,
        };
        let Some(pos) = position else {
            // Fading out (fullscreen) keeps its last line.
            if view != View::Hidden {
                self.lyrics.hide();
            }
            self.set_lyric_timer(None);
            return false;
        };
        let changed = self.lyrics.advance(pos, now);
        if self.lyrics.fading(now) {
            self.lyric_fading = true;
            self.ensure_clock();
        }
        self.set_lyric_timer(self.lyrics.wait_ms(pos));
        changed
    }

    /// One-shot: `SetTimer` on the same id replaces the pending one.
    fn set_lyric_timer(&mut self, wait_ms: Option<u64>) {
        unsafe {
            match wait_ms {
                Some(ms) => {
                    let ms = u32::try_from(ms).unwrap_or(u32::MAX);
                    SetTimer(Some(self.hwnd), TIMER_LYRIC, ms, None);
                }
                None if self.lyric_timer => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_LYRIC);
                }
                None => {}
            }
        }
        self.lyric_timer = wait_ms.is_some();
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
        let clock = clock_timer_wanted(&view, self.config.idle_clock);
        if clock != self.clock_timer {
            self.clock_timer = clock;
            if clock {
                self.arm_clock();
            } else {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_CLOCK);
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

    /// The wall clock jumped (time or zone changed, or the PC woke up): due
    /// reminders ring and the reminder timer re-arms for the new time; a
    /// showing clock redraws now and re-aligns to the new minute boundary.
    fn on_clock_jump(&mut self) {
        self.fire_reminders(false);
        if !self.clock_timer {
            return;
        }
        if !self.animating() {
            self.draw(Instant::now());
        }
        self.arm_clock();
    }

    /// One-shot to just past the next minute boundary, re-armed after each redraw.
    fn arm_clock(&self) {
        let wait = ms_to_next_minute(Local::now().time());
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_CLOCK, wait, None);
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
                // A CLI window that died without `Closed` must not pin the panel.
                if self.state.cli_focused() && !cli_window::is_open() {
                    self.apply_now(Event::CliFocus(false));
                }
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
            TIMER_REMINDER => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_REMINDER);
                }
                // Also re-arms: the next one, or another hour's wait.
                self.fire_reminders(false);
            }
            TIMER_LYRIC => {
                let now = Instant::now();
                // A crossfade starts the frame clock, which draws it.
                if self.sync_lyric_line(now) && !self.animating() {
                    self.draw(now);
                }
            }
            TIMER_CLOCK => {
                if !self.animating() {
                    self.draw(Instant::now());
                }
                if self.clock_timer {
                    self.arm_clock();
                }
            }
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
        // Checked at the drawn frame's time, so its last frame was the settled one.
        if self.lyric_fading && !self.lyrics.fading(now) {
            self.lyric_fading = false;
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

    /// `save_config`, also writing the fields `edit` sets: ones the owner may
    /// edit by hand (Google client, battery threshold) but just set in the
    /// settings window. Other hand edits in the file stay.
    fn save_config_with(&mut self, edit: impl FnOnce(&mut Config)) {
        let out = config::save_merged_with(&self.config, edit);
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
        self.apply_theme();
    }

    /// Puts a changed accent, surface or window mode into effect: the island
    /// redraws and an open editor restyles. Compared with the theme in effect
    /// rather than the old config, so a caller may set the fields first.
    fn apply_theme(&mut self) {
        if theme::set_current(Theme::of(&self.config)) {
            editor::restyle_open_editor();
            cli_window::restyle();
            settings::restyle();
            self.draw(Instant::now());
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

    fn set_autostart(&mut self, enabled: bool) {
        if tray::set_autostart(enabled).is_ok() {
            self.config.start_with_windows = enabled;
            self.save_config();
        }
    }

    /// What the settings window shows.
    fn settings_model(&self) -> settings::Model {
        let status = self.state.pomodoro().status();
        settings::Model {
            pomodoro: self.state.pomodoro().settings(),
            pomodoro_locked: !matches!(status, PomoStatus::Idle | PomoStatus::Ready),
            calendar: self.calendar.status,
            ..settings::Model::from_config(&self.config)
        }
    }

    /// Brings an open settings window up to date; it redraws only on a change.
    fn sync_settings(&self) {
        if settings::is_open() {
            settings::update(self.settings_model());
        }
    }

    /// A change made in the settings window: applied now and saved.
    fn on_settings_action(&mut self, action: SettingsAction) {
        let now = Instant::now();
        match action {
            SettingsAction::Style(style) => self.set_style(style),
            SettingsAction::Accent(accent) => {
                self.config.accent = accent;
                self.save_config();
            }
            SettingsAction::Surface(surface) => {
                self.config.surface = surface;
                self.save_config();
            }
            SettingsAction::WindowMode(mode) => {
                self.config.window_theme = mode;
                self.save_config();
            }
            SettingsAction::Lang(lang) => {
                self.config.language = lang;
                // `adopt_config` switches the strings; everything showing text relabels.
                self.save_config();
                editor::relabel_open_editor();
                cli_window::relabel();
                self.draw(now);
            }
            SettingsAction::Autostart(on) => self.set_autostart(on),
            SettingsAction::IdleClock(on) => {
                self.config.idle_clock = on;
                self.save_config();
                // Redraws the idle island and starts or stops its timer.
                self.refresh(now);
            }
            SettingsAction::Lyrics(on) => {
                self.config.lyrics = on;
                self.save_config();
                // Looks the track up and switches the view now.
                self.apply_now(Event::LyricsSetting(on));
            }
            SettingsAction::Shell(shell) => {
                // The next command runs in it; one running goes on as it is.
                self.config.cli_shell = shell;
                self.save_config();
                self.sync_cli_window();
                self.draw(now);
            }
            SettingsAction::BatteryThreshold(threshold) => {
                self.save_config_with(|c| c.battery_threshold = threshold);
                self.battery.set_threshold(self.config.battery_threshold);
            }
            SettingsAction::Pomo(field, dir) => {
                // Through the reducer like a click: state, then config.
                self.apply_now(Event::Click(Hit::PomoAdjust(field, dir)));
            }
            SettingsAction::SaveGoogle { id, secret } => {
                self.save_config_with(|c| {
                    c.google_client_id = id;
                    c.google_client_secret = secret;
                });
            }
            SettingsAction::ConnectGoogle => {
                if let Some(event) = self.connect_google() {
                    self.apply_now(event);
                }
            }
            SettingsAction::DisconnectGoogle => self.disconnect_google(),
            // Handled outside the app borrow (`deliver_settings_action`).
            SettingsAction::DndHelp | SettingsAction::Closed => {}
        }
        self.sync_settings();
    }

    /// Forgets the Google login; the calendar asks to connect again.
    fn disconnect_google(&mut self) {
        if let Some(sync) = &self.sync {
            sync.send(SyncRequest::Disconnect);
        }
        if self.calendar.status != CalStatus::NotConfigured {
            self.calendar.status = CalStatus::NeedsLogin;
        }
        self.calendar_asked = None;
        self.refresh(Instant::now());
    }

    fn finish_onboarding(&mut self) {
        self.config.onboarding_done = true;
        self.save_config();
    }

    fn draw(&mut self, now: Instant) {
        let Some(win) = &self.win else { return };
        let (shape, opacity) = self.anim.frame(now);
        let state = &self.state;
        let cwd = self.cli_cwd.to_string_lossy();
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
            reminders: self.reminders.upcoming(),
            reminders_page: state.reminders_page(),
            notes_page: state.notes_page(),
            day_page: state.day_page(),
            page_slide: self.slide.map_or(0.0, |s| s.offset(now)),
            palette: theme::island_palette(&self.config),
            clock: (self.config.idle_clock && self.drawn == View::Idle)
                .then(|| Local::now().format("%H:%M").to_string()),
            lyric: self.lyrics.shown(now),
            lyrics_on: state.lyrics_on(),
            cli: CliCard {
                command: self.cli.last_command(),
                tail: self.cli.tail().collect(),
                running: self.cli.running().is_some(),
                shell: self.config.cli_shell,
                cwd: &cwd,
            },
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

/// Messages after which the wall clock may have jumped.
fn is_clock_jump(msg: u32, wparam: WPARAM) -> bool {
    match msg {
        WM_TIMECHANGE => true,
        WM_POWERBROADCAST => matches!(wparam.0, PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC),
        _ => false,
    }
}

/// The idle clock's minute timer runs only while the idle island shows it.
fn clock_timer_wanted(view: &View, idle_clock: bool) -> bool {
    idle_clock && *view == View::Idle
}

/// Milliseconds from `now` until the shown minute changes, plus 1 ms so the
/// redraw lands after the change. A timer that fires early just redraws the
/// same minute and re-arms for the few milliseconds left.
fn ms_to_next_minute(now: NaiveTime) -> u32 {
    // A leap second reports 1_000_000_000 ns and more; it counts as the 59th.
    let ms = now.second() * 1000 + (now.nanosecond() / 1_000_000).min(999);
    60_000 - ms + 1
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

/// At startup: reminders.json exists but could not be read or parsed.
fn reminders_unreadable_notice(reminders: &RemindersStore) -> Option<Notice> {
    reminders.load_failed().then(|| {
        island_notice(
            "reminders-unreadable",
            tr(Key::RemindersUnreadable),
            tr(Key::RemindersUnreadableBody),
        )
    })
}

/// The sticky notice for a reminder whose time came: its text over the time
/// ("09.30", or "26 Okt 09.30" on another day), marked "Terlewat · " when it
/// is announced late: at startup, or over a minute after its time.
fn reminder_notice(r: &Reminder, now: i64, startup: bool) -> Notice {
    let lang = i18n::lang();
    let when = match (
        reminders::local_time(r.due_unix),
        reminders::local_time(now),
    ) {
        (Some(due), Some(today)) if due.date() == today.date() => i18n::time_hm(lang, due),
        (Some(due), _) => format!(
            "{} {}",
            i18n::short_date(lang, due.date()),
            i18n::time_hm(lang, due)
        ),
        (None, _) => String::new(),
    };
    let late = startup || now - r.due_unix > REMINDER_LATE_SECS;
    let body = if late {
        format!("{} · {when}", i18n::text(lang, Key::Missed))
    } else {
        when
    };
    Notice {
        key: format!("reminder-{}", r.id),
        source: tr(Key::SourceReminder).into(),
        title: r.text.clone(),
        body,
        kind: NoticeKind::Reminder { id: r.id.clone() },
    }
}

/// Milliseconds until the reminder due at `next` (Unix seconds), plus 1 ms
/// so it is due when the timer fires; never more than `REMINDER_MAX_MS`.
/// `None` when nothing is pending.
fn reminder_wait_ms(next: Option<i64>, now_ms: i64) -> Option<u32> {
    let due_ms = next?.saturating_mul(1000);
    let wait = due_ms.saturating_sub(now_ms).max(0).saturating_add(1);
    Some(wait.min(REMINDER_MAX_MS) as u32)
}

/// The reminder editor for `id`; an unknown id (deleted meanwhile) or `None`
/// opens a new one-off reminder at the next whole hour after `now`.
fn reminder_editor_kind(
    reminders: &RemindersStore,
    id: Option<&str>,
    now: NaiveDateTime,
) -> EditorKind {
    let existing = id.and_then(|id| reminders.get(id));
    if let Some(r) = existing
        && let Some(due) = reminders::local_time(r.due_unix)
    {
        return EditorKind::Reminder {
            id: Some(r.id.clone()),
            text: r.text.clone(),
            date: due.date(),
            // The time as set: on a day DST skips it, `due_unix` shows another.
            time: reminders.wall_time(&r.id).unwrap_or(due.time()),
            repeat: r.repeat,
        };
    }
    let next_hour =
        now.date().and_hms_opt(now.hour(), 0, 0).unwrap_or(now) + chrono::Duration::hours(1);
    EditorKind::Reminder {
        id: None,
        text: String::new(),
        date: next_hour.date(),
        time: next_hour.time(),
        repeat: reminders::Repeat::Once,
    }
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
        Some(MenuCmd::Settings) => open_settings(hwnd),
        Some(MenuCmd::StylePill) => {
            with_app(|app| app.set_style(Style::Pill));
        }
        Some(MenuCmd::StyleNotch) => {
            with_app(|app| app.set_style(Style::Notch));
        }
        Some(MenuCmd::Exit) => shutdown(hwnd),
        None => {}
    }
}

/// Opens the settings window, or brings the open one forward. Outside
/// `with_app`: creating and focusing a window sends messages.
fn open_settings(hwnd: HWND) {
    let Some(model) = with_app(|app| app.settings_model()) else {
        return;
    };
    if let Err(error) = settings::open(hwnd, model, None) {
        eprintln!("dynamic-island: cannot open settings: {error}");
    }
}

/// Runs a settings action now, or once the handler holding the app returns.
/// The Do Not Disturb help opens the Settings app, which may pump messages,
/// so it never runs inside `with_app`.
fn deliver_settings_action(action: SettingsAction) {
    if action == SettingsAction::DndHelp {
        show_dnd_help();
        return;
    }
    let mut pending = Some(action);
    with_app(|app| {
        if let Some(action) = pending.take() {
            app.on_settings_action(action);
        }
    });
    if let Some(action) = pending {
        DEFERRED_SETTINGS.with_borrow_mut(|queue| queue.push(action));
    }
}

/// Closes the island. An open editor is closed first and its last result
/// (an edited note) handled here: once the island is gone, nobody reads it.
fn shutdown(hwnd: HWND) {
    settings::close();
    flush_editor();
    // A command still running ends with the island's process, which closes
    // its job.
    cli_window::close();
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
    for action in DEFERRED_SETTINGS.take() {
        deliver_settings_action(action);
    }
    for action in DEFERRED_CLI.take() {
        deliver_cli_action(action);
    }
}

/// Handles a CLI window action now, or once the handler holding the app returns.
fn deliver_cli_action(action: CliAction) {
    let mut pending = Some(action);
    with_app(|app| {
        if let Some(action) = pending.take() {
            app.on_cli_action(action);
        }
    });
    if let Some(action) = pending {
        DEFERRED_CLI.with_borrow_mut(|queue| queue.push(action));
    }
}

/// `TIMER_SHORTCUT`: runs the waiting shortcut outside the app borrow (the
/// shell may pump messages). If the app is busy, the timer fires again.
fn run_shortcut() {
    if let Some(action) = with_app(App::take_shortcut).flatten() {
        shortcuts::run(action);
    }
}

/// `WM_SETTINGCHANGE` for "ImmersiveColorSet": the Windows app mode (light or
/// dark) or the accent changed.
fn is_color_set_change(lparam: LPARAM) -> bool {
    let area = PCWSTR(lparam.0 as *const u16);
    !area.is_null() && unsafe { area.to_string() }.is_ok_and(|a| a == "ImmersiveColorSet")
}

/// Windows switched between light and dark apps: windows following it restyle.
/// The island itself is dark either way.
fn on_system_theme_changed() {
    let changed = theme::set_system_light(theme::read_system_light());
    if changed && theme::current().window == theme::WindowTheme::System {
        editor::restyle_open_editor();
        cli_window::restyle();
        settings::restyle();
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
        WM_TIMER if wparam.0 == TIMER_SHORTCUT => {
            run_shortcut();
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_TIMECHANGE | WM_POWERBROADCAST => {
            if is_clock_jump(msg, wparam) {
                with_app(App::on_clock_jump);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
        WM_APP_SETTINGS_ACTION => {
            // Each action is taken exactly once, here.
            if let Some(action) = unsafe { settings::take_action(lparam) } {
                deliver_settings_action(action);
            }
            LRESULT(0)
        }
        WM_APP_EDITOR_RESULT => {
            // Each result is taken exactly once, here.
            if let Some(result) = unsafe { editor::take_result(lparam) } {
                deliver_result(result);
            }
            LRESULT(0)
        }
        WM_APP_CLI_ACTION => {
            // Each action is taken exactly once, here.
            if let Some(action) = unsafe { cli_window::take_action(lparam) } {
                deliver_cli_action(action);
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
        WM_SETTINGCHANGE => {
            if is_color_set_change(lparam) {
                on_system_theme_changed();
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
        for notice in [dnd_notice(), invalid_config_notice(None)] {
            assert_eq!(notice.source, "Dynamic Island");
            assert_eq!(notice.kind, NoticeKind::App { app_id: None });
        }
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
    fn clock_redraws_just_after_the_minute_changes() {
        let at = |h, m, s, ms| NaiveTime::from_hms_milli_opt(h, m, s, ms).unwrap();
        assert_eq!(ms_to_next_minute(at(9, 41, 0, 0)), 60_001);
        assert_eq!(ms_to_next_minute(at(9, 41, 30, 250)), 29_751);
        assert_eq!(ms_to_next_minute(at(23, 59, 59, 999)), 2);
        // A leap second (1_000+ ms) waits like the last millisecond, never 0.
        assert_eq!(ms_to_next_minute(at(23, 59, 59, 1_500)), 2);
    }

    #[test]
    fn time_changes_and_wake_ups_are_clock_jumps() {
        assert!(is_clock_jump(WM_TIMECHANGE, WPARAM(0)));
        assert!(is_clock_jump(
            WM_POWERBROADCAST,
            WPARAM(PBT_APMRESUMEAUTOMATIC)
        ));
        assert!(is_clock_jump(
            WM_POWERBROADCAST,
            WPARAM(PBT_APMRESUMESUSPEND)
        ));
        // Going to sleep (PBT_APMSUSPEND) or a power-source change is not.
        assert!(!is_clock_jump(WM_POWERBROADCAST, WPARAM(0x0004)));
        assert!(!is_clock_jump(WM_POWERBROADCAST, WPARAM(0x000A)));
        assert!(!is_clock_jump(WM_TIMER, WPARAM(0)));
    }

    #[test]
    fn clock_timer_runs_only_while_the_idle_clock_shows() {
        assert!(clock_timer_wanted(&View::Idle, true));
        assert!(!clock_timer_wanted(&View::Idle, false), "setting off");
        let others = [
            View::Hidden,
            View::MediaCompact,
            View::PomodoroCompact,
            View::Panel(Page::Calendar),
            View::Notice(island_notice("t", "title", "body")),
        ];
        for view in others {
            assert!(!clock_timer_wanted(&view, true), "{view:?}");
        }
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

    fn local_unix(d: NaiveDate, h: u32, m: u32) -> i64 {
        reminders::local_unix(d, NaiveTime::from_hms_opt(h, m, 0).unwrap()).unwrap()
    }

    fn due_reminder(due: i64) -> Reminder {
        Reminder {
            id: "r1-0".into(),
            text: "Minum obat".into(),
            due_unix: due,
            repeat: reminders::Repeat::Daily,
            done: false,
        }
    }

    #[test]
    fn reminder_notice_on_time_late_and_missed() {
        let due = local_unix(date(26), 8, 30);
        let on_time = reminder_notice(&due_reminder(due), due + 1, false);
        assert_eq!(on_time.title, "Minum obat");
        assert_eq!(on_time.body, "08.30");
        assert_eq!(on_time.source, "Pengingat");
        assert_eq!(on_time.key, "reminder-r1-0");
        assert_eq!(on_time.kind, NoticeKind::Reminder { id: "r1-0".into() });
        // Woken from sleep ten minutes late.
        let late = reminder_notice(&due_reminder(due), due + 600, false);
        assert_eq!(late.body, "Terlewat · 08.30");
        // At startup, even a just-passed one was missed while off.
        let missed = reminder_notice(&due_reminder(due), due + 5, true);
        assert_eq!(missed.body, "Terlewat · 08.30");
        // From another day: with its date.
        let next_day = local_unix(date(27), 9, 0);
        let old = reminder_notice(&due_reminder(due), next_day, true);
        assert_eq!(old.body, "Terlewat · 26 Okt 08.30");
    }

    #[test]
    fn reminder_notice_in_english() {
        i18n::set_lang(i18n::Lang::En);
        let due = local_unix(date(26), 8, 30);
        let missed = reminder_notice(&due_reminder(due), due + 5, true);
        assert_eq!(
            (missed.source.as_str(), missed.body.as_str()),
            ("Reminder", "Missed · 08:30")
        );
    }

    #[test]
    fn reminder_timer_waits_for_the_next_but_at_most_an_hour() {
        assert_eq!(reminder_wait_ms(None, 5_000), None);
        // Due in 90 s.
        assert_eq!(reminder_wait_ms(Some(100), 10_000), Some(90_001));
        // Already due (a moment ago): at once.
        assert_eq!(reminder_wait_ms(Some(100), 100_500), Some(1));
        // Tomorrow: an hour, then it looks again.
        assert_eq!(reminder_wait_ms(Some(86_400), 0), Some(3_600_000));
    }

    #[test]
    fn reminder_editor_opens_the_reminder_or_a_new_one_next_hour() {
        let dir = std::env::temp_dir().join(format!(
            "dynamic-island-app-reminder-kind-{}",
            std::process::id()
        ));
        let mut store = RemindersStore::open(dir.join("reminders.json"));
        let due = local_unix(date(27), 7, 15);
        let id = store
            .upsert(None, "Telepon ibu", due, reminders::Repeat::Weekly, 1)
            .unwrap();
        let now = date(26).and_hms_opt(22, 40, 0).unwrap();
        assert_eq!(
            reminder_editor_kind(&store, Some(&id), now),
            EditorKind::Reminder {
                id: Some(id),
                text: "Telepon ibu".into(),
                date: date(27),
                time: NaiveTime::from_hms_opt(7, 15, 0).unwrap(),
                repeat: reminders::Repeat::Weekly,
            }
        );
        let new = |now| match reminder_editor_kind(&store, Some("r0-gone"), now) {
            EditorKind::Reminder {
                id: None,
                text,
                date,
                time,
                repeat: reminders::Repeat::Once,
            } if text.is_empty() => (date, time),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            new(now),
            (date(26), NaiveTime::from_hms_opt(23, 0, 0).unwrap())
        );
        let late = date(26).and_hms_opt(23, 5, 0).unwrap();
        assert_eq!(new(late), (date(27), NaiveTime::MIN), "past midnight");
        assert!(!dir.exists(), "nothing written");
    }

    #[test]
    fn unreadable_reminders_file_is_announced() {
        let dir = std::env::temp_dir().join(format!(
            "dynamic-island-app-reminders-bad-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reminders.json"), "[{").unwrap();
        let store = RemindersStore::open(dir.join("reminders.json"));
        let _ = std::fs::remove_dir_all(&dir);
        let notice = reminders_unreadable_notice(&store).expect("a notice");
        assert_eq!(notice.title, "reminders.json tidak terbaca");
        assert_eq!(
            reminders_unreadable_notice(&RemindersStore::open(dir.join("none.json"))),
            None
        );
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
    }

    #[test]
    fn client_id_notice_points_to_the_readme() {
        let n = client_id_notice();
        assert_eq!(n.source, "Kalender");
        assert_eq!(n.title, "Isi Google client ID dulu");
        assert_eq!(n.body, "Lihat README");
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
