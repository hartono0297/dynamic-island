//! Pointer gestures on the island while music plays: a swipe left or right
//! with the left button, a sideways wheel (a touchpad's two-finger scroll, a
//! tilt wheel) and the middle button. Plain window messages: no global hook.
//! The evaluators here are pure; `App` feeds them and decides where they
//! count (the music views, with a session).

use std::time::{Duration, Instant};

/// A swipe's shortest way, in logical pixels.
pub const SWIPE_MIN_DIP: f32 = 56.0;
/// A swipe's longest time from press to release.
const SWIPE_MAX: Duration = Duration::from_millis(700);
/// A sideways wheel's delta that counts as one flick (one notch).
const HWHEEL_STEP: i32 = 120;
/// Deltas further apart than this are not one scroll.
const HWHEEL_GAP: Duration = Duration::from_millis(250);
/// After a flick, the rest of the same scroll (a touchpad's momentum) is
/// ignored for this long.
const HWHEEL_COOLDOWN: Duration = Duration::from_millis(500);

/// Which way a gesture asks the music to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Prev,
    Next,
}

/// A press with the left button and what it turns into on release.
#[derive(Default)]
pub struct Swipe {
    down: Option<(i32, i32, Instant)>,
}

impl Swipe {
    pub fn press(&mut self, x: i32, y: i32, now: Instant) {
        self.down = Some((x, y, now));
    }

    pub fn cancel(&mut self) {
        self.down = None;
    }

    /// The swipe the release ends, if it was one: far enough sideways
    /// (`min_px`), mostly sideways (twice as far across as up or down) and
    /// quick. Left is the next song, right the one before. Every release
    /// ends the press.
    pub fn release(&mut self, x: i32, y: i32, now: Instant, min_px: i32) -> Option<Dir> {
        let (x0, y0, t0) = self.down.take()?;
        let (dx, dy) = (x - x0, y - y0);
        let quick = now.saturating_duration_since(t0) <= SWIPE_MAX;
        (quick && dx.abs() >= min_px && dx.abs() >= 2 * dy.abs())
            .then(|| if dx < 0 { Dir::Next } else { Dir::Prev })
    }
}

/// The sideways wheel's deltas, added up until they make one flick.
#[derive(Default)]
pub struct HWheel {
    sum: i32,
    last: Option<Instant>,
    quiet_until: Option<Instant>,
}

impl HWheel {
    /// One `WM_MOUSEHWHEEL` delta (positive: scrolled right). Scrolling
    /// right is the next song, left the one before. A flick fires once; what
    /// follows it is spent until the scroll has been still for
    /// `HWHEEL_COOLDOWN` (each delta meanwhile pushes the end out), so a
    /// touchpad's long momentum tail never fires a second time.
    pub fn push(&mut self, delta: i32, now: Instant) -> Option<Dir> {
        if self.quiet_until.is_some_and(|t| now < t) {
            self.last = Some(now);
            self.quiet_until = Some(now + HWHEEL_COOLDOWN);
            return None;
        }
        if self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) > HWHEEL_GAP)
        {
            self.sum = 0;
        }
        self.last = Some(now);
        self.sum += delta;
        if self.sum.abs() < HWHEEL_STEP {
            return None;
        }
        let dir = if self.sum > 0 { Dir::Next } else { Dir::Prev };
        self.sum = 0;
        self.quiet_until = Some(now + HWHEEL_COOLDOWN);
        Some(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_quick_sideways_drag_is_a_swipe() {
        let t0 = Instant::now();
        let mut s = Swipe::default();
        s.press(200, 10, t0);
        assert_eq!(s.release(120, 14, t0 + ms(180), 56), Some(Dir::Next));
        s.press(100, 10, t0);
        assert_eq!(s.release(190, 0, t0 + ms(300), 56), Some(Dir::Prev));
    }

    #[test]
    fn short_slow_or_diagonal_drags_and_releases_without_a_press_are_not() {
        let t0 = Instant::now();
        let mut s = Swipe::default();
        s.press(200, 10, t0);
        assert_eq!(s.release(160, 10, t0 + ms(100), 56), None, "too short");
        s.press(200, 10, t0);
        assert_eq!(s.release(100, 10, t0 + ms(900), 56), None, "too slow");
        s.press(200, 10, t0);
        assert_eq!(s.release(120, 70, t0 + ms(100), 56), None, "diagonal");
        assert_eq!(s.release(100, 10, t0 + ms(100), 56), None, "no press");
        // Every release ends the press; a cancel does too.
        s.press(200, 10, t0);
        s.cancel();
        assert_eq!(s.release(100, 10, t0 + ms(100), 56), None);
        // The bigger the screen scale, the further a swipe has to go.
        s.press(200, 10, t0);
        assert_eq!(s.release(120, 10, t0 + ms(100), 84), None);
    }

    #[test]
    fn a_sideways_wheel_flicks_once_per_scroll() {
        let t0 = Instant::now();
        let mut w = HWheel::default();
        // A notch.
        assert_eq!(w.push(120, t0), Some(Dir::Next));
        // A touchpad's momentum right after it: spent.
        for i in 1..20 {
            assert_eq!(w.push(40, t0 + ms(i * 20)), None);
        }
        // Later, small deltas add up; the other way is the other song.
        let t1 = t0 + ms(2_000);
        assert_eq!(w.push(-50, t1), None);
        assert_eq!(w.push(-50, t1 + ms(30)), None);
        assert_eq!(w.push(-50, t1 + ms(60)), Some(Dir::Prev));
    }

    #[test]
    fn a_whole_second_of_momentum_fires_once() {
        let t0 = Instant::now();
        let mut w = HWheel::default();
        assert_eq!(w.push(120, t0), Some(Dir::Next));
        // A touchpad's tail: a delta every 20 ms for a full second, well past
        // the cooldown counted from the flick.
        for i in 1..=50 {
            assert_eq!(w.push(40, t0 + ms(i * 20)), None, "delta {i}");
        }
        // Once the scroll has been still for the cooldown, a flick counts.
        let after = t0 + ms(1_000) + HWHEEL_COOLDOWN;
        assert_eq!(w.push(120, after), Some(Dir::Next));
    }

    #[test]
    fn slow_drips_of_wheel_never_add_up() {
        let t0 = Instant::now();
        let mut w = HWheel::default();
        for i in 0..10 {
            assert_eq!(w.push(40, t0 + ms(i * 400)), None);
        }
    }
}
