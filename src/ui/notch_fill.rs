//! The closed island's background: solid black (as always) or see-through.
//! Pure alpha math, so the renderer and the app only ask for numbers.
//!
//! Scope: the compact views (idle, music, lyric, timer) and the notices use
//! `fill_alpha`; the open panel always draws solid (alpha 1).

use crate::config::NotchFill;

/// The lowest alpha a see-through fill is drawn at, 3 / 255 (8-bit alpha 3).
/// A layered window only takes the pointer on pixels with alpha above 0, so a
/// notch set to 0 % must still be hovered and clicked; the same idea as the
/// drag margin's `HALO_ALPHA` in `render.rs`, which is invisible too.
pub const FILL_FLOOR: f32 = 3.0 / 255.0;

/// From this alpha up the fill is dark enough to read on, so no text shadow.
const LEGIBLE_FROM: f32 = 0.85;

/// The surface fill's alpha for the closed island: 1 for Solid, otherwise
/// `opacity_pct` % (clamped to 0..=100) but never below `FILL_FLOOR`.
pub fn fill_alpha(fill: NotchFill, opacity_pct: u8) -> f32 {
    match fill {
        NotchFill::Solid => 1.0,
        NotchFill::Transparent => (f32::from(opacity_pct.min(100)) / 100.0).max(FILL_FLOOR),
    }
}

/// The fill's alpha part-way through a morph from `from` to `to`, `t` the
/// eased progress (clamped, so the overshoot never leaves 0..=1).
pub fn morph_fill_alpha(from: f32, to: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    from + (to - from) * t
}

/// The progress to blend with when the island shrinks from a more solid fill
/// into a see-through one (a panel closing into the notch): the panel's
/// content is still on screen for most of the morph, so the fill holds solid
/// and turns see-through over the last 30 %, with the compact content fading
/// in. 0 until 70 % of the way, 1 at the end.
pub fn closing_fill_t(t: f32) -> f32 {
    ((t - 0.7) / 0.3).clamp(0.0, 1.0)
}

/// The fill's alpha at eased progress `t` of a morph from `from` to `to`:
/// growing more solid (opening a panel) follows the morph, becoming more
/// see-through (closing it) waits for the end (`closing_fill_t`).
pub fn morph_fill(from: f32, to: f32, t: f32) -> f32 {
    if to < from {
        morph_fill_alpha(from, to, closing_fill_t(t))
    } else {
        morph_fill_alpha(from, to, t)
    }
}

/// How strong the text shadow is for a fill of `alpha`: 0 from 0.85 up (and
/// for Solid), rising linearly to 1 at 0.
pub fn legibility(alpha: f32) -> f32 {
    ((LEGIBLE_FROM - alpha.clamp(0.0, 1.0)) / LEGIBLE_FROM).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_is_fully_opaque_at_any_opacity() {
        for pct in [0, 30, 100, 255] {
            assert_eq!(fill_alpha(NotchFill::Solid, pct), 1.0);
        }
    }

    #[test]
    fn transparent_follows_the_percentage_above_the_floor() {
        assert_eq!(fill_alpha(NotchFill::Transparent, 100), 1.0);
        assert_eq!(fill_alpha(NotchFill::Transparent, 200), 1.0);
        assert!((fill_alpha(NotchFill::Transparent, 30) - 0.3).abs() < 1e-6);
        assert!((fill_alpha(NotchFill::Transparent, 50) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zero_percent_is_still_a_pixel_the_window_owns() {
        let a = fill_alpha(NotchFill::Transparent, 0);
        assert!(a > 0.0 && a == FILL_FLOOR);
        // 8-bit alpha 3: what the layered window needs to take the pointer.
        assert_eq!((a * 255.0).round() as u8, 3);
        // And too faint to see.
        assert!(a < 0.02);
    }

    #[test]
    fn the_alpha_never_falls_as_the_opacity_rises() {
        let mut last = 0.0;
        for pct in 0..=100u8 {
            let a = fill_alpha(NotchFill::Transparent, pct);
            assert!(a >= last, "{pct}");
            last = a;
        }
    }

    #[test]
    fn a_morph_runs_from_one_alpha_to_the_other_and_never_past_them() {
        assert_eq!(morph_fill_alpha(0.3, 1.0, 0.0), 0.3);
        assert_eq!(morph_fill_alpha(0.3, 1.0, 1.0), 1.0);
        assert!((morph_fill_alpha(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        // The eased overshoot is clamped.
        assert_eq!(morph_fill_alpha(0.3, 1.0, 1.02), 1.0);
        assert_eq!(morph_fill_alpha(0.3, 1.0, -0.1), 0.3);
        assert_eq!(morph_fill_alpha(1.0, 0.3, 1.02), 0.3);
    }

    #[test]
    fn opening_follows_the_morph_and_closing_waits_for_the_end() {
        let a = 0.3;
        // Opening: more solid as the morph goes.
        assert_eq!(morph_fill(a, 1.0, 0.0), a);
        assert!(morph_fill(a, 1.0, 0.5) > a);
        assert_eq!(morph_fill(a, 1.0, 1.0), 1.0);
        // Closing: solid while the panel's content is still there.
        assert_eq!(morph_fill(1.0, a, 0.0), 1.0);
        assert_eq!(morph_fill(1.0, a, 0.5), 1.0);
        assert_eq!(morph_fill(1.0, a, 0.7), 1.0);
        assert!(morph_fill(1.0, a, 0.85) < 1.0);
        assert!((morph_fill(1.0, a, 1.0) - a).abs() < 1e-6);
        // Never pops: continuous and steady in the last stretch.
        let mut last = 1.0;
        for i in 0..=100 {
            let v = morph_fill(1.0, a, i as f32 / 100.0);
            assert!(v <= last + 1e-6 && last - v < 0.05, "{i}");
            last = v;
        }
        // Same on both ends: nothing moves.
        assert_eq!(morph_fill(1.0, 1.0, 0.4), 1.0);
        assert_eq!(morph_fill(a, a, 0.4), a);
    }

    #[test]
    fn the_shadow_is_off_for_solid_and_full_at_zero() {
        assert_eq!(legibility(fill_alpha(NotchFill::Solid, 30)), 0.0);
        assert_eq!(legibility(1.0), 0.0);
        assert_eq!(legibility(0.85), 0.0);
        assert_eq!(legibility(0.9), 0.0);
        assert_eq!(legibility(0.0), 1.0);
        // Rising as the fill thins.
        let mut last = 0.0;
        for i in (0..=100).rev() {
            let l = legibility(i as f32 / 100.0);
            assert!(l >= last, "{i}");
            last = l;
        }
        assert!(legibility(FILL_FLOOR) > 0.98);
        assert!(legibility(0.3) > 0.5 && legibility(0.3) < 1.0);
    }
}
