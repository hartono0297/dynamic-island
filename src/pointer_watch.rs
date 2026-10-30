//! The one pointer poll the closed island shares between two features, and
//! the pure decisions behind them. `App` reads `GetCursorPos` on
//! `TIMER_POINTER` (paced by `poll_ms`) and feeds both:
//!
//! - **Notch peek**: with "Di belakang jendela" off, the notch shrinks to the
//!   cat's two eyes while the pointer is near it and an ordinary window lies
//!   under it (a browser's tab strip), so what is under stays reachable. It
//!   comes back once the pointer rests on the eyes (`Peek`).
//! - **The cat looks at the pointer**: sideways, eased (`look_target`,
//!   `LookEase`).
//!
//! Nothing here touches Win32; `App` reads the pointer and applies the result.

use std::time::{Duration, Instant};

use crate::behind;
use crate::model::View;
use crate::ui::geometry::{AA_MARGIN, Rect, ShapeRect};

/// How far past the normal notch (left, right and below) the pointer counts as
/// near it, in logical pixels.
pub const PEEK_ZONE_DIP: f32 = 50.0;
/// How long the pointer must rest on the eyes before the notch comes back.
pub const PEEK_DWELL: Duration = Duration::from_millis(500);
/// How long a "no window under the notch" reading is trusted while the
/// pointer stays in the zone.
pub const UNDER_RECHECK: Duration = Duration::from_secs(1);

/// The cat follows the pointer at this pace near the top edge, and at
/// `FOLLOW_FAR_MS` otherwise (the ease runs on the frame clock, not on this).
pub const FOLLOW_NEAR_MS: u32 = 33;
pub const FOLLOW_FAR_MS: u32 = 250;

/// A change of the look target smaller than this is ignored: no redraw.
pub const LOOK_EPS: f32 = 0.02;
/// The look's ease time constant, milliseconds.
pub const LOOK_TAU_MS: f32 = 120.0;
/// Within this of its target the look snaps there and stops moving.
pub const LOOK_SNAP: f32 = 0.002;

/// The poll's pace in milliseconds; 0 when neither feature wants it. The cat
/// follows at 33 ms near the top edge and 250 ms far from it; the peek alone
/// reads as `behind` does (80 ms near, 350 ms far). `near_top`: the pointer is
/// within `behind::NEAR_TOP_DIP` of the island's monitor's top edge.
pub fn poll_ms(follow_on: bool, peek_on: bool, near_top: bool) -> u32 {
    match (follow_on, peek_on) {
        (false, false) => 0,
        (true, _) => {
            if near_top {
                FOLLOW_NEAR_MS
            } else {
                FOLLOW_FAR_MS
            }
        }
        (false, true) => {
            if near_top {
                behind::POLL_MS
            } else {
                behind::SLOW_MS
            }
        }
    }
}

/// The pointer is near the island's monitor's top edge: `dy_px` physical
/// pixels below it (negative above it, on a monitor stacked on top) and less
/// than `behind::NEAR_TOP_DIP` down at `scale`. Above the edge is not near.
pub fn near_top(dy_px: i32, scale: f32) -> bool {
    dy_px >= 0 && (dy_px as f32) < behind::NEAR_TOP_DIP * scale
}

/// The views whose closed island may shrink to the eyes: the quiet compact
/// ones. A notice has to be seen, a panel is open on purpose, and Hidden shows
/// nothing.
pub fn peek_view(view: &View) -> bool {
    matches!(
        view,
        View::Idle | View::MediaCompact | View::MediaLyric | View::PomodoroCompact
    )
}

/// Where the notch peek is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekState {
    /// The notch as it is.
    Normal,
    /// Shrunk to the eyes; `over_since` is when the pointer came onto them
    /// and has stayed since.
    Shrunk { over_since: Option<Instant> },
    /// Back at full size, with the pointer still near: it does not shrink
    /// again until the pointer has left the zone.
    Restored,
}

/// What one reading of the pointer says (`Peek::step`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeekInputs {
    /// The peek may act at all (no stay-behind, no fullscreen, not locked, a
    /// quiet compact view).
    pub eligible: bool,
    /// The pointer is inside the zone round the *normal* notch (never the
    /// eyes, so the zone does not flap with the shape).
    pub in_zone: bool,
    /// The pointer is over the normal notch.
    pub over_normal: bool,
    /// The pointer is over the drawn eyes.
    pub over_pill: bool,
    /// An ordinary window lies under the normal notch.
    pub window_under: bool,
    /// A mouse button is down (a drag may be on its way to the island): a
    /// notch that is not shrunk yet stays as it is.
    pub dragging: bool,
}

