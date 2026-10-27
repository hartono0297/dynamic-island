use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{Days, Months, NaiveDate};

use crate::calendar::range_for;
use crate::i18n::{self, Key, text};
use crate::lyrics;
use crate::model::{
    CalView, Effect, Event, Hit, MediaCommand, MediaInfo, Notice, NoticeKind, Page, Timeline, View,
};
use crate::pomodoro::{Phase, PhaseDone, PomoSettings, PomoStatus, Pomodoro};
use crate::ui::geometry::ANIM_MS;

/// How long each temporary notice stays on screen, counted from when it becomes current.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);
/// The morning greeting goes a little sooner.
pub const GREETING_DURATION: Duration = Duration::from_secs(4);

/// How long `notice` stays once it is current.
fn duration_of(notice: &Notice) -> Duration {
    match notice.kind {
        NoticeKind::Greeting => GREETING_DURATION,
        _ => NOTICE_DURATION,
    }
}

/// After hover opens the panel, its media buttons ignore clicks until the
/// opening animation (`ANIM_MS`) has finished and this much longer: a pointer
/// on its way to a browser tab must not land on play/pause as the panel
/// grows under it.
pub const MEDIA_HOVER_GUARD: Duration = Duration::from_millis(ANIM_MS + 300);

/// One mouse-wheel notch, as Windows reports it.
const WHEEL_STEP: i32 = 120;

/// Key shared by all pomodoro notices, so a newer one replaces an older one.
const POMODORO_KEY: &str = "pomodoro";

/// Decides what the island shows. Pure logic: OS sources feed it `Event`s and the
/// renderer draws its `View`; the caller performs the returned `Effect`s.
pub struct IslandState {
    media: Option<MediaInfo>,
    /// The media session's last reported timeline, as the watcher sent it
    /// (re-based here when play/pause flips).
    timeline: Option<Timeline>,
    /// "Lirik di notch" is on.
    lyrics_on: bool,
    /// `App` has synced lyrics for the current track.
    lyrics_ready: bool,
    queue: VecDeque<Notice>,
    current: Option<(Notice, Instant)>,
    /// Sticky notices, oldest first; the newest is shown. At most one
    /// `ClaudeWaiting` per session and one `Reminder` per reminder.
    sticky: Vec<Notice>,
    fullscreen: bool,
    hover: bool,
    /// When hover last opened the closed panel; cleared when it closes.
    /// The media buttons wait `MEDIA_HOVER_GUARD` from then.
    hover_opened: Option<Instant>,
    /// An editor window is open; it keeps the panel open like hover.
    editor_open: bool,
    /// The CLI page's input has the keyboard; it keeps the panel open like
    /// an editor.
    cli_focused: bool,
    /// The CLI page shows the enlarged panel; back to small when the panel
    /// closes.
    cli_tall: bool,
    /// How many lines the CLI output is scrolled up from the newest; 0
    /// follows new output. Reset when the panel closes.
    cli_scroll: usize,
    /// The CLI page's folder menu is open. It never takes the keyboard;
    /// closed by a pick, a click beside it, Esc, a page change or the
    /// panel closing.
    cli_menu: bool,
    /// A shortcut tile was clicked: the panel stays shut, whatever keeps it
    /// open (an editor; the CLI input lets go), until the pointer leaves the island
    /// and comes back. Leaving alone is not enough: once the panel shrinks
    /// the pointer is over transparent pixels, so the leave arrives at the
    /// next small move, before the screen clip or picker is up.
    collapsed: Collapse,
    pomodoro: Pomodoro,
    /// The panel page, kept across closes so the panel reopens where it was left.
    page: Page,
    /// Wheel delta not yet worth a whole page; cleared when the panel closes.
    wheel: i32,
    notes_page: usize,
    reminders_page: usize,
    /// Day view: which chunk of the day's events "+N lagi" has paged to.
    day_page: usize,
    today: NaiveDate,
    cal_view: CalView,
    cal_anchor: NaiveDate,
    cal_selected: NaiveDate,
}

impl IslandState {
    pub fn new(today: NaiveDate, pomo: PomoSettings) -> Self {
        Self {
            media: None,
            timeline: None,
            lyrics_on: false,
            lyrics_ready: false,
            queue: VecDeque::new(),
            current: None,
            sticky: Vec::new(),
            fullscreen: false,
            hover: false,
            hover_opened: None,
            editor_open: false,
            cli_focused: false,
            cli_tall: false,
            cli_scroll: 0,
            cli_menu: false,
            collapsed: Collapse::No,
            pomodoro: Pomodoro::new(pomo),
            page: Page::Calendar,
            wheel: 0,
            notes_page: 0,
            reminders_page: 0,
            day_page: 0,
            today,
            cal_view: CalView::Month,
            cal_anchor: today,
            cal_selected: today,
        }
    }

    pub fn apply(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        let calendar_was_shown = self.view() == View::Panel(Page::Calendar);
        let calendar_was = (self.cal_view, self.cal_anchor);
        let selected_was = self.cal_selected;
        let panel_was_open = self.panel_open();
        let hover_on = matches!(event, Event::Hover(true));
        let mut effects = Vec::new();
        // Finish a phase whose end has passed before handling the input, so a
        // click that beats the deadline's Tick never pauses the timer at 0:00.
        if let Some(done) = self.pomodoro.tick(now) {
            self.phase_done(done, &mut effects);
        }
        self.advance(now);
        match event {
            Event::Media(media) => self.set_media(media, now),
            Event::MediaTimeline(timeline) => self.timeline = timeline,
            Event::LyricsReady(ready) => self.lyrics_ready = ready,
            Event::LyricsSetting(on) => self.lyrics_on = on,
            Event::Notice(notice) => match &notice.kind {
                NoticeKind::ClaudeWaiting { .. } | NoticeKind::Reminder { .. } => {
                    if let Some(key) = sticky_key(&notice) {
                        self.clear_sticky(key);
                    }
                    // The sticky slot is hidden in fullscreen, so show it briefly like
                    // any other new notice; it stays in `sticky` for afterwards.
                    if self.fullscreen {
                        self.queue.push_back(notice.clone());
                    }
                    self.sticky.push(notice);
                }
                NoticeKind::ClaudeDone { session_id } => {
                    self.clear_sticky(Sticky::Session(session_id));
                    self.queue.push_back(notice);
                }
                NoticeKind::App { .. } | NoticeKind::BatteryLow | NoticeKind::Pomodoro => {
                    self.queue.push_back(notice)
                }
                // Never over a fullscreen app (`App` checks first and tries
                // again later; this only guards a race).
                NoticeKind::Greeting => {
                    if !self.fullscreen {
                        self.queue.push_back(notice);
                    }
                }
            },
            Event::ClaudeActivity { session_id } => self.clear_sticky(Sticky::Session(&session_id)),
            Event::Fullscreen(on) => self.fullscreen = on,
            Event::Hover(on) => {
                self.hover = on;
                self.collapsed = match (self.collapsed, on) {
                    (Collapse::Shut, false) => Collapse::Left,
                    // A new visit: the user came back for the panel.
                    (Collapse::Left, true) => Collapse::No,
                    (c, _) => c,
                };
            }
            Event::EditorOpen(on) => self.editor_open = on,
            Event::CliFocus(on) => self.cli_focused = on,
            Event::CliScroll { lines, max } => {
                if self.view() == View::Panel(Page::Cli) {
                    let scrolled = self.cli_scroll as i64 + i64::from(lines);
                    self.cli_scroll = scrolled.clamp(0, max as i64) as usize;
                }
            }
            Event::CliOutput(n) => {
                // A view scrolled up stays on what it shows; at 0 it follows.
                if self.cli_scroll > 0 {
                    self.cli_scroll = self.cli_scroll.saturating_add(n);
                }
            }
            Event::Wheel(delta) => self.wheel(delta),
            Event::Today(date) => self.set_today(date),
            Event::ReminderCleared(id) => self.clear_sticky(Sticky::Reminder(&id)),
            Event::Click(hit) => self.click(hit, now, &mut effects),
            Event::Calendar(_)
            | Event::Lyrics(_)
            | Event::Cli(_)
            | Event::ExplorerFolders { .. }
            | Event::Tick => {}
        }
        // A new notice or a dismissal may have left the slot free for the next queued one.
        self.advance(now);
        // The input lets go once its page is gone: another page, or the
        // panel shut for a shortcut.
        if self.cli_focused && (self.page != Page::Cli || self.collapsed != Collapse::No) {
            self.cli_focused = false;
            effects.push(Effect::ReleaseCliFocus);
        }
        // The menu belongs to the CLI page of an open panel.
        if self.cli_menu && (self.page != Page::Cli || !self.panel_open()) {
            self.cli_menu = false;
        }
        if hover_on && !panel_was_open && self.panel_open() {
            self.hover_opened = Some(now);
        }
        // Closed, or no longer open by hover (an editor or the CLI input
        // holds it after the pointer left): no guard.
        if !self.panel_open() || !self.hover {
            self.hover_opened = None;
        }
        if panel_was_open && !self.panel_open() {
            self.wheel = 0;
            self.notes_page = 0;
            self.reminders_page = 0;
            self.cli_tall = false;
            self.cli_scroll = 0;
        }
        let calendar_shown = self.view() == View::Panel(Page::Calendar);
        // Any navigation, a view change, or the calendar leaving the screen
        // (panel closed or another page) starts the day list from the top.
        if calendar_was != (self.cal_view, self.cal_anchor)
            || selected_was != self.cal_selected
            || !calendar_shown
        {
            self.day_page = 0;
        }
        if (calendar_shown && !calendar_was_shown)
            || calendar_was != (self.cal_view, self.cal_anchor)
        {
            let (from, to) = self.cal_range();
            effects.push(Effect::RefreshCalendar { from, to });
        }
        effects
    }

