// Nothing outside tests uses the reducer yet; removed when app.rs wires it (Task 10).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::model::{Effect, Event, Hit, MediaCommand, MediaInfo, Notice, NoticeKind, View};

/// How long each temporary notice stays on screen, counted from when it becomes current.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

/// Decides what the island shows. Pure logic: OS sources feed it `Event`s and the
/// renderer draws its `View`; the caller performs the returned `Effect`s.
#[derive(Default)]
pub struct IslandState {
    media: Option<MediaInfo>,
    queue: VecDeque<Notice>,
    current: Option<(Notice, Instant)>,
    /// At most one `ClaudeWaiting` per session, oldest first; the newest is shown.
    waiting: Vec<Notice>,
    fullscreen: bool,
    hover: bool,
}

impl IslandState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        self.advance(now);
        let mut effects = Vec::new();
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
                NoticeKind::App { .. } | NoticeKind::BatteryLow => self.queue.push_back(notice),
            },
            Event::ClaudeActivity { session_id } => self.clear_waiting(&session_id),
            Event::Fullscreen(on) => self.fullscreen = on,
            Event::Hover(on) => self.hover = on,
            Event::Click(Hit::Body) => {
                if let Some(effect) = self.dismiss_visible_notice() {
                    effects.push(effect);
                }
            }
            Event::Click(Hit::Prev) => effects.push(Effect::Media(MediaCommand::Prev)),
            Event::Click(Hit::PlayPause) => effects.push(Effect::Media(MediaCommand::PlayPause)),
            Event::Click(Hit::Next) => effects.push(Effect::Media(MediaCommand::Next)),
            Event::Tick => {}
        }
        // A new notice or a dismissal may have left the slot free for the next queued one.
        self.advance(now);
        effects
    }

    pub fn view(&self) -> View {
        if let Some((notice, _)) = &self.current {
            return View::Notice(notice.clone());
        }
        if self.fullscreen {
            return View::Hidden;
        }
        if let Some(notice) = self.waiting.last() {
            return View::Notice(notice.clone());
        }
        match (&self.media, self.hover) {
            (Some(_), true) => View::MediaExpanded,
            (Some(_), false) => View::MediaCompact,
            (None, _) => View::Hidden,
        }
    }

    pub fn media(&self) -> Option<&MediaInfo> {
        self.media.as_ref()
    }

    /// When the current notice expires; the caller should deliver a `Tick` then.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.current.as_ref().map(|(_, expires)| *expires)
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

    /// Dismisses whichever notice the view is showing, if any.
    fn dismiss_visible_notice(&mut self) -> Option<Effect> {
        if let Some((notice, _)) = self.current.take() {
            return match notice.kind {
                NoticeKind::App { app_id: Some(id) } => Some(Effect::OpenApp(id)),
                NoticeKind::ClaudeWaiting { session_id } => {
                    self.clear_waiting(&session_id);
                    None
                }
                _ => None,
            };
        }
        if !self.fullscreen {
            self.waiting.pop();
        }
        None
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
    use std::time::{Duration, Instant};

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
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

    #[test]
    fn starts_hidden() {
        assert_eq!(IslandState::new().view(), View::Hidden);
    }

    #[test]
    fn media_shows_compact_and_hover_expands() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::MediaExpanded);
        s.apply(Event::Hover(false), t0);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn paused_media_still_shows() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(false))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        assert_eq!(s.media(), Some(&media(false)));
    }

    #[test]
    fn media_gone_hides_island() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.view(), View::MediaCompact);
        s.apply(Event::Media(None), t0);
        assert_eq!(s.view(), View::Hidden);
        assert_eq!(s.media(), None);
    }

    #[test]
    fn notice_shows_then_returns_to_media() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
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
        let mut s = IslandState::new();
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
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn fullscreen_hides_media_but_not_new_notice() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
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
        let mut s = IslandState::new();
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
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn claude_done_clears_waiting_of_same_session() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(done("s1")), t0);
        assert_eq!(s.view(), View::Notice(done("s1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn temporary_notice_preempts_waiting_then_waiting_returns() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn click_app_notice_opens_app_and_dismisses() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Notice(app_notice("k1")), t0);
        let effects = s.apply(Event::Click(Hit::Body), t0 + secs(1));
        assert_eq!(
            effects,
            vec![Effect::OpenApp("WhatsApp.Desktop!App".into())]
        );
        assert_eq!(s.view(), View::Hidden);
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn click_media_buttons_emit_commands() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Hover(true), t0);
        assert_eq!(s.view(), View::MediaExpanded);
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
        assert_eq!(s.view(), View::MediaExpanded);
    }

    #[test]
    fn click_body_on_media_does_nothing() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn click_waiting_notice_dismisses_it() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Notice(waiting("s1")), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::MediaCompact);
    }

    #[test]
    fn waiting_is_tracked_per_session() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
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
        let mut s = IslandState::new();
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
        let mut s = IslandState::new();
        s.apply(Event::Notice(waiting("s1")), t0);
        s.apply(Event::Notice(waiting("s2")), t0);
        assert_eq!(s.apply(Event::Click(Hit::Body), t0), vec![]);
        assert_eq!(s.view(), View::Notice(waiting("s1")));
    }

    #[test]
    fn waiting_during_fullscreen_shows_briefly_and_returns_after() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
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
        let mut s = IslandState::new();
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
        assert_eq!(s.view(), View::Hidden);
    }

    #[test]
    fn next_deadline_is_current_notice_expiry() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        assert_eq!(s.next_deadline(), None);
        s.apply(Event::Notice(app_notice("k1")), t0);
        assert_eq!(s.next_deadline(), Some(t0 + secs(5)));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn hover_during_notice_is_invisible_but_expands_media_after() {
        let t0 = Instant::now();
        let mut s = IslandState::new();
        s.apply(Event::Media(Some(media(true))), t0);
        s.apply(Event::Notice(app_notice("k1")), t0);
        s.apply(Event::Hover(true), t0 + secs(1));
        assert_eq!(s.view(), View::Notice(app_notice("k1")));
        s.apply(Event::Tick, t0 + secs(5));
        assert_eq!(s.view(), View::MediaExpanded);
    }
}
