use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{Days, Months, NaiveDate};

use crate::calendar::range_for;
use crate::model::{
    CalView, Effect, Event, Hit, MediaCommand, MediaInfo, Notice, NoticeKind, Page, View,
};
use crate::pomodoro::{Phase, PhaseDone, PomoSettings, PomoStatus, Pomodoro};

/// How long each temporary notice stays on screen, counted from when it becomes current.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

/// One mouse-wheel notch, as Windows reports it.
const WHEEL_STEP: i32 = 120;

/// Key shared by all pomodoro notices, so a newer one replaces an older one.
const POMODORO_KEY: &str = "pomodoro";

/// Decides what the island shows. Pure logic: OS sources feed it `Event`s and the
/// renderer draws its `View`; the caller performs the returned `Effect`s.
pub struct IslandState {
    media: Option<MediaInfo>,
    queue: VecDeque<Notice>,
    current: Option<(Notice, Instant)>,
    /// At most one `ClaudeWaiting` per session, oldest first; the newest is shown.
    waiting: Vec<Notice>,
    fullscreen: bool,
    hover: bool,
    /// A notes or event editor window is open; it keeps the panel open like hover.
    editor_open: bool,
    pomodoro: Pomodoro,
    /// The panel page, kept across closes so the panel reopens where it was left.
    page: Page,
    /// Wheel delta not yet worth a whole page; cleared when the panel closes.
    wheel: i32,
    notes_page: usize,
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
            queue: VecDeque::new(),
            current: None,
            waiting: Vec::new(),
            fullscreen: false,
            hover: false,
            editor_open: false,
            pomodoro: Pomodoro::new(pomo),
            page: Page::Calendar,
            wheel: 0,
            notes_page: 0,
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
        let mut effects = Vec::new();
        // Finish a phase whose end has passed before handling the input, so a
        // click that beats the deadline's Tick never pauses the timer at 0:00.
        if let Some(done) = self.pomodoro.tick(now) {
            self.phase_done(done, &mut effects);
        }
        self.advance(now);
        match event {
            Event::Media(media) => self.media = media,
            Event::Notice(notice) => match &notice.kind {
                NoticeKind::ClaudeWaiting { session_id } => {
                    self.clear_waiting(session_id);
                    // The sticky slot is hidden in fullscreen, so show it briefly like
                    // any other new notice; it stays in `waiting` for afterwards.
                    if self.fullscreen {
                        self.queue.push_back(notice.clone());
                    }
                    self.waiting.push(notice);
                }
                NoticeKind::ClaudeDone { session_id } => {
                    self.clear_waiting(session_id);
                    self.queue.push_back(notice);
                }
                NoticeKind::App { .. } | NoticeKind::BatteryLow | NoticeKind::Pomodoro => {
                    self.queue.push_back(notice)
                }
            },
            Event::ClaudeActivity { session_id } => self.clear_waiting(&session_id),
            Event::Fullscreen(on) => self.fullscreen = on,
            Event::Hover(on) => self.hover = on,
            Event::EditorOpen(on) => self.editor_open = on,
            Event::Wheel(delta) => self.wheel(delta),
            Event::Today(date) => self.set_today(date),
            Event::Click(hit) => self.click(hit, now, &mut effects),
            Event::Calendar(_) | Event::Tick => {}
        }
        // A new notice or a dismissal may have left the slot free for the next queued one.
        self.advance(now);
        if panel_was_open && !self.panel_open() {
            self.wheel = 0;
            self.notes_page = 0;
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
        // Claude waiting does not block the panel; it shows as a strip inside it.
        if self.panel_open() {
            return View::Panel(self.page);
        }
        if let Some(notice) = self.waiting.last() {
            return View::Notice(notice.clone());
        }
        if self.pomodoro.is_active() {
            return View::PomodoroCompact;
        }
        if self.media.is_some() {
            return View::MediaCompact;
        }
        View::Idle
    }

    pub fn media(&self) -> Option<&MediaInfo> {
        self.media.as_ref()
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
        self.hover || self.editor_open
    }