#[derive(Debug)]
pub struct Peek {
    state: PeekState,
}

impl Default for Peek {
    fn default() -> Self {
        Peek {
            state: PeekState::Normal,
        }
    }
}

impl Peek {
    pub fn state(&self) -> PeekState {
        self.state
    }

    pub fn is_shrunk(&self) -> bool {
        matches!(self.state, PeekState::Shrunk { .. })
    }

    /// Back to `Normal` at once. True when it was something else.
    pub fn reset(&mut self) -> bool {
        std::mem::replace(&mut self.state, PeekState::Normal) != PeekState::Normal
    }

    /// A click on the eyes: the notch comes back at once. True when it was shrunk.
    pub fn restore(&mut self) -> bool {
        if self.is_shrunk() {
            self.state = PeekState::Restored;
            true
        } else {
            false
        }
    }

    /// The state after one reading.
    pub fn step(&mut self, i: PeekInputs, now: Instant) -> PeekState {
        self.state = match self.state {
            _ if !i.eligible => PeekState::Normal,
            PeekState::Normal => {
                if !i.in_zone {
                    PeekState::Normal
                } else if i.over_normal {
                    // A notch the pointer has already reached is never pulled
                    // away from under it.
                    PeekState::Restored
                } else if i.window_under && !i.dragging {
                    PeekState::Shrunk { over_since: None }
                } else {
                    PeekState::Normal
                }
            }
            PeekState::Shrunk { over_since } => {
                if !i.in_zone {
                    PeekState::Normal
                } else if i.over_pill {
                    let since = over_since.unwrap_or(now);
                    if now.saturating_duration_since(since) >= PEEK_DWELL {
                        PeekState::Restored
                    } else {
                        PeekState::Shrunk {
                            over_since: Some(since),
                        }
                    }
                } else {
                    PeekState::Shrunk { over_since: None }
                }
            }
            PeekState::Restored => {
                if i.in_zone {
                    PeekState::Restored
                } else {
                    PeekState::Normal
                }
            }
        };
        self.state
    }
}

/// The zone round the normal notch `s`: it reaches `PEEK_ZONE_DIP` past its
/// left, right and bottom, and up to the monitor's top edge (`AA_MARGIN`
/// past it, no further: a monitor that sits above is another screen).
pub fn peek_zone(s: ShapeRect) -> Rect {
    const ABOVE: f32 = AA_MARGIN;
    Rect {
        x: s.x - PEEK_ZONE_DIP,
        y: -ABOVE,
        w: s.w + 2.0 * PEEK_ZONE_DIP,
        h: ABOVE + s.y + s.h + PEEK_ZONE_DIP,
    }
}

/// Remembers whether a window lay under the notch, so the walk over the
/// windows runs when the pointer first comes near and at most once a
/// `UNDER_RECHECK` while it stays.
#[derive(Debug, Default)]
pub struct UnderCache {
    read: Option<(Instant, bool)>,
}

impl UnderCache {
    /// The cached answer, or `read()` when there is none or it is stale.
    pub fn get(&mut self, now: Instant, read: impl FnOnce() -> bool) -> bool {
        if let Some((at, value)) = self.read
            && now.saturating_duration_since(at) < UNDER_RECHECK
        {
            return value;
        }
        let value = read();
        self.read = Some((now, value));
        value
    }

    /// The pointer left the zone: the next approach reads afresh.
    pub fn clear(&mut self) {
        self.read = None;
    }
}

/// Where the pointer is across the screen, -1 (the left edge) to 1 (the right
/// edge), 0 straight over the notch's centre: its offset from `centre_x` over
/// the distance from the centre to that side's edge of the monitor.
pub fn look_target(pointer_x: f32, centre_x: f32, monitor_left: f32, monitor_right: f32) -> f32 {
    let offset = pointer_x - centre_x;
    let span = if offset >= 0.0 {
        monitor_right - centre_x
    } else {
        centre_x - monitor_left
    };
    if span <= 0.0 {
        return 0.0;
    }
    (offset / span).clamp(-1.0, 1.0)
}

/// The cat's look, -1 to 1, eased toward its target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookEase {
    current: f32,
    target: f32,
}

impl LookEase {
    pub fn value(&self) -> f32 {
        self.current
    }

    pub fn moving(&self) -> bool {
        self.current != self.target
    }