    pub fn view(&self) -> View {
        if let Some((notice, _)) = &self.current {
            return View::Notice(notice.clone());
        }
        if self.fullscreen {
            return View::Hidden;
        }
        // Sticky notices do not block the panel; they show as a strip inside it.
        if self.panel_open() {
            return View::Panel(self.page);
        }
        if let Some(notice) = self.sticky.last() {
            return View::Notice(notice.clone());
        }
        if self.pomodoro.is_active() {
            return View::PomodoroCompact;
        }
        if let Some(media) = &self.media {
            if self.lyrics_on && self.lyrics_ready && media.playing {
                return View::MediaLyric;
            }
            return View::MediaCompact;
        }
        View::Idle
    }

    pub fn media(&self) -> Option<&MediaInfo> {
        self.media.as_ref()
    }

    /// A fullscreen app is in front (the island hides).
    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_ref()
    }

    pub fn lyrics_on(&self) -> bool {
        self.lyrics_on
    }

    /// The CLI input has the keyboard and holds the panel open.
    pub fn cli_focused(&self) -> bool {
        self.cli_focused
    }

    /// The CLI page shows the enlarged (560 × 440) panel.
    pub fn cli_tall(&self) -> bool {
        self.cli_tall
    }

    /// Lines the CLI output is scrolled up from the newest.
    pub fn cli_scroll(&self) -> usize {
        self.cli_scroll
    }

    /// The CLI page's folder menu is open.
    pub fn cli_menu(&self) -> bool {
        self.cli_menu
    }

    /// The panel is open (hover, an editor or the CLI input holds it), even
    /// while a notice covers it.
    pub fn panel_is_open(&self) -> bool {
        self.panel_open()
    }

    pub fn lyrics_ready(&self) -> bool {
        self.lyrics_ready
    }

    /// A new track's lyrics are not known yet. When the same track pauses or
    /// resumes, the timeline is re-based at that moment, so a player that
    /// does not report a fresh one neither runs on while paused nor jumps
    /// ahead when it resumes (a fresh one from the watcher replaces it).
    fn set_media(&mut self, media: Option<MediaInfo>, now: Instant) {
        match (&self.media, &media) {
            (Some(old), Some(new)) if old.title == new.title && old.artist == new.artist => {
                if old.playing != new.playing
                    && let Some(t) = &self.timeline
                {
                    self.timeline = Some(lyrics::rebase(t, old.playing, now));
                }
            }
            _ => self.lyrics_ready = false,
        }
        self.media = media;
    }

    /// The earliest of the current notice's expiry and the pomodoro phase end;
    /// the caller should deliver a `Tick` then.
    pub fn next_deadline(&self) -> Option<Instant> {
        let notice = self.current.as_ref().map(|(_, expires)| *expires);
        match (notice, self.pomodoro.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn panel_open(&self) -> bool {
        self.collapsed == Collapse::No && (self.hover || self.editor_open || self.cli_focused)
    }

    /// How long ago hover opened the panel; `None` when it is closed or held
    /// open by something else (an editor, the CLI input).
    pub fn hover_open_age(&self, now: Instant) -> Option<Duration> {
        self.hover_opened
            .map(|at| now.saturating_duration_since(at))
    }

    /// The panel opened by hover less than `MEDIA_HOVER_GUARD` ago: its media
    /// buttons do nothing yet.
    pub fn media_guarded(&self, now: Instant) -> bool {
        self.hover_open_age(now)
            .is_some_and(|age| age < MEDIA_HOVER_GUARD)
    }

    fn click(&mut self, hit: Hit, now: Instant, effects: &mut Vec<Effect>) {
        if is_media_hit(&hit) && self.media_guarded(now) {
            return;
        }
        match hit {
            Hit::Body => match self.view() {
                View::Notice(_) => effects.extend(self.dismiss_visible_notice(now)),
                View::PomodoroCompact if self.pomodoro.status() == PomoStatus::Ready => {
                    self.pomodoro.start_pause(now)
                }
                _ => {}
            },
            Hit::Prev => effects.push(Effect::Media(MediaCommand::Prev)),
            // What the island shows decides: playing pauses, paused plays.
            Hit::PlayPause => {
                if let Some(media) = &self.media {
                    let cmd = if media.playing {
                        MediaCommand::Pause
                    } else {
                        MediaCommand::Play
                    };
                    effects.push(Effect::Media(cmd));
                }
            }
            Hit::Next => effects.push(Effect::Media(MediaCommand::Next)),
            Hit::PomoStartPause => self.pomodoro.start_pause(now),
            Hit::PomoReset => self.pomodoro.reset(),
            Hit::PomoSkip => {
                let done = self.pomodoro.skip(now);
                self.phase_done(done, effects);
            }
            Hit::PomoAdjust(field, dir) => {
                if self.pomodoro.adjust(field, dir) {
                    effects.push(Effect::SavePomodoro(self.pomodoro.settings()));
                }
            }
            Hit::CalPrev => self.move_anchor(false),
            Hit::CalNext => self.move_anchor(true),
            Hit::CalViewSel(view) => {
                self.cal_view = view;
                self.cal_anchor = self.cal_selected;
            }
            Hit::CalDate(date) => {
                self.cal_selected = date;
                if self.cal_view == CalView::Day {
                    self.cal_anchor = date;
                }
            }
            // In the day view on that date it pages through the day's events
            // (the layout wraps the count); elsewhere it opens that day.
            Hit::CalMore(date)
                if self.cal_view == CalView::Day
                    && self.cal_anchor == date
                    && self.cal_selected == date =>
            {
                self.day_page += 1
            }
            Hit::CalMore(date) => {
                self.cal_view = CalView::Day;
                self.cal_anchor = date;
                self.cal_selected = date;
            }
            Hit::CalAdd => effects.push(Effect::OpenEventEditor {
                event_id: None,
                date: self.cal_selected,
            }),
            Hit::CalEvent(id) => effects.push(Effect::OpenEventEditor {
                event_id: Some(id),
                date: self.cal_selected,
            }),
            Hit::CalConnect => effects.push(Effect::ConnectGoogle),
            Hit::NoteNew => effects.push(Effect::OpenNoteEditor(None)),
            Hit::NoteOpen(id) => effects.push(Effect::OpenNoteEditor(Some(id))),
            Hit::NotesMore => self.notes_page += 1,
            Hit::ReminderNew => effects.push(Effect::OpenReminderEditor(None)),
            Hit::ReminderOpen(id) => effects.push(Effect::OpenReminderEditor(Some(id))),
            Hit::RemindersMore => self.reminders_page += 1,
            Hit::LyricsToggle => {
                self.lyrics_on = !self.lyrics_on;
                effects.push(Effect::SaveLyrics(self.lyrics_on));
            }
            // The strip shows the newest sticky notice; clicking it dismisses that one.
            Hit::Strip => {
                if matches!(self.view(), View::Panel(_)) {
                    effects.extend(self.dismiss_newest_sticky());
                }
            }
            // The panel gets out of the way first: the screenshot or picker
            // is for the window underneath. `App` waits for that.
            Hit::Shortcut(s) => {
                if matches!(self.view(), View::Panel(_)) {
                    // Without hover (an editor or the CLI pins the panel) the
                    // pointer is already away: its next visit reopens it.
                    self.collapsed = if self.hover {
                        Collapse::Shut
                    } else {
                        Collapse::Left
                    };
                    effects.push(Effect::RunShortcut(s));
                }
            }
            Hit::CliInput
            | Hit::CliStop
            | Hit::CliExpand
            | Hit::CliFolder
            | Hit::CliFolderPick { .. }
            | Hit::CliFolderStar { .. }
            | Hit::CliMenuDismiss
                if self.view() != View::Panel(Page::Cli) =>
            {
                self.cli_menu = false;
            }
            Hit::CliInput => effects.push(Effect::FocusCliInput),
            Hit::CliStop => effects.push(Effect::StopCli),
            Hit::CliExpand => self.cli_tall = !self.cli_tall,
            // The menu is drawn in the panel: none of these touch the
            // keyboard (no FocusCliInput, no ReleaseCliFocus).
            Hit::CliFolder => self.cli_menu = !self.cli_menu,
            Hit::CliFolderPick { path, .. } => {
                if std::mem::take(&mut self.cli_menu) {
                    effects.push(Effect::CliChangeFolder(path));
                }
            }
            Hit::CliFolderStar { path, on } => {
                if self.cli_menu {
                    effects.push(Effect::CliFavorite { path, on });
                }
            }
            Hit::CliMenuDismiss => self.cli_menu = false,
        }
    }

    /// Scrolling down (negative delta) moves right through the pages. Steps
    /// past either end are spent, so one notch back always moves a page.
    fn wheel(&mut self, delta: i32) {
        if !matches!(self.view(), View::Panel(_)) {
            return;
        }
        self.wheel = self.wheel.saturating_add(delta);
        let steps = self.wheel / WHEEL_STEP;
        self.wheel -= steps * WHEEL_STEP;
        let index = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0) as i64;
        let last = Page::ALL.len() as i64 - 1;
        self.page = Page::ALL[(index - i64::from(steps)).clamp(0, last) as usize];
    }

    /// Day view: one day, and the selection follows. Week: 7 days. Month: one
    /// calendar month, clamped to the month's last day.
    fn move_anchor(&mut self, forward: bool) {
        let a = self.cal_anchor;
        let moved = match (self.cal_view, forward) {
            (CalView::Day, true) => a.checked_add_days(Days::new(1)),
            (CalView::Day, false) => a.checked_sub_days(Days::new(1)),
            (CalView::Week, true) => a.checked_add_days(Days::new(7)),
            (CalView::Week, false) => a.checked_sub_days(Days::new(7)),
            (CalView::Month, true) => a.checked_add_months(Months::new(1)),
            (CalView::Month, false) => a.checked_sub_months(Months::new(1)),
        };
        self.cal_anchor = moved.unwrap_or(a);
        if self.cal_view == CalView::Day {
            self.cal_selected = self.cal_anchor;
        }
    }

    /// Dates still pointing at the old today follow it to the new one.
    fn set_today(&mut self, date: NaiveDate) {
        if self.cal_anchor == self.today {
            self.cal_anchor = date;
        }
        if self.cal_selected == self.today {
            self.cal_selected = date;
        }
        self.today = date;
    }

    /// Announces a finished phase: one sound and a notice that replaces any
    /// earlier pomodoro notice still queued or showing.
    fn phase_done(&mut self, done: PhaseDone, effects: &mut Vec<Effect>) {
        let settings = self.pomodoro.settings();
        let lang = i18n::lang();
        let (title, body) = match (done.finished, done.next) {
            (Phase::Focus, next) => {
                let min = match next {
                    Phase::LongBreak => settings.long_break_min,
                    _ => settings.short_break_min,
                };
                (text(lang, Key::FocusDone), i18n::focus_done_body(lang, min))
            }
            _ => {
                // Lowering the session count while Ready can leave the
                // session number above it; never show "5 dari 4".
                let k = self.pomodoro.session().min(settings.sessions);
                (
                    text(lang, Key::BreakDone),
                    i18n::break_done_body(lang, k, settings.sessions),
                )
            }
        };
        self.queue.retain(|n| n.kind != NoticeKind::Pomodoro);
        if matches!(&self.current, Some((n, _)) if n.kind == NoticeKind::Pomodoro) {
            self.current = None;
        }
        self.queue.push_back(Notice {
            key: POMODORO_KEY.into(),
            source: text(lang, Key::Pomodoro).into(),
            title: title.into(),
            body,
            kind: NoticeKind::Pomodoro,
        });
        effects.push(Effect::PlaySound);
    }

    /// Drops the current notice once it has expired and promotes the next queued one.
    fn advance(&mut self, now: Instant) {
        if matches!(&self.current, Some((_, expires)) if *expires <= now) {
            self.current = None;
        }
        if self.current.is_none()
            && let Some(next) = self.queue.pop_front()
        {
            let expires = now + duration_of(&next);
            self.current = Some((next, expires));
        }
    }

    /// Removes the sticky notice for `key`, including a brief fullscreen copy
    /// that is queued or showing, so a resolved wait or a dismissed reminder is
    /// never displayed.
    fn clear_sticky(&mut self, key: Sticky) {
        let is_key = |n: &Notice| sticky_key(n) == Some(key);
        self.sticky.retain(|n| !is_key(n));
        self.queue.retain(|n| !is_key(n));
        if matches!(&self.current, Some((n, _)) if is_key(n)) {
            self.current = None;
        }
    }

    /// Dismisses the newest sticky notice: a reminder is then done.
    fn dismiss_newest_sticky(&mut self) -> Option<Effect> {
        let notice = self.sticky.last()?.clone();
        self.dismiss_sticky(&notice)
    }

    /// Removes a sticky notice (and its copies); a reminder is then done.
    fn dismiss_sticky(&mut self, notice: &Notice) -> Option<Effect> {
        self.clear_sticky(sticky_key(notice)?);
        match &notice.kind {
            NoticeKind::Reminder { id } => Some(Effect::ReminderDone(id.clone())),
            _ => None,
        }
    }

    /// Dismisses whichever notice the view is showing. Only called while the
    /// view is a notice: the current one, else the newest sticky one.
    fn dismiss_visible_notice(&mut self, now: Instant) -> Option<Effect> {
        let Some((notice, _)) = self.current.take() else {
            return self.dismiss_newest_sticky();
        };
        match notice.kind {
            NoticeKind::App { app_id: Some(id) } => Some(Effect::OpenApp(id)),
            NoticeKind::ClaudeWaiting { .. } | NoticeKind::Reminder { .. } => {
                self.dismiss_sticky(&notice)
            }
            NoticeKind::Pomodoro => {
                if self.pomodoro.status() == PomoStatus::Ready {
                    self.pomodoro.start_pause(now);
                }
                None
            }
            _ => None,
        }
    }
}