    fn click(&mut self, hit: Hit, now: Instant, effects: &mut Vec<Effect>) {
        match hit {
            Hit::Body => match self.view() {
                View::Notice(_) => effects.extend(self.dismiss_visible_notice(now)),
                View::PomodoroCompact if self.pomodoro.status() == PomoStatus::Ready => {
                    self.pomodoro.start_pause(now)
                }
                _ => {}
            },
            Hit::Prev => effects.push(Effect::Media(MediaCommand::Prev)),
            Hit::PlayPause => effects.push(Effect::Media(MediaCommand::PlayPause)),
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
            // The strip shows the newest waiting notice; clicking it dismisses that one.
            Hit::Strip => {
                if matches!(self.view(), View::Panel(_))
                    && let Some(session) = self.waiting.last().and_then(waiting_session)
                {
                    let session = session.to_owned();
                    self.clear_waiting(&session);
                }
            }
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
        let (title, body) = match (done.finished, done.next) {
            (Phase::Focus, next) => {
                let min = match next {
                    Phase::LongBreak => settings.long_break_min,
                    _ => settings.short_break_min,
                };
                (
                    "Fokus selesai",
                    format!("Istirahat {min} menit · klik untuk mulai"),
                )
            }
            _ => {
                // Lowering the session count while Ready can leave the
                // session number above it; never show "5 dari 4".
                let k = self.pomodoro.session().min(settings.sessions);
                (
                    "Istirahat selesai",
                    format!(
                        "Fokus sesi {k} dari {} · klik untuk mulai",
                        settings.sessions
                    ),
                )
            }
        };
        self.queue.retain(|n| n.kind != NoticeKind::Pomodoro);
        if matches!(&self.current, Some((n, _)) if n.kind == NoticeKind::Pomodoro) {
            self.current = None;
        }
        self.queue.push_back(Notice {
            key: POMODORO_KEY.into(),
            source: "Pomodoro".into(),
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
            self.current = Some((next, now + NOTICE_DURATION));
        }
    }

    /// Removes session's waiting notice, including a brief fullscreen copy that is
    /// queued or showing, so a resolved wait is never displayed.
    fn clear_waiting(&mut self, session: &str) {
        let is_session = |n: &Notice| waiting_session(n) == Some(session);
        self.waiting.retain(|n| !is_session(n));
        self.queue.retain(|n| !is_session(n));
        if matches!(&self.current, Some((n, _)) if is_session(n)) {
            self.current = None;
        }
    }

    /// Dismisses whichever notice the view is showing. Only called while the
    /// view is a notice: the current one, else the newest waiting one.
    fn dismiss_visible_notice(&mut self, now: Instant) -> Option<Effect> {
        let Some((notice, _)) = self.current.take() else {
            self.waiting.pop();
            return None;
        };
        match notice.kind {
            NoticeKind::App { app_id: Some(id) } => Some(Effect::OpenApp(id)),
            NoticeKind::ClaudeWaiting { session_id } => {
                self.clear_waiting(&session_id);
                None
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

    /// The Claude waiting notice to show at the top of the open panel.
    pub fn waiting_strip(&self) -> Option<&Notice> {
        match self.view() {
            View::Panel(_) => self.waiting.last(),
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
}

fn waiting_session(notice: &Notice) -> Option<&str> {
    match &notice.kind {
        NoticeKind::ClaudeWaiting { session_id } => Some(session_id),
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

    // ---- v1 behaviour, adapted to v2 views ----

    #[test]
    fn media_shows_compact_and_hover_opens_panel() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-360), t0);
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
        assert_eq!(
            s.apply(Event::Click(Hit::Next), t0),
            vec![Effect::Media(MediaCommand::Next)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::Prev), t0),
            vec![Effect::Media(MediaCommand::Prev)]
        );
        assert_eq!(
            s.apply(Event::Click(Hit::PlayPause), t0),
            vec![Effect::Media(MediaCommand::PlayPause)]
        );
        assert_eq!(s.view(), View::Panel(Page::Media));
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
        assert_eq!(s.view(), View::Panel(Page::Notes));
    }

    #[test]
    fn wheel_up_goes_to_previous_page() {
        let t0 = Instant::now();
        let mut s = st();
        open_on(&mut s, Page::Notes, t0);
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Pomodoro));
    }

    #[test]
    fn wheel_clamps_at_ends() {
        let t0 = Instant::now();
        let mut s = st();
        s.apply(Event::Hover(true), t0);
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Calendar));
        s.apply(Event::Wheel(-120 * 3), t0);
        assert_eq!(s.view(), View::Panel(Page::Media));
        s.apply(Event::Wheel(-120), t0);
        assert_eq!(s.view(), View::Panel(Page::Media));
        // Scrolling past the end is not stored up: one notch back moves one page.
        s.apply(Event::Wheel(120), t0);
        assert_eq!(s.view(), View::Panel(Page::Notes));
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
}
