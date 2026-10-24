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
    Tick,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Body,
    Prev,
    PlayPause,
    Next,
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Hidden,
    MediaCompact,
    MediaExpanded,
    Notice(Notice),
}
