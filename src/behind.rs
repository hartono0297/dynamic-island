//! "Di belakang jendela": with the setting on, the island does not stay on
//! top of everything. It sits at the bottom of the window order, so ordinary
//! windows cover it, and comes up when it has something to say or the pointer
//! touches the top edge of the screen over it. The decision is pure; `App`
//! reads the pointer and applies the order (`IslandWindow::set_front`).

use std::time::{Duration, Instant};

use crate::model::View;

/// The strip along the top edge of the screen that brings the island up, in
/// physical pixels.
pub const STRIP_PX: i32 = 3;
/// How long the island stays up after the pointer leaves it (and nothing is
/// showing) before it goes back down.
pub const LOWER_AFTER: Duration = Duration::from_millis(250);
/// How often the pointer is read near the top edge, or while lowering is due.
pub const POLL_MS: u32 = 80;
/// How often it is read otherwise: the pointer far below the top edge cannot
/// reach the strip within one slow poll unnoticed for long.
pub const SLOW_MS: u32 = 350;
/// "Near the top edge", in logical pixels from it.
pub const NEAR_TOP_DIP: f32 = 150.0;

/// The poll's pace: quick while lowering is due (`LOWER_AFTER` must be kept)
/// or the pointer is near the top edge (the strip, the island); slow
/// otherwise. A view that must be seen needs no poll to stay up: a change of
/// view re-reads at once.
pub fn poll_ms(lower_pending: bool, pointer_near_top: bool) -> u32 {
    if lower_pending || pointer_near_top {
        POLL_MS
    } else {
        SLOW_MS
    }
}

/// Where the island sits in the window order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Z {
    /// Above every window, as without the setting.
    Top,
    /// Below ordinary windows.
    Bottom,
}

/// Whether what the island shows must be seen: anything but the quiet
/// compact states. A notice, a panel, the search and a screenshot preview all
/// are; the idle island, the music and the timer in the notch are not.
pub fn view_needs_front(view: &View) -> bool {
    !matches!(
        view,
        View::Hidden
            | View::Idle
            | View::MediaCompact
            | View::MediaLyric
            | View::PomodoroCompact
    )
}

/// What the decision reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// The view must be seen (`view_needs_front`).
    pub needs_front: bool,
    /// The pointer is on the top-edge strip over the island's width.
    pub on_strip: bool,
    /// The pointer is over the drawn island (counts only while it is up).
    pub over_island: bool,
}

/// The island's place in the window order and the time it has been idle up
/// there.
#[derive(Debug)]
pub struct Behind {
    z: Z,
    idle_since: Option<Instant>,
}

impl Default for Behind {
    fn default() -> Self {
        Behind {
            z: Z::Top,
            idle_since: None,
        }
    }
}

impl Behind {
    pub fn z(&self) -> Z {
        self.z
    }

    /// Up and counting down to going back down.
    pub fn lower_pending(&self) -> bool {
        self.idle_since.is_some()
    }

    /// Back to the start: up, nothing waiting.
    pub fn reset(&mut self) {
        *self = Behind::default();
    }

    /// The place the island should have now. Something to show, the pointer
    /// on the strip, or the pointer still on the island while it is up, keeps
    /// it up; once none of those holds for `LOWER_AFTER` it goes down.
    pub fn decide(&mut self, inputs: Inputs, now: Instant) -> Z {
        let hold = inputs.needs_front
            || inputs.on_strip
            || (inputs.over_island && self.z == Z::Top);
        if hold {
            self.z = Z::Top;
            self.idle_since = None;
        } else if self.z == Z::Top {
            let since = *self.idle_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= LOWER_AFTER {
                self.z = Z::Bottom;
                self.idle_since = None;
            }
        }
        self.z
    }
}

/// How far past the drawn island the strip reaches on each side, in logical
/// pixels.
pub const STRIP_MARGIN_DIP: f32 = 12.0;

