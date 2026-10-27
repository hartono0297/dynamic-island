use std::path::PathBuf;
use std::time::Instant;

use chrono::NaiveDate;

use crate::calendar::CalendarSnapshot;
use crate::cli::CliEvent;
use crate::lyrics::LyricsFound;
use crate::pomodoro::PomoSettings;
use crate::system::SystemSample;

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// Identifies the notice for de-duplication and tests; not shown to the user.
    pub key: String,
    pub source: String,
    pub title: String,
    pub body: String,
    pub kind: NoticeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NoticeKind {
    App {
        app_id: Option<String>,
    },
    ClaudeDone {
        session_id: String,
    },
    /// Sticky: stays until the same session shows activity or finishes.
    ClaudeWaiting {
        session_id: String,
    },
    BatteryLow,
    /// A pomodoro phase ended; clicking it starts the next phase.
    Pomodoro,
    /// Sticky like `ClaudeWaiting`: a reminder whose time came. Clicking it
    /// marks it done (or moves a repeating one to its next time).
    Reminder {
        id: String,
    },
    /// The morning greeting: today's events and reminders, from Bata. Shows
    /// `GREETING_DURATION`, never in fullscreen.
    Greeting,
    /// A fixed drive below the disk limit, or RAM above its limit for a
    /// minute (`system::WarnMonitor`). Clicking it opens the panel on the
    /// System page.
    SystemLow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub art: Option<Vec<u8>>,
    /// The session's AppUserModelId: transport commands go to the session
    /// with this id, not to whichever one Windows calls current.
    pub source_app: String,
}

/// Where the current track's playback was at a known moment, from the media
/// session's timeline. While it plays, the position at `now` is `position_ms
/// + (now − updated)` (`lyrics::position_at`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    pub position_ms: u64,
    /// The track's length; 0 when the player does not report one.
    pub end_ms: u64,
    /// When `position_ms` was true.
    pub updated: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `None` means no media session is active.
    Media(Option<MediaInfo>),
    /// The session's timeline (seek, track change, play/pause); `None` when
    /// it reports none. Sent apart from `Media` so a seek never resends art.
    MediaTimeline(Option<Timeline>),
    /// Synced lyrics for the current track exist (`App` decides); the
    /// compact media view may show them.
    LyricsReady(bool),
    /// The "Lirik di notch" setting, at startup and when the settings window
    /// changes it.
    LyricsSetting(bool),
    /// A lookup by the lyrics worker finished; `App` keeps it, the reducer
    /// ignores it.
    Lyrics(LyricsFound),
    Notice(Notice),
    ClaudeActivity {
        session_id: String,
    },
    Fullscreen(bool),
    Hover(bool),
    Click(Hit),
    /// Raw `WM_MOUSEWHEEL` delta: +120 per notch away from the user.
    Wheel(i32),
    /// A note, event or reminder editor window opened (true) or closed (false).
    EditorOpen(bool),
    /// The local date rolled over.
    Today(NaiveDate),
    /// The reminder was edited, deleted or dismissed elsewhere: its sticky
    /// notice goes.
    ReminderCleared(String),
    /// The CLI page's input has the keyboard (true) or lost it: like an open
    /// editor, it keeps the panel open meanwhile.
    CliFocus(bool),
    /// Scroll the CLI output `lines` up (negative: down), at most `max` lines
    /// above the newest: the wheel over the output, PageUp and PageDown.
    CliScroll {
        lines: i32,
        max: usize,
    },
    /// This many new output lines arrived; a view scrolled up stays on the
    /// lines it shows.
    CliOutput(usize),
    /// A calendar sync result; `App` keeps it, the reducer ignores it.
    Calendar(CalendarSnapshot),
    /// Output or the end of a CLI command; `App` keeps it, the reducer ignores it.
    Cli(CliEvent),
    /// The open File Explorer folders, the last active first, from the
    /// reader `request` started; `App` keeps them, the reducer ignores them.
    ExplorerFolders {
        request: u64,
        folders: Vec<PathBuf>,
    },
    /// A reading from the system sampler (`system::Sampler`); `App` keeps
    /// it, the reducer ignores it.
    SystemSample(SystemSample),
    Tick,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Body,
    Prev,
    PlayPause,
    Next,
    CalPrev,
    CalNext,
    CalViewSel(CalView),
    CalDate(NaiveDate),
    /// Google event id.
    CalEvent(String),
    /// "+N lagi" on a day in the month or week view.
    CalMore(NaiveDate),
    CalAdd,
    CalConnect,
    PomoStartPause,
    PomoReset,
    PomoSkip,
    /// From the settings window; the panel page has no steppers.
    PomoAdjust(PomoField, i32),
    NoteNew,
    /// Note id.
    NoteOpen(String),
    NotesMore,
    ReminderNew,
    /// Reminder id.
    ReminderOpen(String),
    RemindersMore,
    /// The Musik page's "Lirik" button: lyrics in the notch on or off.
    LyricsToggle,
    /// The strip at the top of the open panel: the newest Claude waiting or
    /// due reminder notice.
    Strip,
    /// A tile on the Pintasan page.
    Shortcut(Shortcut),
    /// The CLI page's terminal: the input takes the keyboard.
    CliInput,
    /// The CLI header's Stop button (the same as Ctrl+C).
    CliStop,
    /// The CLI header's enlarge / shrink button.
    CliExpand,
    /// The CLI header's folder chip (or the "dari Explorer" chip): opens or
    /// closes the folder menu.
    CliFolder,
    /// A folder row of the menu: the CLI moves there. `favorite`: the row is
    /// under Favorit (a right-click on it unstars it).
    CliFolderPick {
        path: PathBuf,
        favorite: bool,
    },
    /// A row's star: `on` stars the folder, off unstars it.
    CliFolderStar {
        path: PathBuf,
        on: bool,
    },
    /// Anywhere on the page outside the open menu: it closes.
    CliMenuDismiss,
    /// A drive's meter on the System page: Explorer opens on it.
    SystemDrive(char),
    /// A busiest-process row on the System page: Task Manager opens.
    SystemProcess,
    /// The System page's "Bersihkan" button (no region until the cleaner
    /// exists, Run 5).
    SystemClean,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCommand {
    Prev,
    /// Explicit, from the state the island shows: never a blind toggle.
    Play,
    Pause,
    Next,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenApp(String),
    Media(MediaCommand),
    PlaySound,
    SavePomodoro(PomoSettings),
    ConnectGoogle,
    /// Fetch events for `[from, to)`.
    RefreshCalendar {
        from: NaiveDate,
        to: NaiveDate,
    },
    /// `None` opens an empty editor for a new note.
    OpenNoteEditor(Option<String>),
    /// `event_id: None` creates a new event on `date`.
    OpenEventEditor {
        event_id: Option<String>,
        date: NaiveDate,
    },
    /// `None` opens an empty editor for a new reminder.
    OpenReminderEditor(Option<String>),
    /// A due reminder's notice was clicked: done, or on to its next time.
    ReminderDone(String),
    /// The Musik page's lyric button changed the setting; save it.
    SaveLyrics(bool),
    /// Run a shortcut once the panel has collapsed.
    RunShortcut(Shortcut),
    /// Give the CLI input the keyboard (the island activates for it).
    FocusCliInput,
    /// The CLI input lets go of the keyboard; the window that had it
    /// before gets it back.
    ReleaseCliFocus,
    /// End the running command (Stop, Ctrl+C).
    StopCli,
    /// A folder picked in the menu: the CLI moves there (not while a
    /// command runs).
    CliChangeFolder(PathBuf),
    /// Star (`on`) or unstar a folder in the menu.
    CliFavorite {
        path: PathBuf,
        on: bool,
    },
    /// Explorer on this drive's root, once the panel has collapsed.
    OpenDrive(char),
    /// The cleaner (Run 5); nothing yet.
    OpenCleaner,
}

