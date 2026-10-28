use std::path::PathBuf;
use std::time::Instant;

use chrono::NaiveDate;

use crate::calendar::CalendarSnapshot;
use crate::clean::{CleanCat, CleanEvent, CleanMode};
use crate::config::OpenMode;
use crate::cli::CliEvent;
use crate::lyrics::LyricsFound;
use crate::mole::{MoleEvent, MoleOpt, MoleOpts};
use crate::pages::PanelPages;
use crate::pomodoro::PomoSettings;
use crate::screenshot::{FileCandidate, FileOutcome};
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
    /// The cleaner finished: what it freed, measured on the drive (Bata
    /// happy). Temporary; a click only dismisses it.
    CleanDone,
    /// A Mole clean finished: the free space the island measured (Bata
    /// happy, a "Log" chip). A click opens the cleaner on Mole's log.
    MoleDone,
    /// A Mole clean failed or stopped (Bata low, "Log"). Sticky like a
    /// reminder: it stays until clicked, which opens Mole's log.
    MoleFailed,
    /// "Disalin": an entry was copied from the island (or, with the setting,
    /// any copy). Short (`COPIED_DURATION`); a newer one replaces it.
    Copied,
    /// The big screenshot preview (`ShapeKind::NoticeShot`): its thumbnail,
    /// size and Salin lagi / Simpan / Hapus. `shot` names `App`'s screenshot
    /// state; it shows `secs` from when it becomes current (never expiring
    /// under the pointer), and only a true full-time expiry with
    /// `delete_if_ignored` ("Jika diabaikan: Hapus") asks to delete.
    Screenshot {
        shot: u64,
        secs: u32,
        delete_if_ignored: bool,
    },
    /// "Disimpan · Pictures\Screenshots" after Simpan (Bata happy).
    ShotSaved,
    /// "Dihapus · clipboard dikosongkan" after Hapus (Bata idle).
    ShotDeleted,
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
/// + (now − updated) × rate` (`lyrics::position_at`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    pub position_ms: u64,
    /// The track's length; 0 when the player does not report one.
    pub end_ms: u64,
    /// When `position_ms` was true.
    pub updated: Instant,
    /// The playback rate (1.0 normally; a player may play faster or slower).
    pub rate: f32,
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
    /// The Clipboard page's search field has the keyboard (true) or lost it;
    /// it keeps the panel open like the CLI input.
    ClipFocus(bool),
    /// The optional global hotkey: opens the panel on the Clipboard page
    /// (without taking the foreground), or closes it when it shows that.
    ClipHotkey,
    /// The hotkey's panel was not visited in time: it closes.
    ClipHoldEnd,
    /// Enter in the search field copied an entry: the panel closes like
    /// after a shortcut, and the keyboard goes back.
    ClipCommit,
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
    /// A scan or a clean from the cleaner thread (`clean::Cleaner`); `App`
    /// keeps it, the reducer ignores it.
    Clean(CleanEvent),
    /// The clean the panel asked for is over: its rows take clicks again.
    CleanFinished,
    /// From the Mole thread (`mole::MoleWorker`); `App` keeps it and tells
    /// the reducer what it needs below.
    Mole(MoleEvent),
    /// Mole was looked for after `Effect::MoleDetect`: found or not.
    MoleFound(bool),
    /// A dry-run with these options ended; `ok` when it finished well (a
    /// `mole::Preview` exists for them).
    MolePreviewed {
        opts: MoleOpts,
        ok: bool,
    },
    /// The confirmed Mole clean ended (`ok`: everything ran well).
    MoleFinished {
        ok: bool,
    },
    /// The cleaner's remembered mode (`clean_mode`), at startup.
    CleanModeSetting(CleanMode),
    /// "Mode buka" (startup and the settings window).
    OpenMode(OpenMode),
    /// A mouse button went down outside the island while the panel was kept
    /// open: it closes.
    Dismiss,
    /// "Halaman panel", at startup and when the settings window changes it.
    PanelPages(PanelPages),
    /// A folder check for screenshot `shot` ended: the Screenshots folder
    /// and the file that matched, if any. `App` keeps it, the reducer
    /// ignores it.
    ShotFolder {
        shot: u64,
        dir: Option<PathBuf>,
        found: Option<FileCandidate>,
    },
    /// "Simpan" wrote screenshot `shot` as a PNG: its path, `None` when it
    /// could not be written. `App` raises the notice; the reducer ignores it.
    ShotSaveDone {
        shot: u64,
        path: Option<PathBuf>,
    },
    /// "Hapus" ran for `shot`: what became of the file, and whether the
    /// clipboard was emptied. `App` raises the notice; the reducer ignores it.
    ShotRecycled {
        shot: u64,
        file: FileOutcome,
        emptied: bool,
    },
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
    /// The lyric offset one step earlier (−1) or later (+1) on the Music page.
    LyricOffset(i8),
    /// The offset back to 0.
    LyricOffsetReset,
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
    /// The System page's "Bersihkan" button: the cleaner opens over the
    /// page and scans.
    SystemClean,
    /// The cleaner's back button: the System page again.
    CleanBack,
    /// A cleaner row: its category ticked or not.
    CleanToggle(CleanCat),
    /// The cleaner's main button: the first click arms it ("Yakin?"), a
    /// second one within `CLEAN_CONFIRM` cleans.
    CleanGo,
    /// The cleaner's mode segment: "Pilih sendiri" or "Otomatis · Mole".
    CleanMode(CleanMode),
    /// "+ Sistem (admin)" or "+ Media game > 90 hari": a new preview runs.
    MoleOpt(MoleOpt),
    /// "Jalankan Mole": arms ("Yakin? Hapus permanen"); a second click
    /// within `CLEAN_CONFIRM`, after a preview with these options, cleans.
    /// After a run: "Pratinjau lagi".
    MoleGo,
    /// "Lindungi folder…": Mole's whitelist in a visible window.
    MoleProtect,
    /// "Cara memasang ↗": Mole's README in the browser.
    MoleInstall,
    /// "lihat log" (an unreadable group) or the log shown: the preview's
    /// log in place of the groups, and back.
    MoleShowLog,
    /// The Clipboard page's search field: it takes the keyboard.
    ClipSearch,
    /// A Clipboard card (entry id): copied again.
    ClipCard(u64),
    /// The selected card's buttons: pin or unpin, copy as plain text, delete.
    ClipPin(u64),
    ClipPlain(u64),
    ClipDelete(u64),
    /// The screenshot preview's buttons and its thumbnail (screenshot id):
    /// "Salin lagi", "Simpan", "Hapus", and opening the file.
    ShotCopy(u64),
    ShotKeep(u64),
    ShotDelete(u64),
    ShotOpen(u64),
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
    /// The lyric offset one step earlier (−1) or later (+1), or back to 0
    /// (`None`): `App` keeps it in the config and re-syncs the line.
    LyricOffset(Option<i32>),
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
    /// The cleaner opened: measure the categories.
    OpenCleaner,
    /// Clean these categories. Only after a confirmed second click.
    CleanDelete(Vec<CleanCat>),
    /// Remember the cleaner's mode (`clean_mode`).
    SaveCleanMode(CleanMode),
    /// Look for Mole (`Event::MoleFound` answers).
    MoleDetect,
    /// Mole's dry-run with these options; one running stops first.
    MolePreview(MoleOpts),
    /// Stop a running dry-run (the cleaner closed or its mode changed).
    MoleStopPreview,
    /// The confirmed Mole clean, with the options of the finished preview.
    /// Only on the armed button's second click.
    MoleClean(MoleOpts),
    /// `clean --whitelist` in a visible window.
    MoleWhitelist,
    /// Mole's README in the browser.
    OpenMoleInstall,
    /// Give the Clipboard search field the keyboard (the island activates).
    FocusClipSearch,
    /// The search field lets go; the window that had the keyboard gets it
    /// back.
    ReleaseClipFocus,
    /// Something to do with a history entry.
    Clip(ClipAction),
    /// Something to do with the screenshot on preview.
    Shot(ShotAction),
}