    /// Heads for `v` (clamped to -1..=1). A change of the target smaller than
    /// `LOOK_EPS` is ignored and returns false: nothing needs a redraw.
    pub fn set_target(&mut self, v: f32) -> bool {
        let v = v.clamp(-1.0, 1.0);
        if (v - self.target).abs() < LOOK_EPS {
            return false;
        }
        self.target = v;
        true
    }

    /// Heads back to the centre, whatever the distance. True when that
    /// changed the target.
    pub fn release(&mut self) -> bool {
        std::mem::replace(&mut self.target, 0.0) != 0.0
    }

    /// At the centre now, nothing left to ease. True when that moved the cat.
    pub fn snap_to_centre(&mut self) -> bool {
        let moved = self.current != 0.0;
        self.current = 0.0;
        self.target = 0.0;
        moved
    }

    /// `dt` later: an exponential ease, the same at any frame rate, that never
    /// overshoots and snaps once it is within `LOOK_SNAP`.
    pub fn advance(&mut self, dt: Duration) {
        let rate = 1.0 - (-dt.as_secs_f32() * 1000.0 / LOOK_TAU_MS).exp();
        self.current += (self.target - self.current) * rate;
        if (self.target - self.current).abs() < LOOK_SNAP {
            self.current = self.target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Notice, NoticeKind, Page};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_poll_pace_for_every_combination() {
        // Neither feature: no poll.
        assert_eq!(poll_ms(false, false, true), 0);
        assert_eq!(poll_ms(false, false, false), 0);
        // The cat follows: 33 ms near the top edge, 250 ms far.
        for peek in [false, true] {
            assert_eq!(poll_ms(true, peek, true), 33);
            assert_eq!(poll_ms(true, peek, false), 250);
        }
        // The peek alone paces like stay-behind.
        assert_eq!(poll_ms(false, true, true), behind::POLL_MS);
        assert_eq!(poll_ms(false, true, false), behind::SLOW_MS);
        assert_eq!((behind::POLL_MS, behind::SLOW_MS), (80, 350));
        assert_eq!(behind::NEAR_TOP_DIP, 150.0);
    }

    fn near() -> PeekInputs {
        PeekInputs {
            eligible: true,
            in_zone: true,
            window_under: true,
            ..PeekInputs::default()
        }
    }

    #[test]
    fn near_with_a_window_under_shrinks() {
        let (mut peek, now) = (Peek::default(), Instant::now());
        assert_eq!(peek.state(), PeekState::Normal);
        assert_eq!(
            peek.step(near(), now),
            PeekState::Shrunk { over_since: None }
        );
        assert!(peek.is_shrunk());
    }

    #[test]
    fn the_desktop_under_it_keeps_the_notch() {
        let (mut peek, now) = (Peek::default(), Instant::now());
        let desktop = PeekInputs {
            window_under: false,
            ..near()
        };
        assert_eq!(peek.step(desktop, now), PeekState::Normal);
    }

    #[test]
    fn far_from_it_nothing_happens() {
        let (mut peek, now) = (Peek::default(), Instant::now());
        let far = PeekInputs {
            in_zone: false,
            ..near()
        };
        assert_eq!(peek.step(far, now), PeekState::Normal);
    }

    #[test]
    fn not_eligible_is_normal_at_once() {
        let now = Instant::now();
        // Stay-behind on, fullscreen, a notice, a panel, hidden: none is eligible.
        let off = PeekInputs {
            eligible: false,
            ..near()
        };
        let mut peek = Peek::default();
        assert_eq!(peek.step(off, now), PeekState::Normal);
        // And from every state.
        for from in [
            PeekState::Shrunk {
                over_since: Some(now),
            },
            PeekState::Restored,
        ] {
            let mut peek = Peek { state: from };
            assert_eq!(peek.step(off, now), PeekState::Normal, "{from:?}");
        }
        assert!(Peek { state: PeekState::Restored }.reset());
        assert!(!Peek::default().reset());
    }

    #[test]
    fn the_view_decides_who_may_shrink() {
        let notice = View::Notice(Notice {
            key: "k".into(),
            source: "s".into(),
            title: "t".into(),
            body: "b".into(),
            kind: NoticeKind::BatteryLow,
        });
        for view in [
            View::Idle,
            View::MediaCompact,
            View::MediaLyric,
            View::PomodoroCompact,
        ] {
            assert!(peek_view(&view), "{view:?}");
        }
        // A notice has to be seen; the panel is open on purpose; fullscreen
        // hides the notch, and it stays hidden.
        for view in [notice, View::Panel(Page::Calendar), View::Hidden] {
            assert!(!peek_view(&view), "{view:?}");
        }
    }

    #[test]
    fn a_notch_the_pointer_already_reached_is_never_pulled_away() {
        let (mut peek, now) = (Peek::default(), Instant::now());
        let on_it = PeekInputs {
            over_normal: true,
            ..near()
        };
        assert_eq!(peek.step(on_it, now), PeekState::Restored);
        // Moving off it, still in the zone: it stays, it does not shrink now.
        assert_eq!(peek.step(near(), now + ms(100)), PeekState::Restored);
    }

    #[test]
    fn a_pointer_held_down_does_not_start_a_shrink() {
        // A drag toward the island must still find the notch to drop on.
        let (mut peek, now) = (Peek::default(), Instant::now());
        let drag = PeekInputs {
            dragging: true,
            ..near()
        };
        assert_eq!(peek.step(drag, now), PeekState::Normal);
        assert!(matches!(
            peek.step(near(), now + ms(10)),
            PeekState::Shrunk { .. }
        ));
    }

    fn shrunk() -> (Peek, Instant) {
        let now = Instant::now();
        let mut peek = Peek::default();
        peek.step(near(), now);
        (peek, now)
    }

    #[test]
    fn half_a_second_on_the_eyes_brings_the_notch_back() {
        let (mut peek, t0) = shrunk();
        let on_pill = PeekInputs {
            over_pill: true,
            ..near()
        };
        // First seen on the eyes at t0.
        assert!(matches!(peek.step(on_pill, t0), PeekState::Shrunk { .. }));
        assert!(matches!(
            peek.step(on_pill, t0 + ms(499)),
            PeekState::Shrunk { .. }
        ));
        assert_eq!(peek.step(on_pill, t0 + ms(500)), PeekState::Restored);
    }

    #[test]
    fn leaving_the_eyes_starts_the_dwell_again() {
        let (mut peek, t0) = shrunk();
        let on_pill = PeekInputs {
            over_pill: true,
            ..near()
        };
        peek.step(on_pill, t0);
        peek.step(on_pill, t0 + ms(400));
        // Off the eyes (still in the zone): the dwell is forgotten.
        assert_eq!(
            peek.step(near(), t0 + ms(450)),
            PeekState::Shrunk { over_since: None }
        );
        peek.step(on_pill, t0 + ms(500));
        assert!(matches!(
            peek.step(on_pill, t0 + ms(900)),
            PeekState::Shrunk { .. }
        ));
        assert_eq!(peek.step(on_pill, t0 + ms(1000)), PeekState::Restored);
    }

    #[test]
    fn leaving_the_zone_brings_it_back_from_shrunk_and_from_restored() {
        let out = PeekInputs {
            in_zone: false,
            ..near()
        };
        let (mut peek, t0) = shrunk();
        assert_eq!(peek.step(out, t0 + ms(10)), PeekState::Normal);

        let (mut peek, t0) = shrunk();
        peek.step(
            PeekInputs {
                over_pill: true,
                ..near()
            },
            t0,
        );
        assert!(peek.restore());
        assert_eq!(peek.step(out, t0 + ms(10)), PeekState::Normal);
        // And it can shrink again on the next approach.
        assert!(matches!(
            peek.step(near(), t0 + ms(20)),
            PeekState::Shrunk { .. }
        ));
    }

    #[test]
    fn a_click_on_the_eyes_restores_at_once() {
        let (mut peek, _) = shrunk();
        assert!(peek.restore());
        assert_eq!(peek.state(), PeekState::Restored);
        // Nothing to restore when it is not shrunk.
        assert!(!peek.restore());
    }

    #[test]
    fn the_zone_reaches_50_dip_out_and_up_past_the_screen() {
        let s = ShapeRect {
            x: 215.0,
            y: 8.0,
            w: 190.0,
            h: 34.0,
            radius: 17.0,
        };
        let z = peek_zone(s);
        assert!(z.contains(s.x - 49.0, 20.0) && !z.contains(s.x - 51.0, 20.0));
        assert!(z.contains(s.x + s.w + 49.0, 20.0) && !z.contains(s.x + s.w + 51.0, 20.0));
        assert!(z.contains(300.0, s.y + s.h + 49.0) && !z.contains(300.0, s.y + s.h + 51.0));
        // Up to the screen's top edge, not onto a monitor stacked above it.
        assert!(z.contains(300.0, 0.0) && z.contains(300.0, -AA_MARGIN));
        assert!(!z.contains(300.0, -2.0) && !z.contains(300.0, -30.0));
    }

    #[test]
    fn a_pointer_on_a_monitor_above_is_not_near_the_top() {
        assert!(near_top(0, 1.0) && near_top(149, 1.0));
        assert!(!near_top(150, 1.0));
        // 150 dip at 150 % is 225 px.
        assert!(near_top(224, 1.5) && !near_top(225, 1.5));
        // Above the island's monitor (another screen stacked on top).
        assert!(!near_top(-1, 1.0) && !near_top(-5_000, 1.5));
    }

    #[test]
    fn the_window_walk_runs_once_per_approach_and_once_a_second() {
        let (mut cache, t0) = (UnderCache::default(), Instant::now());
        let mut reads = 0;
        let mut get = |cache: &mut UnderCache, at: Instant, answer: bool| {
            cache.get(at, || {
                reads += 1;
                answer
            })
        };
        assert!(!get(&mut cache, t0, false));
        assert!(!get(&mut cache, t0 + ms(500), true), "the first answer holds");
        assert!(get(&mut cache, t0 + ms(1000), true), "a second on, asked again");
        assert!(get(&mut cache, t0 + ms(1500), false));
        cache.clear();
        assert!(!get(&mut cache, t0 + ms(1600), false), "a new approach reads");
        assert_eq!(reads, 3);
    }

    #[test]
    fn the_look_follows_the_pointer_across_the_screen() {
        // A 1920 wide monitor, the notch in the middle.
        let look = |x| look_target(x, 960.0, 0.0, 1920.0);
        assert_eq!(look(960.0), 0.0);
        assert_eq!(look(1440.0), 0.5);
        assert_eq!(look(480.0), -0.5);
        // At the edges and past them (another monitor): clamped.
        assert_eq!(look(1920.0), 1.0);
        assert_eq!(look(0.0), -1.0);
        assert_eq!(look(5000.0), 1.0);
        assert_eq!(look(-3000.0), -1.0);
        // Off-centre notch: each side has its own span.
        assert_eq!(look_target(100.0, 200.0, 0.0, 1000.0), -0.5);
        assert_eq!(look_target(600.0, 200.0, 0.0, 1000.0), 0.5);
        // No room on a side: no look.
        assert_eq!(look_target(10.0, 0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn the_look_ignores_a_small_change_and_asks_for_no_redraw() {
        let mut look = LookEase::default();
        assert!(!look.set_target(0.01), "below LOOK_EPS");
        assert!(!look.moving());
        assert!(look.set_target(0.5));
        assert!(look.moving());
        assert!(!look.set_target(0.51), "a hair from the target");
        assert!(look.set_target(0.53));
        assert!(look.set_target(5.0), "clamped to 1");
        assert!(!look.set_target(1.0));
    }

    #[test]
    fn the_look_eases_without_overshoot_and_stops() {
        let mut look = LookEase::default();
        look.set_target(1.0);
        let mut last = 0.0;
        for _ in 0..200 {
            look.advance(ms(16));
            assert!(look.value() >= last && look.value() <= 1.0, "{}", look.value());
            last = look.value();
            if !look.moving() {
                break;
            }
        }
        assert!(!look.moving());
        assert_eq!(look.value(), 1.0);
        // Back the other way, past the centre.
        look.set_target(-1.0);
        let mut last = 1.0;
        while look.moving() {
            look.advance(ms(16));
            assert!(look.value() <= last && look.value() >= -1.0);
            last = look.value();
        }
        assert_eq!(look.value(), -1.0);
    }

    #[test]
    fn the_ease_is_the_same_at_any_frame_rate() {
        let at = |frames: u32, each: u64| {
            let mut look = LookEase::default();
            look.set_target(1.0);
            for _ in 0..frames {
                look.advance(ms(each));
            }
            look.value()
        };
        // 240 ms: 4 x 60 ms, 15 x 16 ms, one jump, all the same ease.
        let (a, b, c) = (at(4, 60), at(15, 16), at(1, 240));
        assert!((a - c).abs() < 0.001 && (b - c).abs() < 0.001, "{a} {b} {c}");
        // About 1 - e^-2 after two time constants.
        assert!((c - 0.865).abs() < 0.01, "{c}");
    }

    #[test]
    fn releasing_heads_for_the_centre_and_a_snap_gets_there() {
        let mut look = LookEase::default();
        look.set_target(0.01 + LOOK_EPS);
        look.advance(ms(1000));
        assert!(look.release(), "even a small target is let go");
        assert!(look.moving());
        assert!(look.snap_to_centre());
        assert!(!look.moving());
        assert_eq!(look.value(), 0.0);
        assert!(!look.release());
        assert!(!look.snap_to_centre());
    }
}
