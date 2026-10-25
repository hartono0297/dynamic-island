use std::time::Instant;

use chrono::NaiveDate;

use crate::calendar::CalendarSnapshot;
use crate::lyrics::LyricsFound;
use crate::pomodoro::PomoSettings;

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub art: Option<Vec<u8>>,
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
    /// A calendar sync result; `App` keeps it, the reducer ignores it.
    Calendar(CalendarSnapshot),
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCommand {
    Prev,
    PlayPause,
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
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Calendar,
        Page::Pomodoro,
        Page::Reminders,
        Page::Notes,
        Page::Media,
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
