use chrono::NaiveDate;

use crate::calendar::CalendarSnapshot;
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

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `None` means no media session is active.
    Media(Option<MediaInfo>),
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
    PomodoroCompact,
    Panel(Page),
    Notice(Notice),
}
