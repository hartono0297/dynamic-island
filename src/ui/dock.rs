//! The page dots as a dock: hovered, they become a row of page icons that
//! magnify under the pointer like the macOS Dock. This is the pure model
//! (no drawing, no timers); `render_panel` draws from a `DockFrame`, `App`
//! holds the `Dock` and feeds it the pointer.
//!
//! Two rules keep it steady:
//! - The resting layout (`resting_layout`) is the only source of the icon
//!   centres (the falloff) and of the click rects. It never depends on a
//!   scale, so an icon that grows under the pointer does not move its own
//!   hit target or the falloff of its neighbours (no feedback shake).
//! - Mouse moves only set targets. The displayed values advance once a frame
//!   (`advance`) by a frame-rate independent exponential ease, so the motion
//!   is the same at 30 or 144 Hz and never overshoots.

use std::f32::consts::PI;

use super::geometry::{Rect, ShapeRect};
use super::panel_layout::{DOT_BOTTOM, Region, page_dots};
use crate::model::{Hit, Page};

/// An icon at rest, in dip.
pub const DOCK_ICON: f32 = 14.0;
/// Distance between neighbouring icon centres at rest.
pub const DOCK_PITCH: f32 = 28.0;
/// How far from an icon's centre the pointer still magnifies it.
pub const REACH: f32 = 56.0;
/// The extra scale at the peak: the icon under the pointer is `1 + PEAK`.
pub const PEAK: f32 = 0.9;
/// Time constant of the ease, in seconds.
pub const TAU: f32 = 0.070;
/// A value this close to its target snaps to it.
pub const SNAP: f32 = 0.002;
/// The hover zone is the row's bounds grown by this much.
pub const ZONE_PAD_X: f32 = 8.0;
pub const ZONE_PAD_Y: f32 = 10.0;
/// Clicks only count once the row is more icons than dots.
pub const CLICK_FROM: f32 = 0.5;

/// The icons of `n` pages at rest: `DOCK_ICON` squares at `DOCK_PITCH`,
/// centred on the panel like the dots, their bottom on the dots' line.
pub fn resting_layout(s: ShapeRect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let total = (n - 1) as f32 * DOCK_PITCH + DOCK_ICON;
    let x0 = s.x + (s.w - total) / 2.0;
    let y = s.y + s.h - DOT_BOTTOM - DOCK_ICON;
    (0..n)
        .map(|i| Rect {
            x: x0 + i as f32 * DOCK_PITCH,
            y,
            w: DOCK_ICON,
            h: DOCK_ICON,
        })
        .collect()
}

/// How strongly an icon `d` dip from the pointer is magnified: 1 at 0,
/// 0.5 halfway, 0 from `REACH` on (a cosine bump).
pub fn falloff(d: f32) -> f32 {
    let d = d.abs();
    if d >= REACH {
        0.0
    } else {
        0.5 * (1.0 + (PI * d / REACH).cos())
    }
}

/// The scale each icon of `layout` aims for with the pointer at `pointer_x`.
pub fn targets(pointer_x: f32, layout: &[Rect]) -> Vec<f32> {
    layout
        .iter()
        .map(|r| 1.0 + PEAK * falloff(pointer_x - (r.x + r.w / 2.0)))
        .collect()
}

/// The bounds of a row of rects.
fn bounds(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let last = rects.last()?;
    let top = rects.iter().map(|r| r.y).fold(f32::INFINITY, f32::min);
    let bottom = rects.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
    Some(Rect {
        x: first.x,
        y: top,
        w: last.right() - first.x,
        h: bottom - top,
    })
}

fn grown(r: Rect) -> Rect {
    Rect {
        x: r.x - ZONE_PAD_X,
        y: r.y - ZONE_PAD_Y,
        w: r.w + 2.0 * ZONE_PAD_X,
        h: r.h + 2.0 * ZONE_PAD_Y,
    }
}

/// Where the dock reacts to the pointer on the last frame: the dots as
/// drawn and the resting icons. Both come from pure layouts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockZone {
    pub dots: Vec<Rect>,
    pub icons: Vec<Rect>,
}