/// What the panel draws.
impl IslandState {
    pub fn page(&self) -> Page {
        self.page
    }

    /// The sticky notice (Claude waiting or a due reminder) to show at the top
    /// of the open panel.
    pub fn waiting_strip(&self) -> Option<&Notice> {
        match self.view() {
            View::Panel(_) => self.sticky.last(),
            _ => None,
        }
    }

    pub fn pomodoro(&self) -> &Pomodoro {
        &self.pomodoro
    }

    /// The local date as of the last `Event::Today`.
    pub fn today(&self) -> NaiveDate {
        self.today
    }

    pub fn cal_view(&self) -> CalView {
        self.cal_view
    }

    pub fn cal_anchor(&self) -> NaiveDate {
        self.cal_anchor
    }

    pub fn cal_selected(&self) -> NaiveDate {
        self.cal_selected
    }

    /// Dates `[from, to)` the calendar page shows.
    pub fn cal_range(&self) -> (NaiveDate, NaiveDate) {
        range_for(self.cal_view, self.cal_anchor)
    }

    /// How many times "+N lagi" was clicked since the panel opened; the notes
    /// layout wraps it around the list.
    pub fn notes_page(&self) -> usize {
        self.notes_page
    }

    /// How many times the day view's "+N lagi" was clicked on the open day;
    /// the day layout wraps it around that day's events.
    pub fn day_page(&self) -> usize {
        self.day_page
    }

    /// How many times the reminder list's "+N lagi" was clicked since the
    /// panel opened; the layout wraps it around the list.
    pub fn reminders_page(&self) -> usize {
        self.reminders_page
    }
}

/// The panel after a shortcut click.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Collapse {
    No,
    /// Shut, the pointer still on this visit.
    Shut,
    /// Shut, and the pointer has left; the next hover opens it again.
    Left,
}

