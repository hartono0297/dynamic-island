//! When the island wears its faint drag margin. Windows offers a drag only to
//! the pixels a layered window draws, and the top edge of the screen is where
//! a file is aimed; but a margin that is always there swallows clicks on
//! whatever lies under it (a maximized window's tab strip). So it is drawn
//! only while a drag can be about to start: the left button is down and the
//! pointer is near the island. A drag from Explorer holds the button the
//! whole way, so the margin is there when the pointer arrives.

/// How often the button and the pointer are read, while the island is up.
pub const POLL_MS: u32 = 50;
/// How near the island the pointer must be, in logical pixels.
pub const NEAR_DIP: f32 = 100.0;

/// `island` is (left, top, right, bottom) and `cursor` a point, both in the
/// same unit as `near`.
pub fn halo_wanted(
    button_down: bool,
    cursor: (f32, f32),
    island: (f32, f32, f32, f32),
    near: f32,
) -> bool {
    button_down
        && cursor.0 >= island.0 - near
        && cursor.0 <= island.2 + near
        && cursor.1 >= island.1 - near
        && cursor.1 <= island.3 + near
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: (f32, f32, f32, f32) = (400.0, 0.0, 600.0, 36.0);

    #[test]
    fn the_margin_needs_the_button_down_and_the_pointer_near() {
        assert!(halo_wanted(true, (500.0, 2.0), ISLAND, 100.0));
        assert!(halo_wanted(true, (310.0, 100.0), ISLAND, 100.0));
        assert!(!halo_wanted(false, (500.0, 2.0), ISLAND, 100.0), "no button");
        assert!(!halo_wanted(true, (200.0, 2.0), ISLAND, 100.0), "too far aside");
        assert!(!halo_wanted(true, (500.0, 300.0), ISLAND, 100.0), "too far below");
    }
}