/// A Pomodoro setting the settings window's steppers can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PomoField {
    Focus,
    ShortBreak,
    LongBreak,
    Sessions,
}

/// Panel pages, in scroll order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Calendar,
    Pomodoro,
    Reminders,
    Notes,
    Media,
    /// Sistem: CPU, RAM, disks and the busiest processes.
    System,
    Cli,
    /// Pintasan, the right-most page.
    Shortcuts,
}

impl Page {
    /// Clipboard (Run 7) is not here yet; Run 9 sets the final order.
    pub const ALL: [Page; 8] = [
        Page::Calendar,
        Page::Pomodoro,
        Page::Reminders,
        Page::Notes,
        Page::Media,
        Page::System,
        Page::Cli,
        Page::Shortcuts,
    ];
}

/// The Pintasan page's tiles, in reading order (two rows of four).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Screenshot,
    Clipboard,
    Emoji,
    Explorer,
    Calculator,
    TaskManager,
    WindowsSettings,
    Lock,
}

impl Shortcut {
    pub const ALL: [Shortcut; 8] = [
        Shortcut::Screenshot,
        Shortcut::Clipboard,
        Shortcut::Emoji,
        Shortcut::Explorer,
        Shortcut::Calculator,
        Shortcut::TaskManager,
        Shortcut::WindowsSettings,
        Shortcut::Lock,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalView {
    Day,
    Week,
    Month,
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    /// Only in fullscreen.
    Hidden,
    /// The empty black shape.
    Idle,
    MediaCompact,
    /// The compact media view showing the current synced lyric line.
    MediaLyric,
    PomodoroCompact,
    Panel(Page),
    Notice(Notice),
}
