//! When the island wears its faint drag margin. Windows offers a drag only to
//! the pixels a layered window draws, and the top edge of the screen is where
//! a file is aimed; but a margin that is always there swallows clicks on
//! whatever lies under it (a maximized window's tab strip). So it is drawn
//! only while a drag can be about to start: the left button is down and the
//! pointer is near the island (`is_near`).
//!
//! Reading the button and the pointer needs a poll, and an idle island runs
//! none. The poll starts on a cheap trigger (the pointer moving over the
//! island, a drag entering it) and runs while the button is down, or while
//! the pointer stays near for `LINGER` after the trigger; then it stops until
//! the next one. A drag that comes straight at the empty strip above an
//! island it never touched finds no margin: it must reach the island itself.

use std::time::Duration;

/// How often the button and the pointer are read while a drag is close.
pub const FAST_MS: u32 = 50;
/// How often they are read otherwise, while the poll runs at all.
pub const SLOW_MS: u32 = 250;
/// How near the island the pointer must be, in logical pixels.
pub const NEAR_DIP: f32 = 100.0;
/// The fast pace reaches this many times `NEAR_DIP`, so the margin is up
/// before a quick drag arrives.
pub const FAST_REACH: f32 = 3.0;
/// With the button up, how long after the last trigger the poll keeps
/// waiting near the island for a drag to start.
pub const LINGER: Duration = Duration::from_secs(3);

/// `island` is (left, top, right, bottom) and `cursor` a point, both in the
/// same unit as `near`.
pub fn is_near(cursor: (f32, f32), island: (f32, f32, f32, f32), near: f32) -> bool {
    cursor.0 >= island.0 - near
        && cursor.0 <= island.2 + near
        && cursor.1 >= island.1 - near
        && cursor.1 <= island.3 + near
}

/// The poll goes on after this reading: the button is down (a drag may be
/// on its way; the release ends it), or the pointer is near and the trigger
/// was less than `LINGER` ago.
pub fn keep_polling(button_down: bool, near: bool, since_trigger: Duration) -> bool {
    button_down || (near && since_trigger < LINGER)
}

/// The poll's pace: fast only while the left button is down and the pointer
/// is within `FAST_REACH` × `NEAR_DIP` (`close`); slow otherwise.
pub fn poll_ms(button_down: bool, close: bool) -> u32 {
    if button_down && close {
        FAST_MS
    } else {
        SLOW_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: (f32, f32, f32, f32) = (400.0, 0.0, 600.0, 36.0);

    #[test]
    fn near_is_a_box_round_the_island() {
        assert!(is_near((500.0, 2.0), ISLAND, 100.0));
        assert!(is_near((310.0, 100.0), ISLAND, 100.0));
        assert!(!is_near((200.0, 2.0), ISLAND, 100.0), "too far aside");
        assert!(!is_near((500.0, 300.0), ISLAND, 100.0), "too far below");
    }

    #[test]
    fn the_poll_stops_once_nothing_can_start_a_drag() {
        let s = Duration::from_secs;
        // A drag on its way: wherever the pointer is, until the release.
        assert!(keep_polling(true, false, s(60)));
        // Near, just after the trigger: it waits for a press.
        assert!(keep_polling(false, true, s(1)));
        // Near but long after the trigger (a resting pointer): it stops.
        assert!(!keep_polling(false, true, LINGER));
        // Gone away with the button up: it stops at once.
        assert!(!keep_polling(false, false, Duration::ZERO));
    }

    #[test]
    fn the_poll_is_fast_only_for_a_drag_close_by() {
        assert_eq!(poll_ms(true, true), FAST_MS);
        assert_eq!(poll_ms(true, false), SLOW_MS, "a drag far away");
        assert_eq!(poll_ms(false, true), SLOW_MS, "button up");
        assert_eq!(poll_ms(false, false), SLOW_MS);
        // Close reaches three times as far as near.
        let close = |x| is_near((x, 10.0), ISLAND, 100.0 * FAST_REACH);
        assert!(close(150.0) && !is_near((150.0, 10.0), ISLAND, 100.0));
        assert!(!close(50.0));
    }
}