/// What the screenshot preview asks for (screenshot id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotAction {
    /// "Salin lagi": the image onto the clipboard again; the file stays.
    Copy(u64),
    /// "Simpan": the file stays; "Disimpan" shows.
    Keep(u64),
    /// "Hapus" clicked: the file to the Recycle Bin, the clipboard emptied
    /// if it still holds the image, its history card gone.
    Delete(u64),
    /// The preview ran its full time untouched and was raised with "Jika
    /// diabaikan: Hapus". `App` deletes as for `Delete` only if that is
    /// still the setting now; otherwise the file is kept (Simpan).
    DeleteIgnored(u64),
    /// The thumbnail: the file in its default app.
    Open(u64),
}

/// What a Clipboard card's click or button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipAction {
    /// Onto the clipboard in its own form.
    Copy(u64),
    /// Onto the clipboard as text only (not for an image).
    CopyPlain(u64),
    /// Pin or unpin.
    TogglePin(u64),
    Delete(u64),
}

/// A Pomodoro setting the settings window's steppers can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PomoField {
    Focus,
    ShortBreak,
    LongBreak,
    Sessions,
}

/// Panel pages, in scroll order: Kalender, Pomodoro, Pengingat, Catatan,
/// Musik, Clipboard, Sistem, then CLI and Pintasan. This is the interim
/// order; Run 9 sets the final one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Calendar,
    Pomodoro,
    Reminders,
    Notes,
    Media,
    /// The clipboard history.
    Clipboard,
    /// Sistem: CPU, RAM, disks and the busiest processes.
    System,
    Cli,
    /// Pintasan, the right-most page.
    Shortcuts,
}

impl Page {
    /// Interim order (Clipboard, then Sistem); Run 9 sets the final order.
    pub const ALL: [Page; 9] = [
        Page::Calendar,
        Page::Pomodoro,
        Page::Reminders,
        Page::Notes,
        Page::Media,
        Page::Clipboard,
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