impl DockZone {
    /// The pointer is in the zone: the dots' bounds grown by the padding,
    /// or, once expanded, the resting icons' bounds grown by the same and
    /// reaching up over the click cells (the hysteresis). An icon at its
    /// peak and the top of its cell stand above the padded row, so without
    /// the cells a pointer on a magnified icon's top would collapse it.
    pub fn contains(&self, x: f32, y: f32, expanded: bool) -> bool {
        if !expanded {
            return bounds(&self.dots).is_some_and(|b| grown(b).contains(x, y));
        }
        bounds(&self.icons).is_some_and(|b| grown(b).contains(x, y))
            || cells(&self.icons).any(|c| c.contains(x, y))
    }
}

/// The dock's part of a panel frame: its zone, and its click regions only
/// when the page takes clicks (`clicks`). The zone stands also while a page
/// slides or cross-fades in, so the row under a still pointer stays icons
/// through the slide its own click (or the wheel) started.
pub fn panel_parts(
    s: ShapeRect,
    pages: &[Page],
    current: Page,
    e: f32,
    clicks: bool,
) -> (DockZone, Vec<Region>) {
    let icons = resting_layout(s, pages.len());
    let hits = if clicks {
        regions(&icons, pages, e)
    } else {
        Vec::new()
    };
    let zone = DockZone {
        dots: page_dots(s, pages, current).0,
        icons,
    };
    (zone, hits)
}

/// Where item `i` of the row is drawn: its dot rect lerped to its resting
/// icon rect by `e`, then magnified by `scale` from its bottom edge around
/// its centre x. At `e` 0 and scale 1 this is the dot.
pub fn morph_rect(dot: Rect, icon: Rect, e: f32, scale: f32) -> Rect {
    let e = e.clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * e;
    let cx = lerp(dot.x + dot.w / 2.0, icon.x + icon.w / 2.0);
    let bottom = lerp(dot.bottom(), icon.bottom());
    let w = lerp(dot.w, icon.w) * scale;
    let h = lerp(dot.h, icon.h) * scale;
    Rect {
        x: cx - w / 2.0,
        y: bottom - h,
        w,
        h,
    }
}

/// What a frame draws of the dock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockFrame {
    /// 0 dots, 1 icons.
    pub e: f32,
    /// Each icon's scale; missing entries are 1.
    pub scales: Vec<f32>,
}

impl DockFrame {
    pub fn scale(&self, i: usize) -> f32 {
        self.scales.get(i).copied().unwrap_or(1.0)
    }
}

/// The displayed values and what they ease to.
#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    /// Dots-to-icons progress, 0 to 1.
    pub e: f32,
    pub scales: Vec<f32>,
    pub targets: Vec<f32>,
    /// The pointer's x while it is in the zone; `None` collapses the row.
    pub hover: Option<f32>,
}

impl Default for Dock {
    fn default() -> Self {
        Self::new()
    }
}

/// One step of the exponential ease: frame-rate independent, never past
/// the target, snapping when close.
fn ease(value: f32, target: f32, dt: f32) -> f32 {
    let next = value + (target - value) * (1.0 - (-dt.max(0.0) / TAU).exp());
    if (target - next).abs() < SNAP {
        target
    } else {
        next
    }
}

impl Dock {
    pub fn new() -> Self {
        Dock {
            e: 0.0,
            scales: Vec::new(),
            targets: Vec::new(),
            hover: None,
        }
    }

    /// Whether the row is (or is becoming) icons.
    pub fn expanded(&self) -> bool {
        self.hover.is_some()
    }

    fn e_target(&self) -> f32 {
        if self.hover.is_some() { 1.0 } else { 0.0 }
    }

    /// Sets the pointer (`None`: it left the zone) and the targets; nothing
    /// is displayed differently until `advance`. True when a target changed.
    pub fn hover(&mut self, pointer_x: Option<f32>, layout: &[Rect]) -> bool {
        let new_targets = match pointer_x {
            Some(x) => targets(x, layout),
            None => vec![1.0; layout.len()],
        };
        let changed = self.hover.is_some() != pointer_x.is_some() || new_targets != self.targets;
        self.hover = pointer_x;
        self.scales.resize(new_targets.len(), 1.0);
        self.targets = new_targets;
        changed
    }

    /// One frame of `dt` seconds. Without animation the values jump.
    pub fn advance(&mut self, dt: f32, animate: bool) {
        let e_target = self.e_target();
        self.scales.resize(self.targets.len(), 1.0);
        if !animate {
            self.e = e_target;
            self.scales.clone_from(&self.targets);
            return;
        }
        self.e = ease(self.e, e_target, dt);
        for (v, t) in self.scales.iter_mut().zip(&self.targets) {
            *v = ease(*v, *t, dt);
        }
    }

