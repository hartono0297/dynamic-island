//! Layout math for the island, in logical pixels (96 DPI). Pure functions, so
//! the renderer only draws what this module decides.

use crate::config::Style;
use crate::model::{Hit, Notice, NoticeKind};

pub const WINDOW_W: f32 = 620.0;
pub const WINDOW_H: f32 = 330.0;
pub const ANIM_MS: u64 = 380;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Compact,
    MediaExpanded,
    Notice,
}

/// Pill floats this far below the top edge; Notch is flush with it.
const PILL_TOP: f32 = 8.0;

pub fn shape_for(style: Style, kind: ShapeKind) -> ShapeRect {
    let (w, h) = match kind {
        ShapeKind::Compact => (190.0, 34.0),
        ShapeKind::MediaExpanded => (380.0, 110.0),
        ShapeKind::Notice => (360.0, 76.0),
    };
    let compact = kind == ShapeKind::Compact;
    let (y, radius) = match style {
        Style::Pill => (PILL_TOP, if compact { h / 2.0 } else { 26.0 }),
        Style::Notch => (0.0, if compact { 16.0 } else { 22.0 }),
    };
    ShapeRect {
        x: (WINDOW_W - w) / 2.0,
        y,
        w,
        h,
        radius,
    }
}

/// Linear blend; `t` outside 0..=1 extrapolates, which is how the eased
/// overshoot carries past the target shape.
pub fn lerp_shape(a: ShapeRect, b: ShapeRect, t: f32) -> ShapeRect {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    ShapeRect {
        x: mix(a.x, b.x),
        y: mix(a.y, b.y),
        w: mix(a.w, b.w),
        h: mix(a.h, b.h),
        radius: mix(a.radius, b.radius),
    }
}

/// Ease-out with a slight overshoot (peaks about 5 % past the target).
pub fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.2;
    const C3: f32 = C1 + 1.0;
    let u = t - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// Depth of the Notch's inward-curved top shoulders, drawn outside `w`.
pub const SHOULDER: f32 = 12.0;

/// Corner radius actually drawn: the stored radius clamped so mid-morph
/// (and overshooting) shapes never produce a self-crossing outline.
pub fn corner_radius(style: Style, s: ShapeRect) -> f32 {
    let vertical = match style {
        Style::Pill => s.h / 2.0,
        Style::Notch => s.h - SHOULDER,
    };
    s.radius.min(s.w / 2.0).min(vertical).max(0.0)
}

/// Whether a logical point lies inside the main shape (shoulders excluded).
pub fn shape_contains(style: Style, s: ShapeRect, x: f32, y: f32) -> bool {
    if x < s.x || x >= s.x + s.w || y < s.y || y >= s.y + s.h {
        return false;
    }
    let r = corner_radius(style, s);
    // Corner circle centers; a point beyond one of them on both axes must lie
    // within that corner's circle.
    let cx = if x < s.x + r {
        s.x + r
    } else if x > s.x + s.w - r {
        s.x + s.w - r
    } else {
        return true;
    };
    let top_rounded = style == Style::Pill;
    let cy = if top_rounded && y < s.y + r {
        s.y + r
    } else if y > s.y + s.h - r {
        s.y + s.h - r
    } else {
        return true;
    };
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Compact media: album art, artist, equalizer bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompactLayout {
    pub art: Rect,
    pub text: Rect,
    pub bars: Rect,
}

/// Expanded media: album art, title over artist, transport buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpandedLayout {
    pub art: Rect,
    pub title: Rect,
    pub artist: Rect,
    /// Prev, play/pause, next.
    pub buttons: [Rect; 3],
}

/// Notice: round badge, title over body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoticeLayout {
    pub badge: Rect,
    pub title: Rect,
    pub body: Rect,
}

/// Equalizer: 4 bars, 3 px wide, 2 px apart, up to 14 px tall.
pub const BAR_COUNT: usize = 4;
pub const BAR_W: f32 = 3.0;
pub const BAR_GAP: f32 = 2.0;
pub const BAR_MAX_H: f32 = 14.0;