/// What a sticky notice is for: one per Claude session, one per reminder.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sticky<'a> {
    Session(&'a str),
    Reminder(&'a str),
}

/// A transport button: previous, play/pause or next.
pub fn is_media_hit(hit: &Hit) -> bool {
    matches!(hit, Hit::Prev | Hit::PlayPause | Hit::Next)
}

fn sticky_key(notice: &Notice) -> Option<Sticky<'_>> {
    match &notice.kind {
        NoticeKind::ClaudeWaiting { session_id } => Some(Sticky::Session(session_id)),
        NoticeKind::Reminder { id } => Some(Sticky::Reminder(id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::pomodoro::{Phase, PomoSettings, PomoStatus};
    use chrono::NaiveDate;
    use std::time::{Duration, Instant};

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn mins(n: u64) -> Duration {
        Duration::from_secs(60 * n)
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn st() -> IslandState {
        IslandState::new(date(2026, 10, 25), PomoSettings::default())
    }

    /// Hovers and scrolls the open panel to `page`.
    fn open_on(s: &mut IslandState, page: Page, now: Instant) {
        s.apply(Event::Hover(true), now);
        let target = Page::ALL.iter().position(|p| *p == page).unwrap() as i32;
        let current = Page::ALL.iter().position(|p| *p == s.page()).unwrap() as i32;
        s.apply(Event::Wheel(-120 * (target - current)), now);
        assert_eq!(s.view(), View::Panel(page));
    }

    fn app_notice(key: &str) -> Notice {
        Notice {
            key: key.into(),
            source: "WhatsApp".into(),
            title: "Ibu".into(),
            body: "udah makan belum?".into(),
            kind: NoticeKind::App {
                app_id: Some("WhatsApp.Desktop!App".into()),
            },
        }
    }

    fn media(playing: bool) -> MediaInfo {
        MediaInfo {
            title: "Bohemian Rhapsody".into(),
            artist: "Queen".into(),
            playing,
            art: None,
            source_app: "Chrome".into(),
        }
    }

    fn waiting(session: &str) -> Notice {
        Notice {
            key: format!("claude-waiting-{session}"),
            source: "Claude Code".into(),
            title: "Claude is waiting".into(),
            body: "Needs your input".into(),
            kind: NoticeKind::ClaudeWaiting {
                session_id: session.into(),
            },
        }
    }

    fn done(session: &str) -> Notice {
        Notice {
            key: format!("claude-done-{session}"),
            source: "Claude Code".into(),
            title: "Claude is done".into(),
            body: "Task finished".into(),
            kind: NoticeKind::ClaudeDone {
                session_id: session.into(),
            },
        }
    }

    fn notice_title(view: &View) -> Option<(&NoticeKind, &str, &str)> {
        match view {
            View::Notice(n) => Some((&n.kind, n.title.as_str(), n.body.as_str())),
            _ => None,
        }
    }

    fn greeting() -> Notice {
        Notice {
            key: "greeting".into(),
            source: "Bata".into(),
            title: "Selamat pagi".into(),
            body: "3 acara dan 2 pengingat hari ini".into(),
            kind: NoticeKind::Greeting,
        }
    }

    #[test]
    fn greeting_shows_four_seconds_and_never_in_fullscreen() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(greeting()), t0);
        assert_eq!(s.view(), View::Notice(greeting()));
        assert_eq!(s.next_deadline(), Some(t0 + secs(4)));
        s.apply(Event::Tick, t0 + secs(4));
        assert_eq!(s.view(), View::Idle);
        // Fullscreen: dropped, not queued for afterwards.
        s.apply(Event::Fullscreen(true), t0);
        assert!(s.fullscreen());
        s.apply(Event::Notice(greeting()), t0);
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0);
        assert_eq!(s.view(), View::Idle);
        // A click dismisses it like any notice.
        s.apply(Event::Notice(greeting()), t0);
        let effects = s.apply(Event::Click(Hit::Body), t0);
        assert!(effects.is_empty());
        assert_eq!(s.view(), View::Idle);
    }

    // ---- v1 behaviour, adapted to v2 views ----

    #[test]
    fn media_shows_compact_and_hover_opens_panel() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-480), t0);
        assert_eq!(s.view(), View::Panel(Page::Media));
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn paused_media_still_shows() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(false))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        assert_eq!(s.media(), Some(&media(false)));
    }

    #[test]
    fn media_gone_returns_to_idle() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        s.apply(Event::Media(None), t0);
        assert_eq!(s.view(), View::Idle);
        assert_eq!(s.media(), None);
    }

    #[test]
    fn notice_shows_then_returns_to_media() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + Duration::from_millis(4900));
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn burst_of_notices_shows_each_in_order() {
        let t0 = Instant::now();
        let mut s = st();
        for i in 0..30 {
            s.apply(Event::Notice(app_notice(&format!("k{i}"))), t0);
        }
        for i in 0..30u32 {
            let at = t0 + NOTICE_DURATION * i;
            assert_eq!(
                s.view(),
                View::Notice(app_notice(&format!("k{i}"))),
                "at notice {i}"
            );
            assert_eq!(s.next_deadline(), Some(at + NOTICE_DURATION));
            s.apply(Event::Tick, t0 + NOTICE_DURATION * (i + 1));
        }
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn fullscreen_hides_media_but_not_new_notice() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Fullscreen(true), t0);
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0 + secs(5));
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn claude_waiting_is_sticky_until_same_session_activity() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        s.apply(Event::Tick, t0 + secs(60));
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        s.apply(
            Event::ClaudeActivity {
                session_id: "s2".into(),
            },
            t0 + secs(61),
        );
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        s.apply(
            Event::ClaudeActivity {
                session_id: "s1".into(),
            },
            t0 + secs(62),
        );
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn claude_done_clears_waiting_of_same_session() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(done("s1")), t0);
        assert_eq!(s.view(), View::Notice(done("s1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn temporary_notice_preempts_waiting_then_waiting_returns() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn click_app_notice_opens_app_and_dismisses() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(app_notice("k1")), t0);
        let effects = s.apply(Event::Click(Hit::Body), t0 + secs(1));
        assert_eq!(
            effects,
            vec![Effect::OpenApp("WhatsApp.Desktop!App".into())]
        );
        assert_eq!(s.view(), View::Idle);
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn click_media_buttons_emit_commands() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        open_on(&mut s, Page::Media, t0);
        // Once the hover guard has passed.
        let t = t0 + MEDIA_HOVER_GUARD;
        assert_eq!(
            s.apply(Event::Click(Hit::Next), t),
            vec![Effect::Media(MediaCommand::Next)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::Prev), t),
            vec![Effect::Media(MediaCommand::Prev)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), t),
            vec![Effect::Media(MediaCommand::Pause)]
        );
        assert_eq!(s.view(), View::Panel(Page::Media));
    }

    #[test]
    fn play_pause_follows_the_shown_state() {
        let t0 = Instant::now();
        let t = t0 + MEDIA_HOVER_GUARD;
        let mut s = st();
        open_on(&mut s, Page::Media, t0);
        // No media: nothing to command.
        assert_eq!(s.apply(Event::Click(Hit::PlayPause), t), vec![]);
        s.apply(Event::Media(Some(media(true))), t);
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), t),
            vec![Effect::Media(MediaCommand::Pause)],
            "playing pauses"
        );
        s.apply(Event::Media(Some(media(false))), t);
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), t),
            vec![Effect::Media(MediaCommand::Play)],
            "paused plays"
        );
    }

    #[test]
    fn media_buttons_wait_after_a_hover_open() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        open_on(&mut s, Page::Media, t0);
        s.apply(Event::Hover(false), t0);
        // Hover reopens it on the Music page, under a passing pointer.
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Media));
        let early = t0 + Duration::from_millis(100);
        assert!(s.media_guarded(early));
        assert_eq!(s.hover_open_age(early), Some(Duration::from_millis(100)));
        for hit in [Hit::PlayPause, Hit::Prev, Hit::Next] {
            assert_eq!(s.apply(Event::Click(hit.clone()), early), vec![], "{hit:?}");
        }
        // Just before the window ends: still ignored.
        let edge = t0 + MEDIA_HOVER_GUARD - Duration::from_millis(1);
        assert_eq!(s.apply(Event::Click(Hit::PlayPause), edge), vec![]);
        let after = t0 + MEDIA_HOVER_GUARD;
        assert!(!s.media_guarded(after));
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), after),
            vec![Effect::Media(MediaCommand::Pause)]
        );
        // Closing clears it; a later hover open starts a new window.
        s.apply(Event::Hover(false), after);
        assert_eq!(s.hover_open_age(after), None);
        let later = t0 + secs(10);
        s.apply(Event::Hover(true), later);
        assert_eq!(s.apply(Event::Click(Hit::PlayPause), later), vec![]);
    }

    #[test]
    fn the_hover_guard_spares_other_buttons() {
        let t0 = Instant::now();
        let early = t0 + Duration::from_millis(100);
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        open_on(&mut s, Page::Media, t0);
        assert!(s.media_guarded(early));
        assert_eq!(
            s.apply(Event::Click(Hit::LyricsToggle), early),
            vec![Effect::SaveLyrics(true)]
        );
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Click(Hit::CalNext), early);
        assert_ne!(s.cal_anchor(), date(2026, 10, 25), "calendar moved");
    }

    #[test]
    fn a_panel_held_open_without_hover_is_not_guarded() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(false))), t0);
        // An editor opens the panel: no hover open, no guard.
        s.apply(Event::EditorOpen(true), t0);
        assert!(matches!(s.view(), View::Panel(_)));
        assert!(!s.media_guarded(t0));
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), t0),
            vec![Effect::Media(MediaCommand::Play)]
        );
        // Hover arriving while the editor already holds it open: still none.
        s.apply(Event::Hover(true), t0);
        assert!(!s.media_guarded(t0));
        assert_eq!(
            s.apply(Event::Click(Hit::Next), t0),
            vec![Effect::Media(MediaCommand::Next)]
        );
        // Same for the CLI input's focus: it holds the panel once the pointer
        // has gone, and a return of the pointer opens nothing new.
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::CliFocus(true), t0);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Panel(Page::Cli));
        assert!(!s.media_guarded(t0), "cleared once hover went");
        s.apply(Event::Hover(true), t0);
        assert!(!s.media_guarded(t0));
        assert_eq!(
            s.apply(Event::Click(Hit::Prev), t0),
            vec![Effect::Media(MediaCommand::Prev)]
        );
    }

    #[test]
    fn click_body_on_media_does_nothing() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn click_waiting_notice_dismisses_it() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn waiting_is_tracked_per_session() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s2")));
        s.apply(
            Event::ClaudeActivity {
                session_id: "s2".into(),
            },
            t0,
        );
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn repeated_waiting_moves_session_to_newest() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        s.apply(
            Event::ClaudeActivity {
                session_id: "s1".into(),
            },
            t0,
        );
        assert_eq!(s.view(), View::Notice(waiting("s2")));
    }

    #[test]
    fn click_dismisses_only_the_visible_waiting_notice() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn waiting_during_fullscreen_shows_briefly_and_returns_after() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Fullscreen(true), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0 + secs(6));
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn activity_during_fullscreen_clears_brief_waiting_notice() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Fullscreen(true), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(
            Event::ClaudeActivity {
                session_id: "s1".into(),
            },
            t0 + secs(1),
        );
        assert_eq!(s.view(), View::Hidden);
        assert_eq!(s.next_deadline(), None);
        s.apply(Event::Fullscreen(false), t0 + secs(2));
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn next_deadline_is_current_notice_expiry() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(s.next_deadline(), None);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.next_deadline(), Some(t0 + secs(5)));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn hover_during_notice_is_invisible_but_opens_media_page_after() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        open_on(&mut s, Page::Media, t0);
        s.apply(Event::Hover(false), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        s.apply(Event::Hover(true), t0 + secs(1));
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Panel(Page::Media));
    }

    // ---- v2: idle, panel, pages, wheel ----

    #[test]
    fn idle_shows_empty_shape() {
        assert_eq!(st().view(), View::Idle);
    }

    #[test]
    fn fullscreen_hides_idle() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Fullscreen(true), t0);
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn fullscreen_hides_open_panel() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        s.apply(Event::Fullscreen(true), t0);
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn hover_opens_panel_on_last_page() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.view(), View::Panel(Page::Pomodoro));
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Idle);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Pomodoro));
        assert_eq!(s.page(), Page::Pomodoro);
    }

    #[test]
    fn small_wheel_deltas_accumulate_to_one_page() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        for i in 0..3 {
            s.apply(Event::Wheel(-30), t0);
            assert_eq!(s.view(), View::Panel(Page::Calendar), "after {i}");
        }
        s.apply(Event::Wheel(-30), t0);
        assert_eq!(s.view(), View::Panel(Page::Pomodoro));
        s.apply(Event::Wheel(-119), t0);
        assert_eq!(s.view(), View::Panel(Page::Pomodoro));
        s.apply(Event::Wheel(-1), t0);
        assert_eq!(s.view(), View::Panel(Page::Reminders));
    }

    #[test]
    fn wheel_up_goes_to_previous_page() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Notes, t0);
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Reminders));
    }

    #[test]
    fn wheel_clamps_at_ends() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-120 * 6), t0);
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
        // Scrolling past the end is not stored up: one notch back moves one page.
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Cli));
    }

    #[test]
    fn pages_end_with_cli_then_shortcuts() {
        assert_eq!(
            Page::ALL,
            [
                Page::Calendar,
                Page::Pomodoro,
                Page::Reminders,
                Page::Notes,
                Page::Media,
                Page::Cli,
                Page::Shortcuts,
            ]
        );
    }

    #[test]
    fn shortcut_collapses_the_panel_until_the_pointer_leaves() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Shortcuts, t0);
        let effects = s.apply(Event::Click(Hit::Shortcut(Shortcut::Clipboard)), t0);
        assert_eq!(effects, vec![Effect::RunShortcut(Shortcut::Clipboard)]);
        assert_eq!(
            s.view(),
            View::Idle,
            "collapsed with the pointer still over it"
        );
        // Still hovering: nothing reopens it.
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Idle);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Idle);
        // A new visit opens it again, on the same page.
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
    }

    #[test]
    fn shortcut_collapse_beats_an_open_editor() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Shortcuts, t0);
        s.apply(Event::EditorOpen(true), t0);
        s.apply(Event::Click(Hit::Shortcut(Shortcut::Screenshot)), t0);
        assert_eq!(s.view(), View::Idle);
        // The shrunk panel leaves the pointer over transparent pixels: the
        // leave comes at once and must not reopen it for the editor while
        // the screen clip starts.
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Idle, "after the leave");
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Idle, "later");
        // The user comes back: the panel opens, and the editor holds it open
        // again once the pointer leaves.
        s.apply(Event::Hover(true), t0 + secs(6));
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
        s.apply(Event::Hover(false), t0 + secs(7));
        assert_eq!(s.view(), View::Panel(Page::Shortcuts), "pinned");
    }

    #[test]
    fn panel_pinned_by_an_editor_closes_for_a_shortcut_without_hover() {
        // Hovering ended before the click lands (the leave raced the click):
        // the shortcut still shuts the pinned panel until the next visit.
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Shortcuts, t0);
        s.apply(Event::EditorOpen(true), t0);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
        s.apply(Event::Click(Hit::Shortcut(Shortcut::Emoji)), t0);
        assert_eq!(s.view(), View::Idle);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
    }

    #[test]
    fn shortcut_click_outside_the_panel_does_nothing() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::Shortcut(Shortcut::Lock)), t0),
            vec![]
        );
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
    }

    #[test]
    fn clicking_the_terminal_asks_for_the_keyboard_and_focus_locks_the_panel() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        assert_eq!(
            s.apply(Event::Click(Hit::CliInput), t0),
            vec![Effect::FocusCliInput]
        );
        s.apply(Event::CliFocus(true), t0);
        // The pointer leaves: the panel stays while the input is used.
        assert_eq!(s.apply(Event::Hover(false), t0), vec![]);
        assert_eq!(s.view(), View::Panel(Page::Cli));
        s.apply(Event::Tick, t0 + secs(30));
        assert_eq!(s.view(), View::Panel(Page::Cli));
        // Esc or a click elsewhere: the island lets go, the panel closes.
        assert_eq!(s.apply(Event::CliFocus(false), t0), vec![]);
        assert_eq!(s.view(), View::Idle);
        // Output arriving changes nothing in the reducer.
        let output = crate::cli::CliEvent::Output {
            run: 1,
            lines: vec!["x".into()],
        };
        assert_eq!(s.apply(Event::Cli(output), t0), vec![]);
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn cli_buttons_act_only_on_the_cli_page() {
        let t0 = Instant::now();
        let mut s = st();
        for hit in [Hit::CliInput, Hit::CliStop, Hit::CliExpand] {
            assert_eq!(s.apply(Event::Click(hit.clone()), t0), vec![], "{hit:?}");
        }
        open_on(&mut s, Page::Notes, t0);
        for hit in [Hit::CliInput, Hit::CliStop, Hit::CliExpand] {
            assert_eq!(s.apply(Event::Click(hit.clone()), t0), vec![], "{hit:?}");
        }
        assert!(!s.cli_tall());
        open_on(&mut s, Page::Cli, t0);
        assert_eq!(
            s.apply(Event::Click(Hit::CliStop), t0),
            vec![Effect::StopCli]
        );
    }

    #[test]
    fn leaving_the_cli_page_releases_the_keyboard() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::CliFocus(true), t0);
        // The wheel over anything but the output still pages.
        let effects = s.apply(Event::Wheel(-120), t0);
        assert_eq!(effects, vec![Effect::ReleaseCliFocus]);
        assert!(!s.cli_focused());
        assert_eq!(s.view(), View::Panel(Page::Shortcuts));
    }

    #[test]
    fn a_shortcut_collapse_releases_the_keyboard() {
        // The input only has the keyboard on the CLI page, so a collapse
        // needs a stale focus flag; the reducer still lets go at once.
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Shortcuts, t0);
        s.cli_focused = true;
        let effects = s.apply(Event::Click(Hit::Shortcut(Shortcut::Emoji)), t0);
        assert_eq!(
            effects,
            vec![
                Effect::RunShortcut(Shortcut::Emoji),
                Effect::ReleaseCliFocus
            ]
        );
        assert_eq!(s.view(), View::Idle);
        assert!(!s.cli_focused());
    }

    fn pick(path: &str) -> Hit {
        Hit::CliFolderPick {
            path: path.into(),
            favorite: false,
        }
    }

    #[test]
    fn the_folder_chip_toggles_the_menu_without_touching_the_keyboard() {
        let t0 = Instant::now();
        let mut s = st();
        // Off the CLI page the chip does nothing.
        open_on(&mut s, Page::Notes, t0);
        assert_eq!(s.apply(Event::Click(Hit::CliFolder), t0), vec![]);
        assert!(!s.cli_menu());
        open_on(&mut s, Page::Cli, t0);
        assert_eq!(s.apply(Event::Click(Hit::CliFolder), t0), vec![]);
        assert!(s.cli_menu());
        assert_eq!(s.apply(Event::Click(Hit::CliFolder), t0), vec![]);
        assert!(!s.cli_menu(), "the chip again closes it");
        // While the input has the keyboard, the menu leaves it alone.
        s.apply(Event::CliFocus(true), t0);
        assert_eq!(s.apply(Event::Click(Hit::CliFolder), t0), vec![]);
        assert!(s.cli_menu() && s.cli_focused());
        // Starring keeps the menu open.
        let star = Hit::CliFolderStar {
            path: r"D:\a".into(),
            on: true,
        };
        assert_eq!(
            s.apply(Event::Click(star), t0),
            vec![Effect::CliFavorite {
                path: r"D:\a".into(),
                on: true
            }]
        );
        assert!(s.cli_menu());
        // A pick closes it and moves the CLI; focus stays as it was.
        assert_eq!(
            s.apply(Event::Click(pick(r"D:\b")), t0),
            vec![Effect::CliChangeFolder(r"D:\b".into())]
        );
        assert!(!s.cli_menu() && s.cli_focused());
        // A pick without the menu (a stale click) does nothing.
        assert_eq!(s.apply(Event::Click(pick(r"D:\b")), t0), vec![]);
        // The backdrop closes it, as Esc does (`App` sends the same click).
        s.apply(Event::Click(Hit::CliFolder), t0);
        assert_eq!(s.apply(Event::Click(Hit::CliMenuDismiss), t0), vec![]);
        assert!(!s.cli_menu() && s.cli_focused());
    }

    #[test]
    fn the_folder_menu_closes_with_its_page_or_the_panel() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::Click(Hit::CliFolder), t0);
        assert!(s.cli_menu());
        // A page change.
        s.apply(Event::Wheel(-120), t0);
        assert!(!s.cli_menu());
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::Click(Hit::CliFolder), t0);
        // The panel closing; it reopens without the menu.
        assert_eq!(s.apply(Event::Hover(false), t0), vec![]);
        assert!(!s.cli_menu());
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Cli));
        assert!(!s.cli_menu());
        // Explorer results reach `App` only.
        let read = Event::ExplorerFolders {
            request: 1,
            folders: vec![r"D:\x".into()],
        };
        assert_eq!(s.apply(read, t0), vec![]);
    }

    #[test]
    fn cli_output_scrolls_and_clamps() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        let scroll = |s: &mut IslandState, lines| {
            s.apply(Event::CliScroll { lines, max: 30 }, t0);
            s.cli_scroll()
        };
        assert_eq!(scroll(&mut s, 3), 3, "wheel up over the output");
        assert_eq!(s.view(), View::Panel(Page::Cli), "no page change");
        assert_eq!(scroll(&mut s, -1), 2);
        assert_eq!(scroll(&mut s, -10), 0, "never below the newest");
        // PageUp / PageDown move a page of lines.
        assert_eq!(scroll(&mut s, 8), 8);
        assert_eq!(scroll(&mut s, 100), 30, "never past the oldest");
        assert_eq!(scroll(&mut s, -8), 22);
        // New output keeps a scrolled view on its lines...
        s.apply(Event::CliOutput(4), t0);
        assert_eq!(s.cli_scroll(), 26);
        // ...and one at the bottom on the newest.
        scroll(&mut s, -100);
        s.apply(Event::CliOutput(4), t0);
        assert_eq!(s.cli_scroll(), 0);
        // Closing the panel starts it at the bottom again.
        scroll(&mut s, 5);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.cli_scroll(), 0);
        // Scrolling does nothing off the CLI page.
        open_on(&mut s, Page::Notes, t0);
        assert_eq!(scroll(&mut s, 5), 0);
    }

    #[test]
    fn wheel_elsewhere_still_pages_on_the_cli_page() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Media));
    }

    #[test]
    fn the_enlarged_panel_goes_back_to_small_when_it_closes() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Cli, t0);
        s.apply(Event::Click(Hit::CliExpand), t0);
        assert!(s.cli_tall());
        // Another page and back: still enlarged for the CLI.
        s.apply(Event::Wheel(-120), t0);
        s.apply(Event::Wheel(120), t0);
        assert!(s.cli_tall());
        s.apply(Event::Click(Hit::CliExpand), t0);
        assert!(!s.cli_tall(), "the button shrinks it again");
        s.apply(Event::Click(Hit::CliExpand), t0);
        // Focused, the pointer leaving keeps it; letting go closes it.
        s.apply(Event::CliFocus(true), t0);
        s.apply(Event::Hover(false), t0);
        assert!(s.cli_tall());
        s.apply(Event::CliFocus(false), t0);
        assert!(!s.cli_tall());
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Cli));
        assert!(!s.cli_tall());
    }

    #[test]
    fn wheel_ignored_when_panel_closed() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Wheel(-120), t0);
        s.apply(Event::Wheel(-60), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-60), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
    }

    #[test]
    fn wheel_leftover_resets_when_panel_closes() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        s.apply(Event::Wheel(-90), t0);
        s.apply(Event::Hover(false), t0);
        s.apply(Event::Hover(true), t0);
        s.apply(Event::Wheel(-90), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
    }

    #[test]
    fn temporary_notice_blocks_panel_then_opens_if_still_hovered() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(app_notice("k1")), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Panel(Page::Calendar));
    }

    #[test]
    fn waiting_does_not_block_panel_and_shows_strip() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.waiting_strip(), None);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        assert_eq!(s.waiting_strip(), Some(&waiting("s1")));
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        assert_eq!(s.waiting_strip(), None);
    }

    #[test]
    fn click_inside_panel_keeps_waiting() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.waiting_strip(), Some(&waiting("s1")));
    }

    #[test]
    fn clicking_strip_dismisses_waiting() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.waiting_strip(), Some(&waiting("s2")));
        assert_eq!(s.apply(Event::Click(Hit::Strip), t0), vec![]);
        assert_eq!(s.view(), View::Panel(Page::Calendar), "panel stays open");
        assert_eq!(
            s.waiting_strip(),
            Some(&waiting("s1")),
            "older one shows next"
        );
        s.apply(Event::Click(Hit::Strip), t0);
        assert_eq!(s.waiting_strip(), None);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn editor_keeps_panel_open() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        s.apply(Event::EditorOpen(true), t0);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::EditorOpen(false), t0);
        assert_eq!(s.view(), View::Idle);
    }

    // ---- v2: pomodoro ----

    #[test]
    fn priority_pomodoro_over_media() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        assert_eq!(s.view(), View::PomodoroCompact);
        s.apply(Event::Click(Hit::PomoStartPause), t0 + secs(10));
        assert!(matches!(s.pomodoro().status(), PomoStatus::Paused { .. }));
        assert_eq!(s.view(), View::PomodoroCompact, "paused still counts");
        s.apply(Event::Click(Hit::PomoReset), t0 + secs(20));
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn waiting_outranks_pomodoro() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn phase_done_queues_notice_and_sound() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        assert_eq!(s.next_deadline(), Some(t0 + mins(25)));
        let effects = s.apply(Event::Tick, t0 + mins(25));
        assert!(effects.contains(&Effect::PlaySound), "{effects:?}");
        let view = s.view();
        assert_eq!(
            notice_title(&view),
            Some((
                &NoticeKind::Pomodoro,
                "Fokus selesai",
                "Istirahat 5 menit · klik untuk mulai"
            ))
        );
        let View::Notice(n) = &view else { panic!() };
        assert_eq!(
            (n.source.as_str(), n.key.as_str()),
            ("Pomodoro", "pomodoro")
        );
        s.apply(Event::Tick, t0 + mins(25) + secs(5));
        assert_eq!(s.view(), View::PomodoroCompact);
        assert_eq!(s.pomodoro().status(), PomoStatus::Ready);
    }

    #[test]
    fn break_done_notice_names_next_focus_session() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoSkip), t0);
        let effects = s.apply(Event::Click(Hit::PomoSkip), t0 + secs(1));
        assert_eq!(effects, vec![Effect::PlaySound]);
        assert_eq!(
            notice_title(&s.view()),
            Some((
                &NoticeKind::Pomodoro,
                "Istirahat selesai",
                "Fokus sesi 2 dari 4 · klik untuk mulai"
            ))
        );
    }

    #[test]
    fn last_focus_announces_long_break() {
        let t0 = Instant::now();
        let mut s = IslandState::new(
            date(2026, 10, 25),
            PomoSettings {
                sessions: 1,
                ..PomoSettings::default()
            },
        );
        s.apply(Event::Click(Hit::PomoSkip), t0);
        assert_eq!(
            notice_title(&s.view()).map(|t| t.2),
            Some("Istirahat 15 menit · klik untuk mulai")
        );
    }

    #[test]
    fn session_label_never_exceeds_session_count() {
        let t0 = Instant::now();
        let mut s = st();
        // Focus 1 → short break → focus 2 → short break, then lower sessions to 1.
        s.apply(Event::Click(Hit::PomoSkip), t0);
        s.apply(Event::Click(Hit::PomoSkip), t0);
        s.apply(Event::Click(Hit::PomoSkip), t0);
        for _ in 0..3 {
            s.apply(Event::Click(Hit::PomoAdjust(PomoField::Sessions, -1)), t0);
        }
        assert_eq!(s.pomodoro().settings().sessions, 1);
        s.apply(Event::Click(Hit::PomoSkip), t0);
        assert_eq!(
            notice_title(&s.view()).map(|t| t.2),
            Some("Fokus sesi 1 dari 1 · klik untuk mulai")
        );
    }

    #[test]
    fn pomodoro_notices_follow_the_language() {
        i18n::set_lang(i18n::Lang::En);
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoSkip), t0);
        assert_eq!(
            notice_title(&s.view()),
            Some((
                &NoticeKind::Pomodoro,
                "Focus done",
                "5-minute break · click to start"
            ))
        );
        s.apply(Event::Click(Hit::PomoSkip), t0 + secs(1));
        assert_eq!(
            notice_title(&s.view()).map(|t| (t.1, t.2)),
            Some(("Break done", "Focus session 2 of 4 · click to start"))
        );
    }

    #[test]
    fn repeated_skips_replace_the_pomodoro_notice() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoSkip), t0);
        s.apply(Event::Click(Hit::PomoSkip), t0 + secs(1));
        assert_eq!(
            notice_title(&s.view()).map(|t| t.1),
            Some("Istirahat selesai")
        );
        s.apply(Event::Tick, t0 + secs(6));
        assert_eq!(s.view(), View::PomodoroCompact);
    }

    #[test]
    fn click_ready_compact_starts_next_phase() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        s.apply(Event::Tick, t0 + mins(25));
        s.apply(Event::Tick, t0 + mins(25) + secs(5));
        assert_eq!(s.view(), View::PomodoroCompact);
        let at = t0 + mins(26);
        assert_eq!(s.apply(Event::Click(Hit::Body), at), vec![]);
        assert_eq!(s.pomodoro().phase(), Phase::ShortBreak);
        assert_eq!(
            s.pomodoro().status(),
            PomoStatus::Running {
                ends_at: at + mins(5)
            }
        );
    }

    #[test]
    fn click_running_compact_does_not_pause() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        s.apply(Event::Click(Hit::Body), t0 + secs(30));
        assert!(matches!(s.pomodoro().status(), PomoStatus::Running { .. }));
    }

    #[test]
    fn click_pomodoro_notice_starts_next_phase() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        s.apply(Event::Tick, t0 + mins(25));
        let at = t0 + mins(25) + secs(2);
        assert_eq!(s.apply(Event::Click(Hit::Body), at), vec![]);
        assert_eq!(
            s.pomodoro().status(),
            PomoStatus::Running {
                ends_at: at + mins(5)
            }
        );
        assert_eq!(s.view(), View::PomodoroCompact);
    }

    #[test]
    fn input_after_deadline_finishes_phase_first() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        // The deadline passed but its Tick has not arrived yet.
        let effects = s.apply(Event::Click(Hit::PomoStartPause), t0 + mins(25) + secs(1));
        assert!(effects.contains(&Effect::PlaySound));
        assert_ne!(
            s.pomodoro().status(),
            PomoStatus::Paused {
                remaining: Duration::ZERO
            }
        );
        assert_eq!(s.pomodoro().phase(), Phase::ShortBreak);
    }

    #[test]
    fn next_deadline_includes_pomodoro_end() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        s.apply(Event::Notice(app_notice("k1")), t0 + secs(1));
        assert_eq!(s.next_deadline(), Some(t0 + secs(6)));
        s.apply(Event::Tick, t0 + secs(6));
        assert_eq!(s.next_deadline(), Some(t0 + mins(25)));
    }

    #[test]
    fn adjust_emits_save() {
        let t0 = Instant::now();
        let mut s = st();
        let effects = s.apply(Event::Click(Hit::PomoAdjust(PomoField::Focus, 1)), t0);
        assert_eq!(
            effects,
            vec![Effect::SavePomodoro(PomoSettings {
                focus_min: 30,
                ..PomoSettings::default()
            })]
        );
    }

    #[test]
    fn adjust_without_change_saves_nothing() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::PomoStartPause), t0);
        assert_eq!(
            s.apply(Event::Click(Hit::PomoAdjust(PomoField::Focus, 1)), t0),
            vec![]
        );
    }

    #[test]
    fn save_sends_clamped_settings() {
        let t0 = Instant::now();
        let mut s = IslandState::new(
            date(2026, 10, 25),
            PomoSettings {
                focus_min: 0,
                ..PomoSettings::default()
            },
        );
        assert_eq!(
            s.apply(Event::Click(Hit::PomoAdjust(PomoField::Sessions, 1)), t0),
            vec![Effect::SavePomodoro(PomoSettings {
                focus_min: 5,
                sessions: 5,
                ..PomoSettings::default()
            })]
        );
    }

    // ---- v2: calendar ----

    fn refresh(from: NaiveDate, to: NaiveDate) -> Effect {
        Effect::RefreshCalendar { from, to }
    }

    #[test]
    fn calendar_starts_on_today_month() {
        let s = st();
        assert_eq!(s.cal_view(), CalView::Month);
        assert_eq!(s.cal_anchor(), date(2026, 10, 25));
        assert_eq!(s.cal_selected(), date(2026, 10, 25));
        // Oct 1 2026 is a Thursday.
        assert_eq!(s.cal_range(), (date(2026, 9, 28), date(2026, 11, 9)));
    }

    #[test]
    fn calendar_nav_moves_anchor_and_requests_range() {
        let t0 = Instant::now();
        let mut s = st();
        let effects = s.apply(Event::Click(Hit::CalNext), t0);
        assert_eq!(s.cal_anchor(), date(2026, 11, 25));
        // Nov 1 2026 is a Sunday.
        let from = date(2026, 10, 26);
        assert_eq!(effects, vec![refresh(from, from + chrono::Days::new(42))]);
        s.apply(Event::Click(Hit::CalPrev), t0);
        s.apply(Event::Click(Hit::CalPrev), t0);
        assert_eq!(s.cal_anchor(), date(2026, 9, 25));
    }

    #[test]
    fn month_nav_clamps_to_month_end() {
        let t0 = Instant::now();
        let mut s = IslandState::new(date(2026, 1, 31), PomoSettings::default());
        s.apply(Event::Click(Hit::CalNext), t0);
        assert_eq!(s.cal_anchor(), date(2026, 2, 28));
    }

    #[test]
    fn week_and_day_nav_and_ranges() {
        let t0 = Instant::now();
        let mut s = st();
        let effects = s.apply(Event::Click(Hit::CalViewSel(CalView::Week)), t0);
        // Oct 25 2026 is a Sunday: its week starts Monday Oct 19.
        assert_eq!(
            effects,
            vec![refresh(date(2026, 10, 19), date(2026, 10, 26))]
        );
        s.apply(Event::Click(Hit::CalNext), t0);
        assert_eq!(s.cal_range(), (date(2026, 10, 26), date(2026, 11, 2)));
        s.apply(Event::Click(Hit::CalViewSel(CalView::Day)), t0);
        assert_eq!(
            s.cal_anchor(),
            date(2026, 10, 25),
            "day view opens on selected"
        );
        let effects = s.apply(Event::Click(Hit::CalPrev), t0);
        assert_eq!(
            effects,
            vec![refresh(date(2026, 10, 24), date(2026, 10, 25))]
        );
        assert_eq!(s.cal_selected(), date(2026, 10, 24));
    }

    #[test]
    fn same_view_selection_requests_nothing() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::CalViewSel(CalView::Month)), t0),
            vec![]
        );
    }

    #[test]
    fn cal_date_selects_without_moving_month() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::CalDate(date(2026, 11, 3))), t0),
            vec![]
        );
        assert_eq!(s.cal_selected(), date(2026, 11, 3));
        assert_eq!(s.cal_anchor(), date(2026, 10, 25));
        s.apply(Event::Click(Hit::CalViewSel(CalView::Day)), t0);
        assert_eq!(s.cal_anchor(), date(2026, 11, 3));
        s.apply(Event::Click(Hit::CalDate(date(2026, 11, 5))), t0);
        assert_eq!(
            s.cal_anchor(),
            date(2026, 11, 5),
            "day view follows selection"
        );
    }

    #[test]
    fn cal_add_opens_editor_on_selected_date() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Click(Hit::CalDate(date(2026, 10, 30))), t0);
        assert_eq!(
            s.apply(Event::Click(Hit::CalAdd), t0),
            vec![Effect::OpenEventEditor {
                event_id: None,
                date: date(2026, 10, 30)
            }]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::CalEvent("ev1".into())), t0),
            vec![Effect::OpenEventEditor {
                event_id: Some("ev1".into()),
                date: date(2026, 10, 30)
            }]
        );
    }

    #[test]
    fn cal_connect_starts_google_login() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::CalConnect), t0),
            vec![Effect::ConnectGoogle]
        );
    }

    #[test]
    fn opening_panel_on_calendar_requests_refresh() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Hover(true), t0),
            vec![refresh(date(2026, 9, 28), date(2026, 11, 9))]
        );
        // Already showing: no repeat.
        assert_eq!(s.apply(Event::Tick, t0), vec![]);
        s.apply(Event::Wheel(-120), t0);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.apply(Event::Hover(true), t0), vec![], "opens on Pomodoro");
        assert_eq!(
            s.apply(Event::Wheel(120), t0),
            vec![refresh(date(2026, 9, 28), date(2026, 11, 9))],
            "scrolling onto the calendar shows it too"
        );
    }

    #[test]
    fn cal_more_switches_to_day_view() {
        let t0 = Instant::now();
        let mut s = st();
        let effects = s.apply(Event::Click(Hit::CalMore(date(2026, 10, 14))), t0);
        assert_eq!(s.cal_view(), CalView::Day);
        assert_eq!(s.cal_anchor(), date(2026, 10, 14));
        assert_eq!(s.cal_selected(), date(2026, 10, 14));
        assert_eq!(
            effects,
            vec![refresh(date(2026, 10, 14), date(2026, 10, 15))]
        );
    }

    #[test]
    fn cal_more_in_day_view_pages_through_that_day() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Calendar, t0);
        let d = date(2026, 10, 14);
        s.apply(Event::Click(Hit::CalMore(d)), t0);
        assert_eq!((s.cal_view(), s.day_page()), (CalView::Day, 0));
        // Already on that day: next chunk, no new fetch.
        assert_eq!(s.apply(Event::Click(Hit::CalMore(d)), t0), vec![]);
        s.apply(Event::Click(Hit::CalMore(d)), t0);
        assert_eq!(s.day_page(), 2);
        assert_eq!(s.cal_anchor(), d);
    }

    #[test]
    fn day_page_resets_on_navigation_view_change_and_close() {
        let t0 = Instant::now();
        let d = date(2026, 10, 25);
        let paged = |s: &mut IslandState| {
            s.apply(Event::Click(Hit::CalViewSel(CalView::Day)), t0);
            s.apply(Event::Click(Hit::CalMore(d)), t0);
            assert_eq!(s.day_page(), 1);
        };
        let mut s = st();
        open_on(&mut s, Page::Calendar, t0);
        paged(&mut s);
        s.apply(Event::Click(Hit::CalNext), t0);
        assert_eq!(s.day_page(), 0, "next day");

        s.apply(Event::Click(Hit::CalPrev), t0);
        paged(&mut s);
        s.apply(Event::Click(Hit::CalViewSel(CalView::Week)), t0);
        assert_eq!(s.day_page(), 0, "view change");

        paged(&mut s);
        s.apply(Event::Click(Hit::CalDate(date(2026, 10, 26))), t0);
        assert_eq!(s.day_page(), 0, "another date");

        s.apply(Event::Click(Hit::CalDate(d)), t0);
        paged(&mut s);
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.day_page(), 0, "page left the calendar");

        s.apply(Event::Wheel(120), t0);
        paged(&mut s);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.day_page(), 0, "panel closed");
    }

    #[test]
    fn today_rollover_moves_dates_that_were_today() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Today(date(2026, 10, 26)), t0);
        assert_eq!(s.cal_anchor(), date(2026, 10, 26));
        assert_eq!(s.cal_selected(), date(2026, 10, 26));

        let mut s = st();
        s.apply(Event::Click(Hit::CalDate(date(2026, 10, 20))), t0);
        s.apply(Event::Click(Hit::CalNext), t0);
        s.apply(Event::Today(date(2026, 10, 26)), t0);
        assert_eq!(s.cal_anchor(), date(2026, 11, 25));
        assert_eq!(s.cal_selected(), date(2026, 10, 20));
    }

    // ---- v2: notes ----

    #[test]
    fn note_hits_open_editor() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::NoteNew), t0),
            vec![Effect::OpenNoteEditor(None)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::NoteOpen("n1".into())), t0),
            vec![Effect::OpenNoteEditor(Some("n1".into()))]
        );
    }

    // ---- v2.2: reminders ----

    fn reminder(id: &str) -> Notice {
        Notice {
            key: format!("reminder-{id}"),
            source: "Pengingat".into(),
            title: format!("Minum obat {id}"),
            body: "08.00".into(),
            kind: NoticeKind::Reminder { id: id.into() },
        }
    }

    #[test]
    fn reminder_is_sticky_and_click_marks_it_done() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        assert_eq!(s.next_deadline(), None, "no expiry");
        s.apply(Event::Tick, t0 + secs(600));
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        // Claude activity has nothing to do with it.
        s.apply(
            Event::ClaudeActivity {
                session_id: "r1".into(),
            },
            t0 + secs(601),
        );
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        assert_eq!(
            s.apply(Event::Click(Hit::Body), t0 + secs(602)),
            vec![Effect::ReminderDone("r1".into())]
        );
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn newest_sticky_notice_shows_first() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        s.apply(Event::Notice(reminder("r2")), t0);
        assert_eq!(s.view(), View::Notice(reminder("r2")));
        assert_eq!(
            s.apply(Event::Click(Hit::Body), t0),
            vec![Effect::ReminderDone("r2".into())]
        );
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        s.apply(Event::Click(Hit::Body), t0);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
        // A newer waiting notice goes above an older reminder.
        s.apply(Event::Notice(reminder("r3")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        assert_eq!(s.view(), View::Notice(waiting("s2")));
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::Notice(reminder("r3")));
    }

    #[test]
    fn same_reminder_again_is_not_shown_twice() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(reminder("r1")), t0);
        s.apply(Event::Notice(reminder("r2")), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        s.apply(Event::Click(Hit::Body), t0);
        assert_eq!(s.view(), View::Notice(reminder("r2")));
        s.apply(Event::Click(Hit::Body), t0);
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn temporary_notice_preempts_reminder_then_it_returns() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(reminder("r1")), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Notice(reminder("r1")));
    }

    #[test]
    fn reminder_during_fullscreen_shows_briefly_and_returns_after() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Fullscreen(true), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        assert_eq!(s.next_deadline(), Some(t0 + NOTICE_DURATION));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0 + secs(6));
        assert_eq!(s.view(), View::Notice(reminder("r1")));
    }

    #[test]
    fn clicking_the_brief_fullscreen_reminder_marks_it_done() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Fullscreen(true), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        assert_eq!(
            s.apply(Event::Click(Hit::Body), t0 + secs(1)),
            vec![Effect::ReminderDone("r1".into())]
        );
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0 + secs(2));
        assert_eq!(s.view(), View::Idle, "not sticky any more");
    }

    #[test]
    fn cleared_reminder_disappears_everywhere() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(reminder("r1")), t0);
        s.apply(Event::Notice(reminder("r2")), t0);
        s.apply(Event::ReminderCleared("r2".into()), t0);
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        s.apply(Event::ReminderCleared("gone".into()), t0);
        assert_eq!(s.view(), View::Notice(reminder("r1")));
        // Also its brief fullscreen copy, showing or queued.
        s.apply(Event::Fullscreen(true), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        s.apply(Event::Notice(reminder("r3")), t0);
        s.apply(Event::ReminderCleared("r3".into()), t0);
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Hidden, "the queued copy went too");
        s.apply(Event::ReminderCleared("r1".into()), t0 + secs(5));
        s.apply(Event::Fullscreen(false), t0 + secs(6));
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn strip_shows_newest_sticky_and_click_marks_reminder_done() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(reminder("r1")), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        assert_eq!(s.waiting_strip(), Some(&reminder("r1")));
        assert_eq!(
            s.apply(Event::Click(Hit::Strip), t0),
            vec![Effect::ReminderDone("r1".into())]
        );
        assert_eq!(s.waiting_strip(), Some(&waiting("s1")));
        assert_eq!(s.apply(Event::Click(Hit::Strip), t0), vec![]);
        assert_eq!(s.waiting_strip(), None);
    }

    #[test]
    fn reminder_hits_open_editor_and_page() {
        let t0 = Instant::now();
        let mut s = st();
        assert_eq!(
            s.apply(Event::Click(Hit::ReminderNew), t0),
            vec![Effect::OpenReminderEditor(None)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::ReminderOpen("r1".into())), t0),
            vec![Effect::OpenReminderEditor(Some("r1".into()))]
        );
        open_on(&mut s, Page::Reminders, t0);
        assert_eq!(s.reminders_page(), 0);
        s.apply(Event::Click(Hit::RemindersMore), t0);
        s.apply(Event::Click(Hit::RemindersMore), t0);
        assert_eq!(s.reminders_page(), 2);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.reminders_page(), 0);
    }

    #[test]
    fn reminder_page_sits_between_pomodoro_and_notes() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Pomodoro, t0);
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.view(), View::Panel(Page::Reminders));
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.view(), View::Panel(Page::Notes));
    }

    #[test]
    fn notes_more_advances_and_resets_on_close() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Notes, t0);
        assert_eq!(s.notes_page(), 0);
        s.apply(Event::Click(Hit::NotesMore), t0);
        s.apply(Event::Click(Hit::NotesMore), t0);
        assert_eq!(s.notes_page(), 2);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.notes_page(), 0);
    }

    // ---- v2.2: lyrics ----

    fn lyric_ready(s: &mut IslandState, t0: Instant) {
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::LyricsSetting(true), t0);
        s.apply(Event::LyricsReady(true), t0);
    }

    #[test]
    fn lyric_view_needs_setting_lyrics_and_playback() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::LyricsReady(true), t0);
        assert_eq!(s.view(), View::MediaCompact, "setting off");
        s.apply(Event::LyricsSetting(true), t0);
        assert_eq!(s.view(), View::MediaLyric);
        s.apply(Event::LyricsReady(false), t0);
        assert_eq!(s.view(), View::MediaCompact, "no synced lyrics");
        s.apply(Event::LyricsReady(true), t0);
        s.apply(Event::Media(Some(media(false))), t0);
        assert_eq!(s.view(), View::MediaCompact, "paused shows the title");
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaLyric, "same track resumes its lyrics");
        s.apply(Event::Media(None), t0);
        assert_eq!(s.view(), View::Idle);
    }

    #[test]
    fn lyric_view_ranks_like_the_compact_media() {
        let t0 = Instant::now();
        let mut s = st();
        lyric_ready(&mut s, t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::MediaLyric);
        s.apply(Event::Click(Hit::PomoStartPause), t0 + secs(5));
        assert_eq!(s.view(), View::PomodoroCompact);
        s.apply(Event::Click(Hit::PomoReset), t0 + secs(6));
        s.apply(Event::Hover(true), t0 + secs(6));
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Hover(false), t0 + secs(6));
        s.apply(Event::Fullscreen(true), t0 + secs(6));
        assert_eq!(s.view(), View::Hidden);
        s.apply(Event::Fullscreen(false), t0 + secs(6));
        assert_eq!(s.view(), View::MediaLyric);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0 + secs(7)), vec![]);
    }

    #[test]
    fn a_new_track_waits_for_its_own_lyrics() {
        let t0 = Instant::now();
        let mut s = st();
        lyric_ready(&mut s, t0);
        let next = MediaInfo {
            title: "Another One".into(),
            ..media(true)
        };
        s.apply(Event::Media(Some(next)), t0);
        assert!(!s.lyrics_ready());
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn lyric_button_flips_the_setting_and_saves() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Media, t0);
        assert_eq!(
            s.apply(Event::Click(Hit::LyricsToggle), t0),
            vec![Effect::SaveLyrics(true)]
        );
        assert!(s.lyrics_on());
        assert_eq!(
            s.apply(Event::Click(Hit::LyricsToggle), t0),
            vec![Effect::SaveLyrics(false)]
        );
        assert!(!s.lyrics_on());
        assert_eq!(s.view(), View::Panel(Page::Media));
    }

    #[test]
    fn timeline_is_kept_and_rebased_on_play_pause() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        let t = Timeline {
            position_ms: 10_000,
            end_ms: 200_000,
            updated: t0,
        };
        s.apply(Event::MediaTimeline(Some(t)), t0);
        assert_eq!(s.timeline(), Some(&t));
        // Paused 3 s later: the position freezes there.
        let paused = t0 + secs(3);
        s.apply(Event::Media(Some(media(false))), paused);
        let frozen = Timeline {
            position_ms: 13_000,
            end_ms: 200_000,
            updated: paused,
        };
        assert_eq!(s.timeline(), Some(&frozen));
        // A minute later it resumes from there.
        let resumed = paused + secs(60);
        s.apply(Event::Media(Some(media(true))), resumed);
        assert_eq!(
            s.timeline(),
            Some(&Timeline {
                updated: resumed,
                ..frozen
            })
        );
        // A fresh timeline from the watcher wins.
        s.apply(Event::MediaTimeline(Some(t)), resumed);
        assert_eq!(s.timeline(), Some(&t));
        s.apply(Event::MediaTimeline(None), resumed);
        assert_eq!(s.timeline(), None);
    }
}