/// The strip along the top of the screen (`top` is its first row) over the
/// island's `span` (screen x, from and up to) and the point `pt` on the
/// screen: true when the point is on it. The span is the drawn island plus a
/// margin, not the window, which is wider than the island.
pub fn on_strip(top: i32, span: (i32, i32), pt: (i32, i32)) -> bool {
    pt.1 >= top && pt.1 < top + STRIP_PX && pt.0 >= span.0 && pt.0 < span.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Notice, NoticeKind, Page};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn quiet() -> Inputs {
        Inputs::default()
    }

    #[test]
    fn it_starts_up_and_goes_down_only_after_a_quiet_moment() {
        let t0 = Instant::now();
        let mut b = Behind::default();
        assert_eq!(b.z(), Z::Top);
        assert_eq!(b.decide(quiet(), t0), Z::Top, "the wait starts");
        assert_eq!(b.decide(quiet(), t0 + ms(240)), Z::Top);
        assert_eq!(b.decide(quiet(), t0 + ms(250)), Z::Bottom);
        assert_eq!(b.decide(quiet(), t0 + ms(900)), Z::Bottom, "it stays down");
    }

    #[test]
    fn the_strip_brings_it_up_and_the_pointer_on_it_keeps_it_up() {
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.decide(quiet(), t0);
        b.decide(quiet(), t0 + ms(300));
        assert_eq!(b.z(), Z::Bottom);
        let strip = Inputs {
            on_strip: true,
            ..quiet()
        };
        assert_eq!(b.decide(strip, t0 + ms(400)), Z::Top);
        // The pointer comes down onto the island: it holds.
        let over = Inputs {
            over_island: true,
            ..quiet()
        };
        for i in 1..20 {
            assert_eq!(b.decide(over, t0 + ms(400 + i * 80)), Z::Top);
        }
        // It leaves: 250 ms later the island goes down.
        let gone = t0 + ms(2_000);
        assert_eq!(b.decide(quiet(), gone), Z::Top);
        assert_eq!(b.decide(quiet(), gone + ms(250)), Z::Bottom);
    }

    #[test]
    fn being_over_the_island_alone_does_not_raise_it_from_the_bottom() {
        // Down there another window may be what the pointer is over.
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.decide(quiet(), t0);
        b.decide(quiet(), t0 + ms(300));
        let over = Inputs {
            over_island: true,
            ..quiet()
        };
        assert_eq!(b.decide(over, t0 + ms(400)), Z::Bottom);
    }

    #[test]
    fn something_to_show_raises_it_at_once_and_keeps_it_up() {
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.decide(quiet(), t0);
        b.decide(quiet(), t0 + ms(300));
        let notice = Inputs {
            needs_front: true,
            ..quiet()
        };
        assert_eq!(b.decide(notice, t0 + ms(310)), Z::Top);
        assert_eq!(b.decide(notice, t0 + ms(60_000)), Z::Top);
        // Once it is gone the quiet moment starts from then.
        let gone = t0 + ms(61_000);
        assert_eq!(b.decide(quiet(), gone), Z::Top);
        assert_eq!(b.decide(quiet(), gone + ms(250)), Z::Bottom);
    }

    #[test]
    fn a_pointer_that_returns_restarts_the_wait() {
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.decide(quiet(), t0);
        assert_eq!(b.decide(quiet(), t0 + ms(200)), Z::Top);
        let over = Inputs {
            over_island: true,
            ..quiet()
        };
        b.decide(over, t0 + ms(220));
        assert_eq!(b.decide(quiet(), t0 + ms(240)), Z::Top, "a new wait");
        assert_eq!(b.decide(quiet(), t0 + ms(490)), Z::Bottom);
    }

    #[test]
    fn the_poll_slows_down_only_far_from_the_top_with_nothing_due() {
        assert_eq!(poll_ms(false, false), SLOW_MS);
        assert_eq!(poll_ms(false, true), POLL_MS, "near the strip");
        assert_eq!(poll_ms(true, false), POLL_MS, "lowering is due");
        // The countdown to lowering shows as pending until it lands.
        let t0 = Instant::now();
        let mut b = Behind::default();
        assert!(!b.lower_pending());
        b.decide(quiet(), t0);
        assert!(b.lower_pending());
        b.decide(quiet(), t0 + LOWER_AFTER);
        assert!(!b.lower_pending() && b.z() == Z::Bottom);
    }

    #[test]
    fn reset_puts_it_back_up() {
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.decide(quiet(), t0);
        b.decide(quiet(), t0 + ms(300));
        b.reset();
        assert_eq!(b.z(), Z::Top);
    }

    #[test]
    fn only_the_quiet_views_may_stay_behind() {
        for view in [
            View::Hidden,
            View::Idle,
            View::MediaCompact,
            View::MediaLyric,
            View::PomodoroCompact,
        ] {
            assert!(!view_needs_front(&view), "{view:?}");
        }
        assert!(view_needs_front(&View::Panel(Page::Calendar)));
        assert!(view_needs_front(&View::Panel(Page::Search)));
        let notice = Notice {
            key: "n".into(),
            source: "s".into(),
            title: "t".into(),
            body: String::new(),
            kind: NoticeKind::Copied,
        };
        assert!(view_needs_front(&View::Notice(notice)));
    }

    #[test]
    fn the_strip_is_three_pixels_over_the_drawn_island_only() {
        // The island is drawn from x 400 to 600; the window around it is wider.
        let span = (400, 600);
        assert!(on_strip(0, span, (500, 0)));
        assert!(on_strip(0, span, (500, 2)));
        assert!(!on_strip(0, span, (500, 3)));
        assert!(!on_strip(0, span, (399, 0)));
        assert!(on_strip(0, span, (400, 0)));
        assert!(!on_strip(0, span, (600, 0)));
        assert!(!on_strip(0, span, (200, 0)), "beside the island, over the window");
        assert!(!on_strip(0, span, (500, -1)));
    }
}