    /// Some displayed value has not reached its target: the frame clock
    /// has to keep running.
    pub fn moving(&self) -> bool {
        self.e != self.e_target()
            || self.scales.len() != self.targets.len()
            || self.scales.iter().zip(&self.targets).any(|(v, t)| v != t)
    }

    /// Back to dots at once (the panel closed).
    pub fn reset(&mut self) {
        *self = Dock::new();
    }

    /// What a frame draws.
    pub fn frame(&self) -> DockFrame {
        DockFrame {
            e: self.e,
            scales: self.scales.clone(),
        }
    }

    /// Which icon a click at (`x`, `y`) is on, by the resting `layout`
    /// alone (never the displayed scales). Each icon owns its pitch-wide
    /// cell and reaches up as far as it can grow, so what is seen is what
    /// is hit. The renderer registers the same cells as click regions
    /// (`regions`); this is their pure lookup.
    #[cfg(test)]
    pub fn hit(&self, x: f32, y: f32, layout: &[Rect]) -> Option<usize> {
        cells(layout).position(|c| c.contains(x, y))
    }
}

/// The click cells of a resting layout.
fn cells(layout: &[Rect]) -> impl Iterator<Item = Rect> + '_ {
    let side = (DOCK_PITCH - DOCK_ICON) / 2.0;
    let up = DOCK_ICON * PEAK + 2.0;
    layout.iter().map(move |r| Rect {
        x: r.x - side,
        y: r.y - up,
        w: r.w + 2.0 * side,
        h: r.h + up + 3.0,
    })
}