/// Text box between `left` and `right`; never negative while a shape is still small.
fn text_box(left: f32, right: f32, y: f32, h: f32) -> Rect {
    Rect {
        x: left,
        y,
        w: (right - left).max(0.0),
        h,
    }
}

fn square(x: f32, y: f32, size: f32) -> Rect {
    Rect {
        x,
        y,
        w: size,
        h: size,
    }
}

pub fn compact_layout(s: ShapeRect) -> CompactLayout {
    const ART: f32 = 22.0;
    const PAD_L: f32 = 6.0;
    const PAD_R: f32 = 14.0;
    const GAP: f32 = 10.0;
    let art = square(s.x + PAD_L, s.y + (s.h - ART) / 2.0, ART);
    let bars_w = BAR_COUNT as f32 * BAR_W + (BAR_COUNT - 1) as f32 * BAR_GAP;
    let bars = Rect {
        x: s.x + s.w - PAD_R - bars_w,
        y: s.y + (s.h - BAR_MAX_H) / 2.0,
        w: bars_w,
        h: BAR_MAX_H,
    };
    let text = text_box(art.right() + GAP, bars.x - GAP, s.y, s.h);
    CompactLayout { art, text, bars }
}

pub fn expanded_layout(s: ShapeRect) -> ExpandedLayout {
    const ART: f32 = 40.0;
    const PAD: f32 = 14.0;
    const PAD_R: f32 = 18.0;
    const BUTTON_W: f32 = 48.0;
    const BUTTON_H: f32 = 36.0;
    const BUTTON_STEP: f32 = 60.0;
    let art = square(s.x + PAD, s.y + PAD, ART);
    let left = art.right() + 12.0;
    let right = s.x + s.w - PAD_R;
    let title = text_box(left, right, art.y, 20.0);
    let artist = text_box(left, right, art.y + 20.0, 18.0);
    let cx = s.x + s.w / 2.0;
    let cy = s.y + s.h - 26.0;
    let button = |dx: f32| Rect {
        x: cx + dx - BUTTON_W / 2.0,
        y: cy - BUTTON_H / 2.0,
        w: BUTTON_W,
        h: BUTTON_H,
    };
    ExpandedLayout {
        art,
        title,
        artist,
        buttons: [button(-BUTTON_STEP), button(0.0), button(BUTTON_STEP)],
    }
}

pub fn notice_layout(s: ShapeRect) -> NoticeLayout {
    const BADGE: f32 = 40.0;
    const PAD_L: f32 = 18.0;
    const PAD_R: f32 = 20.0;
    let badge = square(s.x + PAD_L, s.y + (s.h - BADGE) / 2.0, BADGE);
    let left = badge.right() + 12.0;
    let right = s.x + s.w - PAD_R;
    let mid = s.y + s.h / 2.0;
    NoticeLayout {
        badge,
        title: text_box(left, right, mid - 19.0, 19.0),
        body: text_box(left, right, mid + 1.0, 18.0),
    }
}

/// What the last frame drew, for mapping clicks back to targets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitRegions {
    pub shape: Option<(Style, ShapeRect)>,
    pub buttons: Option<[Rect; 3]>,
}

impl HitRegions {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let (style, shape) = self.shape?;
        if !shape_contains(style, shape, x, y) {
            return None;
        }
        let targets = [Hit::Prev, Hit::PlayPause, Hit::Next];
        let button = self
            .buttons
            .and_then(|b| b.iter().position(|r| r.contains(x, y)));
        Some(button.map_or(Hit::Body, |i| targets[i].clone()))
    }
}

/// Badge fill as 0xRRGGBB: WhatsApp green, Claude orange, battery amber, else grey.
pub fn badge_color(notice: &Notice) -> u32 {
    match notice.kind {
        NoticeKind::ClaudeDone { .. } | NoticeKind::ClaudeWaiting { .. } => 0xD97757,
        NoticeKind::BatteryLow => 0xF5A524,
        NoticeKind::App { .. } if notice.source.to_lowercase().contains("whatsapp") => 0x25D366,
        NoticeKind::App { .. } => 0x5A5A5E,
    }
}