/// The icons' click regions, one per page, while the row is more icons than
/// dots. The page the panel shows has its cell too, so a click on its icon
/// never falls through to the page content under it; `go_to_page` ignores
/// it (and the linger arms nothing for a page that did not turn).
pub fn regions(layout: &[Rect], pages: &[Page], e: f32) -> Vec<Region> {
    if e <= CLICK_FROM {
        return Vec::new();
    }
    cells(layout)
        .zip(pages)
        .map(|(rect, p)| Region {
            rect,
            hit: Hit::PanelPage(*p),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Style;
    use crate::ui::geometry::{ShapeKind, shape_for};
    use crate::ui::panel_layout::page_dots;

    fn panel() -> ShapeRect {
        shape_for(Style::Pill, ShapeKind::Panel)
    }

    fn settle(d: &mut Dock) {
        for _ in 0..1000 {
            d.advance(0.016, true);
            if !d.moving() {
                return;
            }
        }
    }

    #[test]
    fn falloff_is_a_monotonic_cosine_bump() {
        assert_eq!(falloff(0.0), 1.0);
        assert!((falloff(28.0) - 0.5).abs() < 1e-6);
        assert_eq!(falloff(56.0), 0.0);
        assert_eq!(falloff(300.0), 0.0);
        assert_eq!(falloff(-28.0), falloff(28.0));
        let mut last = 1.0;
        for step in 1..=60 {
            let v = falloff(step as f32);
            assert!(v <= last + 1e-6, "monotonic at {step}");
            last = v;
        }
    }

    #[test]
    fn resting_layout_is_stable_centred_and_in_order() {
        let s = panel();
        for n in [1, 4, 12] {
            let l = resting_layout(s, n);
            assert_eq!(l.len(), n);
            for r in &l {
                assert_eq!((r.w, r.h), (DOCK_ICON, DOCK_ICON));
                assert_eq!(r.bottom(), s.y + s.h - DOT_BOTTOM);
            }
            for pair in l.windows(2) {
                assert_eq!(pair[1].x - pair[0].x, DOCK_PITCH);
            }
            let left = l[0].x - s.x;
            let right = s.x + s.w - l[n - 1].right();
            assert!((left - right).abs() < 1e-4, "centred");
            // The same bottom line as the dots.
            let shown = vec![Page::Calendar; n];
            let (dots, _) = page_dots(s, &shown, Page::Calendar);
            assert_eq!(dots[0].bottom(), l[0].bottom());
            // The same centre as the dots' row.
            let dots_mid = (dots[0].x + dots[n - 1].right()) / 2.0;
            let mid = (l[0].x + l[n - 1].right()) / 2.0;
            assert!((dots_mid - mid).abs() < 1e-4);
        }
        assert!(resting_layout(s, 0).is_empty());
    }

    #[test]
    fn resting_layout_ignores_the_pointer_and_scales() {
        let s = panel();
        let before = resting_layout(s, 8);
        let mut d = Dock::new();
        d.hover(Some(before[3].x + 7.0), &before);
        settle(&mut d);
        assert!(d.scales.iter().any(|v| *v > 1.5));
        assert_eq!(resting_layout(s, 8), before);
    }

    #[test]
    fn hit_does_not_read_the_scales() {
        let layout = resting_layout(panel(), 8);
        let mut flat = Dock::new();
        flat.hover(Some(0.0), &layout);
        flat.scales = vec![1.0; 8];
        let mut big = flat.clone();
        big.scales = vec![1.9; 8];
        let mid = |i: usize| (layout[i].x + 7.0, layout[i].y + 7.0);
        for i in 0..8 {
            let (x, y) = mid(i);
            assert_eq!(flat.hit(x, y, &layout), Some(i));
            assert_eq!(big.hit(x, y, &layout), Some(i));
        }
        // Gaps between icons belong to a neighbour, above and below the
        // row nothing is hit.
        for (x, y) in [
            (layout[0].x - 7.5, layout[0].y + 7.0),
            (layout[3].x + 20.0, layout[3].y - 30.0),
            (layout[3].x + 7.0, layout[3].bottom() + 8.0),
        ] {
            assert_eq!(flat.hit(x, y, &layout), big.hit(x, y, &layout));
        }
        assert_eq!(
            flat.hit(layout[0].x - 20.0, layout[0].y + 7.0, &layout),
            None
        );
    }

    #[test]
    fn smoothing_converges_without_overshoot() {
        let layout = resting_layout(panel(), 6);
        for dt in [0.004, 0.008, 0.016, 0.033, 0.1, 0.5] {
            let mut d = Dock::new();
            d.hover(Some(layout[2].x + 7.0), &layout);
            assert!(d.moving());
            let mut last_e = 0.0;
            let mut last: Vec<f32> = vec![1.0; 6];
            for _ in 0..4000 {
                d.advance(dt, true);
                assert!(d.e >= last_e && d.e <= 1.0, "e rises to 1");
                for (i, v) in d.scales.iter().enumerate() {
                    assert!(*v >= last[i] - 1e-6 && *v <= d.targets[i] + 1e-6);
                }
                last_e = d.e;
                last = d.scales.clone();
                if !d.moving() {
                    break;
                }
            }
            assert!(!d.moving(), "settles at dt {dt}");
            assert_eq!(d.e, 1.0);
            assert_eq!(d.scales, d.targets);
            // And back to dots.
            d.hover(None, &layout);
            assert!(d.moving());
            settle(&mut d);
            assert_eq!((d.e, d.scales.clone()), (0.0, vec![1.0; 6]));
        }
    }

    #[test]
    fn smoothing_is_frame_rate_independent() {
        let layout = resting_layout(panel(), 6);
        let run = |dt: f32, steps: usize| {
            let mut d = Dock::new();
            d.hover(Some(layout[2].x + 7.0), &layout);
            for _ in 0..steps {
                d.advance(dt, true);
            }
            d.e
        };
        // 96 ms either way.
        assert!((run(0.016, 6) - run(0.008, 12)).abs() < 1e-4);
    }

    #[test]
    fn a_still_pointer_is_not_moving_once_settled() {
        let layout = resting_layout(panel(), 6);
        let mut d = Dock::new();
        assert!(!d.moving());
        let x = layout[1].x + 7.0;
        assert!(d.hover(Some(x), &layout));
        settle(&mut d);
        assert!(!d.moving());
        // The same pointer again changes nothing and starts nothing.
        assert!(!d.hover(Some(x), &layout));
        assert!(!d.moving());
        assert!(d.hover(Some(x + 5.0), &layout));
        assert!(d.moving());
    }

    #[test]
    fn without_animations_values_snap() {
        let layout = resting_layout(panel(), 6);
        let mut d = Dock::new();
        d.hover(Some(layout[2].x + 7.0), &layout);
        d.advance(0.0, false);
        assert_eq!(d.e, 1.0);
        assert_eq!(d.scales, d.targets);
        assert!(!d.moving());
        d.hover(None, &layout);
        d.advance(0.0, false);
        assert_eq!((d.e, d.scales.clone()), (0.0, vec![1.0; 6]));
    }

    #[test]
    fn reset_is_at_once() {
        let layout = resting_layout(panel(), 6);
        let mut d = Dock::new();
        d.hover(Some(layout[2].x), &layout);
        settle(&mut d);
        d.reset();
        assert_eq!((d.e, d.hover), (0.0, None));
        assert!(!d.moving());
    }

    #[test]
    fn neighbours_never_overlap_at_peak() {
        // The pointer on any icon, and between two: no icon's grown box
        // reaches the next one's.
        let layout = resting_layout(panel(), 12);
        let mut xs: Vec<f32> = layout.iter().map(|r| r.x + 7.0).collect();
        xs.extend(layout.windows(2).map(|w| (w[0].x + w[1].x) / 2.0 + 7.0));
        for x in xs {
            let t = targets(x, &layout);
            for (i, pair) in layout.windows(2).enumerate() {
                let right = pair[0].x + 7.0 + DOCK_ICON * t[i] / 2.0;
                let left = pair[1].x + 7.0 - DOCK_ICON * t[i + 1] / 2.0;
                assert!(
                    right <= left + 1e-4,
                    "icons {i} and {} overlap at {x}",
                    i + 1
                );
            }
        }
        assert!((targets(layout[3].x + 7.0, &layout)[3] - (1.0 + PEAK)).abs() < 1e-6);
    }

    #[test]
    fn the_expanded_zone_holds_the_peak_icons_and_their_cells() {
        let s = panel();
        let shown = vec![Page::Calendar; 6];
        let icons = resting_layout(s, 6);
        let zone = DockZone {
            dots: page_dots(s, &shown, Page::Calendar).0,
            icons: icons.clone(),
        };
        for (i, icon) in icons.iter().enumerate() {
            let cx = icon.x + icon.w / 2.0;
            // The top of an icon at its peak, and of its click cell.
            let peak_top = icon.bottom() - DOCK_ICON * (1.0 + PEAK);
            assert!(zone.contains(cx, peak_top + 0.1, true), "peak top of {i}");
            for c in cells(&icons) {
                for (x, y) in [
                    (c.x + 0.1, c.y + 0.1),
                    (c.right() - 0.1, c.y + 0.1),
                    (c.x + 0.1, c.bottom() - 0.1),
                ] {
                    assert!(zone.contains(x, y, true), "cell corner ({x}, {y})");
                }
            }
        }
        // Collapsed, the dots' zone is as small as before.
        let top = icons[0].bottom() - DOCK_ICON * (1.0 + PEAK);
        assert!(!zone.contains(icons[0].x + 7.0, top, false));
    }

    #[test]
    fn the_zone_stands_while_a_page_slides_and_only_the_clicks_wait() {
        let s = panel();
        let shown = [Page::Calendar, Page::Media, Page::Clipboard];
        let (zone, hits) = panel_parts(s, &shown, Page::Media, 1.0, false);
        assert!(hits.is_empty(), "no clicks on a page still sliding in");
        let icon = zone.icons[1];
        assert!(zone.contains(icon.x + 7.0, icon.y + 7.0, true));
        assert_eq!(zone.dots, page_dots(s, &shown, Page::Media).0);
        let (settled, hits) = panel_parts(s, &shown, Page::Media, 1.0, true);
        assert_eq!(settled, zone);
        assert_eq!(hits.len(), shown.len());
    }

    #[test]
    fn the_current_icon_shadows_the_content_under_it() {
        // A click anywhere on the current page's cell (its magnified icon)
        // is the dock's, never the page content behind it.
        let s = panel();
        let shown = [Page::Calendar, Page::Clipboard, Page::Media];
        let (zone, hits) = panel_parts(s, &shown, Page::Clipboard, 1.0, true);
        let icon = zone.icons[1];
        let top = icon.bottom() - DOCK_ICON * (1.0 + PEAK);
        for (x, y) in [(icon.x + 7.0, top + 0.5), (icon.x + 7.0, icon.y + 7.0)] {
            let hit = hits
                .iter()
                .find(|r| r.rect.contains(x, y))
                .map(|r| r.hit.clone());
            assert_eq!(hit, Some(Hit::PanelPage(Page::Clipboard)));
        }
    }

    #[test]
    fn morph_lerps_the_dot_rect_to_the_icon_rect() {
        let s = panel();
        let shown = vec![Page::Calendar; 4];
        let dots = page_dots(s, &shown, Page::Calendar).0;
        let icons = resting_layout(s, 4);
        let (dot, icon) = (dots[1], icons[1]);
        assert_eq!(morph_rect(dot, icon, 0.0, 1.0), dot);
        let at_rest = morph_rect(dot, icon, 1.0, 1.0);
        for (a, b) in [
            (at_rest.x, icon.x),
            (at_rest.y, icon.y),
            (at_rest.w, icon.w),
            (at_rest.h, icon.h),
        ] {
            assert!((a - b).abs() < 1e-4);
        }
        let half = morph_rect(dot, icon, 0.5, 1.0);
        assert!((half.w - (dot.w + icon.w) / 2.0).abs() < 1e-4);
        assert!((half.bottom() - icon.bottom()).abs() < 1e-4);
        let mid = |r: Rect| r.x + r.w / 2.0;
        assert!((mid(half) - (mid(dot) + mid(icon)) / 2.0).abs() < 1e-4);
        // Magnified: from the bottom edge, around the centre.
        let big = morph_rect(dot, icon, 1.0, 1.9);
        assert!((big.w - DOCK_ICON * 1.9).abs() < 1e-4);
        assert!((big.bottom() - icon.bottom()).abs() < 1e-4);
        assert!((mid(big) - mid(icon)).abs() < 1e-4);
    }

    #[test]
    fn zone_grows_from_the_dots_and_holds_around_the_icons() {
        let s = panel();
        let shown = vec![Page::Calendar; 6];
        let (dots, _) = page_dots(s, &shown, Page::Calendar);
        let zone = DockZone {
            dots: dots.clone(),
            icons: resting_layout(s, 6),
        };
        let d = dots[0];
        // Just inside the padding of the dots, not beyond it.
        assert!(zone.contains(d.x - 7.0, d.y + 2.0, false));
        assert!(!zone.contains(d.x - 9.0, d.y + 2.0, false));
        assert!(zone.contains(d.x + 2.0, d.y - 9.0, false));
        assert!(!zone.contains(d.x + 2.0, d.y - 11.0, false));
        // Once expanded the (wider, taller) row holds the pointer.
        let i = zone.icons[0];
        assert!(!zone.contains(i.x - 3.0, i.y + 2.0, false));
        assert!(zone.contains(i.x - 3.0, i.y + 2.0, true));
        assert!(!zone.contains(i.x - 9.0, i.y + 2.0, true));
        assert!(!DockZone::default().contains(0.0, 0.0, true));
    }

    #[test]
    fn clicks_follow_the_owners_order() {
        // A custom order with a hidden page: the row shows what the wheel
        // shows, so index i is the i-th shown page.
        let mut pages = crate::pages::PanelPages::default();
        pages.move_to(Page::Shortcuts, 0);
        pages.toggle(Page::Notes);
        let shown = pages.visible();
        assert_eq!(shown[0], Page::Shortcuts);
        assert!(!shown.contains(&Page::Notes));
        let layout = resting_layout(panel(), shown.len());
        let d = Dock::new();
        let regs = regions(&layout, &shown, 1.0);
        assert_eq!(regs.len(), shown.len());
        for (i, page) in shown.iter().enumerate() {
            let (x, y) = (layout[i].x + 7.0, layout[i].y + 7.0);
            let hit = d.hit(x, y, &layout).map(|i| shown[i]);
            assert_eq!(hit, Some(*page));
            let region = regs.iter().find(|r| r.rect.contains(x, y));
            assert_eq!(region.map(|r| r.hit.clone()), Some(Hit::PanelPage(*page)));
        }
    }

    #[test]
    fn clicks_wait_until_the_row_is_mostly_icons() {
        let shown = [Page::Calendar, Page::Notes];
        let layout = resting_layout(panel(), 2);
        assert!(regions(&layout, &shown, 0.5).is_empty());
        assert!(regions(&layout, &shown, 0.0).is_empty());
        assert_eq!(regions(&layout, &shown, 0.51).len(), 2);
    }
}