/// First character of the source, uppercased (empty when the source is).
pub fn badge_letter(source: &str) -> String {
    source
        .trim_start()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(source: &str, kind: NoticeKind) -> Notice {
        Notice {
            key: "k".into(),
            source: source.into(),
            title: "t".into(),
            body: "b".into(),
            kind,
        }
    }

    fn inside(outer: ShapeRect, r: Rect) -> bool {
        r.x >= outer.x
            && r.y >= outer.y
            && r.right() <= outer.x + outer.w
            && r.bottom() <= outer.y + outer.h
    }

    #[test]
    fn radius_is_clamped_for_small_shapes() {
        let mut s = shape_for(Style::Pill, ShapeKind::Notice);
        s.h = 20.0;
        assert_eq!(corner_radius(Style::Pill, s), 10.0);
        // Notch sides start below the shoulder, so the radius leaves room for it.
        assert_eq!(corner_radius(Style::Notch, s), 20.0 - SHOULDER);
        let c = shape_for(Style::Pill, ShapeKind::Compact);
        assert_eq!(corner_radius(Style::Pill, c), 17.0);
    }

    #[test]
    fn pill_corners_are_outside_notch_top_corners_inside() {
        let p = shape_for(Style::Pill, ShapeKind::Notice);
        assert!(!shape_contains(Style::Pill, p, p.x + 1.0, p.y + 1.0));
        assert!(shape_contains(Style::Pill, p, p.x + p.w / 2.0, p.y + 1.0));
        assert!(!shape_contains(Style::Pill, p, p.x - 1.0, p.y + p.h / 2.0));
        let n = shape_for(Style::Notch, ShapeKind::Notice);
        assert!(shape_contains(Style::Notch, n, n.x + 1.0, 1.0));
        assert!(!shape_contains(Style::Notch, n, n.x + 1.0, n.h - 1.0));
        assert!(!shape_contains(Style::Notch, n, n.x + n.w / 2.0, n.h + 1.0));
    }

    #[test]
    fn layouts_fit_inside_their_shapes() {
        for style in [Style::Pill, Style::Notch] {
            let c = shape_for(style, ShapeKind::Compact);
            let l = compact_layout(c);
            for r in [l.art, l.text, l.bars] {
                assert!(inside(c, r), "{style:?} compact {r:?}");
            }
            assert_eq!(l.art.w, 22.0);
            assert!(l.text.x >= l.art.right() && l.text.right() <= l.bars.x);

            let e = shape_for(style, ShapeKind::MediaExpanded);
            let l = expanded_layout(e);
            for r in [
                l.art,
                l.title,
                l.artist,
                l.buttons[0],
                l.buttons[1],
                l.buttons[2],
            ] {
                assert!(inside(e, r), "{style:?} expanded {r:?}");
            }
            assert_eq!(l.art.w, 40.0);
            assert!(l.buttons[0].right() <= l.buttons[1].x);
            assert!(l.buttons[1].right() <= l.buttons[2].x);
            assert!(l.buttons[1].y >= l.art.bottom());

            let n = shape_for(style, ShapeKind::Notice);
            let l = notice_layout(n);
            for r in [l.badge, l.title, l.body] {
                assert!(inside(n, r), "{style:?} notice {r:?}");
            }
            assert_eq!((l.badge.w, l.badge.h), (40.0, 40.0));
            assert!(l.title.x >= l.badge.right() && l.title.bottom() <= l.body.y);
        }
    }

    #[test]
    fn hit_maps_buttons_body_and_outside() {
        let e = shape_for(Style::Pill, ShapeKind::MediaExpanded);
        let l = expanded_layout(e);
        let regions = HitRegions {
            shape: Some((Style::Pill, e)),
            buttons: Some(l.buttons),
        };
        let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (x, y) = center(l.buttons[0]);
        assert_eq!(regions.hit(x, y), Some(Hit::Prev));
        let (x, y) = center(l.buttons[1]);
        assert_eq!(regions.hit(x, y), Some(Hit::PlayPause));
        let (x, y) = center(l.buttons[2]);
        assert_eq!(regions.hit(x, y), Some(Hit::Next));
        let (x, y) = center(l.art);
        assert_eq!(regions.hit(x, y), Some(Hit::Body));
        assert_eq!(regions.hit(2.0, 2.0), None);
        assert_eq!(HitRegions::default().hit(x, y), None);
    }

    #[test]
    fn badge_colors_by_source_and_kind() {
        let app = |s: &str| notice(s, NoticeKind::App { app_id: None });
        assert_eq!(badge_color(&app("WhatsApp")), 0x25D366);
        assert_eq!(badge_color(&app("whatsapp beta")), 0x25D366);
        assert_eq!(badge_color(&app("Discord")), 0x5A5A5E);
        let done = NoticeKind::ClaudeDone {
            session_id: "s".into(),
        };
        assert_eq!(badge_color(&notice("Claude", done)), 0xD97757);
        let waiting = NoticeKind::ClaudeWaiting {
            session_id: "s".into(),
        };
        assert_eq!(badge_color(&notice("Claude", waiting)), 0xD97757);
        assert_eq!(
            badge_color(&notice("Baterai", NoticeKind::BatteryLow)),
            0xF5A524
        );
    }

    #[test]
    fn badge_letter_is_first_char_uppercased() {
        assert_eq!(badge_letter("whatsApp"), "W");
        assert_eq!(badge_letter("ärzte"), "Ä");
        assert_eq!(badge_letter(""), "");
    }

    #[test]
    fn pill_compact_is_centered_below_top() {
        assert_eq!(
            shape_for(Style::Pill, ShapeKind::Compact),
            ShapeRect {
                x: 215.0,
                y: 8.0,
                w: 190.0,
                h: 34.0,
                radius: 17.0
            }
        );
    }

    #[test]
    fn notch_is_flush_with_top() {
        assert_eq!(shape_for(Style::Notch, ShapeKind::Notice).y, 0.0);
    }

    #[test]
    fn sizes_match_plan() {
        for style in [Style::Pill, Style::Notch] {
            let e = shape_for(style, ShapeKind::MediaExpanded);
            assert_eq!((e.w, e.h), (380.0, 110.0));
            let n = shape_for(style, ShapeKind::Notice);
            assert_eq!((n.w, n.h), (360.0, 76.0));
            let c = shape_for(style, ShapeKind::Compact);
            assert_eq!((c.w, c.h), (190.0, 34.0));
            for s in [e, n, c] {
                assert_eq!(s.x * 2.0 + s.w, WINDOW_W, "{style:?} {s:?} not centered");
            }
        }
    }

    #[test]
    fn radii_match_plan() {
        assert_eq!(shape_for(Style::Pill, ShapeKind::Notice).radius, 26.0);
        assert_eq!(shape_for(Style::Notch, ShapeKind::Compact).radius, 16.0);
        assert_eq!(
            shape_for(Style::Notch, ShapeKind::MediaExpanded).radius,
            22.0
        );
    }

    #[test]
    fn ease_endpoints_and_overshoot() {
        assert!(ease_out_back(0.0).abs() < 1e-6);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-6);
        let peak = (0..=100)
            .map(|i| ease_out_back(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.1);
    }

    #[test]
    fn lerp_halfway() {
        let a = shape_for(Style::Pill, ShapeKind::Compact);
        let b = shape_for(Style::Pill, ShapeKind::MediaExpanded);
        let m = lerp_shape(a, b, 0.5);
        assert_eq!(m.w, (a.w + b.w) / 2.0);
        assert_eq!(m.x * 2.0 + m.w, WINDOW_W);
        assert_eq!(lerp_shape(a, b, 0.0), a);
        assert_eq!(lerp_shape(a, b, 1.0), b);
    }
}
