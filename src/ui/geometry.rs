//! Layout math for the island, in logical pixels (96 DPI). Pure functions, so
//! the renderer only draws what this module decides.

use super::icons::Icon;
use super::panel_layout::Region;
use crate::config::Style;
use crate::i18n::Key;
use crate::model::{Hit, Notice, NoticeKind};
use crate::pomodoro::{Phase, PomoStatus};
use crate::theme::{Palette, radius};

pub const WINDOW_W: f32 = 620.0;
pub const WINDOW_H: f32 = 330.0;
pub const ANIM_MS: u64 = 240;

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
    Idle,
    MediaCompact,
    /// The compact media view showing a lyric line: wider, same height.
    MediaLyric,
    PomodoroCompact,
    Notice,
    Panel,
}

/// Pill floats this far below the top edge; Notch is flush with it.
const PILL_TOP: f32 = 8.0;
/// Bottom corners of the Notch's compact shapes.
const NOTCH_COMPACT_RADIUS: f32 = 16.0;

pub fn shape_for(style: Style, kind: ShapeKind) -> ShapeRect {
    let (w, h) = match kind {
        ShapeKind::Idle | ShapeKind::MediaCompact => (190.0, 34.0),
        ShapeKind::PomodoroCompact => (210.0, 34.0),
        ShapeKind::MediaLyric => (300.0, 34.0),
        ShapeKind::Notice => (360.0, 76.0),
        ShapeKind::Panel => (560.0, 290.0),
    };
    // The panel and notices round the same in both styles; compact shapes
    // are a full pill floating, a gentler curve hanging from the edge.
    let radius = match kind {
        ShapeKind::Panel => radius::PANEL,
        ShapeKind::Notice => radius::NOTICE,
        _ if style == Style::Pill => h / 2.0,
        _ => NOTCH_COMPACT_RADIUS,
    };
    let y = match style {
        Style::Pill => PILL_TOP,
        Style::Notch => 0.0,
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

/// Ease-out with a slight overshoot (peaks about 2 % past the target).
pub fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 0.7;
    const C3: f32 = C1 + 1.0;
    // Exact at the start: f32 rounding would otherwise leave t = 0 a hair off 0.
    if t <= 0.0 {
        return 0.0;
    }
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

/// Media page: big album art; beside it a caption, title over artist, and
/// the transport buttons with "Lirik" at the row's right end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpandedLayout {
    pub art: Rect,
    /// "SEDANG DIPUTAR" above the title.
    pub label: Rect,
    pub title: Rect,
    pub artist: Rect,
    /// Prev, play/pause (the round primary button), next.
    pub buttons: [Rect; 3],
    /// "Lirik": lyrics in the notch on or off.
    pub lyrics: Rect,
}

/// Notice: rounded badge, title over body, an optional action chip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoticeLayout {
    pub badge: Rect,
    pub title: Rect,
    pub body: Rect,
    pub action: Option<Rect>,
}

/// Equalizer: 4 bars, 3 px wide, 2 px apart, up to 14 px tall.
pub const BAR_COUNT: usize = 4;
pub const BAR_W: f32 = 3.0;
pub const BAR_GAP: f32 = 2.0;
pub const BAR_MAX_H: f32 = 14.0;

/// Equalizer bar heights `t` seconds into playback: each bar follows
/// `BAR_MAX_H × (0.35 + 0.65 × |sin(2π·f·t + φ)|)`. Paused bars rest at the
/// middle of that range.
pub fn equalizer_heights(t: f32, playing: bool) -> [f32; BAR_COUNT] {
    const FREQ: [f32; BAR_COUNT] = [1.7, 2.3, 1.3, 2.9];
    const PHASE: [f32; BAR_COUNT] = [0.0, 1.1, 2.3, 0.6];
    std::array::from_fn(|i| {
        let wave = if playing {
            (std::f32::consts::TAU * FREQ[i] * t + PHASE[i]).sin().abs()
        } else {
            0.5
        };
        BAR_MAX_H * (0.35 + 0.65 * wave)
    })
}

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

/// The lyric line's box: the shape less its side padding (wider than the
/// corner radius, so trimmed text never runs into the rounded ends).
pub fn lyric_layout(s: ShapeRect) -> Rect {
    const PAD_X: f32 = 20.0;
    text_box(s.x + PAD_X, s.x + s.w - PAD_X, s.y, s.h)
}

/// `action`: room for the chip naming what a click does ("Selesai",
/// "Mulai") at the right, the text stopping short of it.
pub fn notice_layout(s: ShapeRect, action: bool) -> NoticeLayout {
    const BADGE: f32 = 40.0;
    const PAD_L: f32 = 18.0;
    const PAD_R: f32 = 20.0;
    const ACTION_W: f32 = 66.0;
    const ACTION_H: f32 = 24.0;
    const ACTION_GAP: f32 = 10.0;
    let badge = square(s.x + PAD_L, s.y + (s.h - BADGE) / 2.0, BADGE);
    let left = badge.right() + 12.0;
    let mid = s.y + s.h / 2.0;
    let edge = s.x + s.w - PAD_R;
    let action = action.then(|| Rect {
        x: edge - ACTION_W,
        y: mid - ACTION_H / 2.0,
        w: ACTION_W,
        h: ACTION_H,
    });
    let right = action.map_or(edge, |a| a.x - ACTION_GAP);
    NoticeLayout {
        badge,
        title: text_box(left, right, mid - 18.0, 18.0),
        body: text_box(left, right, mid + 1.0, 16.0),
        action,
    }
}

/// What the last frame drew, for mapping clicks back to targets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitRegions {
    pub shape: Option<(Style, ShapeRect)>,
    pub buttons: Option<[Rect; 3]>,
    /// Panel regions from `panel_layout::regions`; checked before `buttons`.
    pub regions: Vec<Region>,
}

impl HitRegions {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let (style, shape) = self.shape?;
        if !shape_contains(style, shape, x, y) {
            return None;
        }
        if let Some(region) = self.regions.iter().find(|r| r.rect.contains(x, y)) {
            return Some(region.hit.clone());
        }
        let targets = [Hit::Prev, Hit::PlayPause, Hit::Next];
        let button = self
            .buttons
            .and_then(|b| b.iter().position(|r| r.contains(x, y)));
        Some(button.map_or(Hit::Body, |i| targets[i].clone()))
    }
}

/// Claude's brand orange: the waiting strip's dot.
pub const CLAUDE: u32 = 0xD97757;
/// WhatsApp's brand green.
pub const WHATSAPP: u32 = 0x25D366;
/// How strongly a tinted badge shows its colour over the island.
const BADGE_TINT: f32 = 0.2;

/// A notice badge: a rounded square of `fill` at `alpha`, and either `icon`
/// or the source's first letter in `ink`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BadgeStyle {
    pub fill: u32,
    pub alpha: f32,
    pub ink: u32,
    pub icon: Option<Icon>,
}

/// Reminders on a solid accent with a bell; the island's own notices on a
/// tint of their colour (Claude the accent, battery ochre, pomodoro the
/// colour of `pomo_next`, the phase a click starts); apps by their letter,
/// WhatsApp on its green tint, others on `raised_hi`.
pub fn badge_style(notice: &Notice, p: &Palette, pomo_next: Phase) -> BadgeStyle {
    let tint = |color: u32, icon| BadgeStyle {
        fill: color,
        alpha: BADGE_TINT,
        ink: color,
        icon: Some(icon),
    };
    match notice.kind {
        NoticeKind::ClaudeDone { .. } | NoticeKind::ClaudeWaiting { .. } => {
            tint(p.accent, Icon::Sparkle)
        }
        NoticeKind::BatteryLow => tint(p.ochre, Icon::Battery),
        NoticeKind::Pomodoro => tint(p.phase(pomo_next), Icon::Timer),
        NoticeKind::Reminder { .. } => BadgeStyle {
            fill: p.accent,
            alpha: 1.0,
            ink: p.on_accent,
            icon: Some(Icon::Bell),
        },
        NoticeKind::App { .. } if notice.source.to_lowercase().contains("whatsapp") => BadgeStyle {
            icon: None,
            ..tint(WHATSAPP, Icon::Mail)
        },
        NoticeKind::App { .. } => BadgeStyle {
            fill: p.raised_hi,
            alpha: 1.0,
            ink: p.text,
            icon: None,
        },
    }
}

/// The chip naming what a click on the notice does, only where the click
/// does it: a reminder is marked done, a pomodoro notice starts the next
/// phase while that phase still waits (`Ready`). Claude's notices get none:
/// a click only dismisses them, nothing opens.
pub fn notice_action(kind: &NoticeKind, pomo: PomoStatus) -> Option<Key> {
    match kind {
        NoticeKind::Reminder { .. } => Some(Key::MarkDone),
        NoticeKind::Pomodoro if pomo == PomoStatus::Ready => Some(Key::Start),
        _ => None,
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
    fn panel_centered_both_styles() {
        let p = shape_for(Style::Pill, ShapeKind::Panel);
        assert_eq!(
            p,
            ShapeRect {
                x: 30.0,
                y: 8.0,
                w: 560.0,
                h: 290.0,
                radius: 30.0
            }
        );
        let n = shape_for(Style::Notch, ShapeKind::Panel);
        assert_eq!(
            (n.x, n.y, n.w, n.h, n.radius),
            (30.0, 0.0, 560.0, 290.0, 30.0)
        );
        // The panel fits the window with the Pill's top gap.
        assert!(p.y + p.h <= WINDOW_H);
        assert_eq!(corner_radius(Style::Notch, n), 30.0);
        assert_eq!(corner_radius(Style::Pill, p), 30.0);
    }

    #[test]
    fn idle_matches_v1_compact_size() {
        for style in [Style::Pill, Style::Notch] {
            let idle = shape_for(style, ShapeKind::Idle);
            assert_eq!(idle, shape_for(style, ShapeKind::MediaCompact));
            assert_eq!((idle.w, idle.h), (190.0, 34.0));
        }
        assert_eq!(shape_for(Style::Pill, ShapeKind::Idle).radius, 17.0);
        assert_eq!(shape_for(Style::Notch, ShapeKind::Idle).radius, 16.0);
    }

    #[test]
    fn radius_is_clamped_for_small_shapes() {
        let mut s = shape_for(Style::Pill, ShapeKind::Notice);
        s.h = 20.0;
        assert_eq!(corner_radius(Style::Pill, s), 10.0);
        // Notch sides start below the shoulder, so the radius leaves room for it.
        assert_eq!(corner_radius(Style::Notch, s), 20.0 - SHOULDER);
        let c = shape_for(Style::Pill, ShapeKind::Idle);
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
            let c = shape_for(style, ShapeKind::MediaCompact);
            let l = compact_layout(c);
            for r in [l.art, l.text, l.bars] {
                assert!(inside(c, r), "{style:?} compact {r:?}");
            }
            assert_eq!(l.art.w, 22.0);
            assert!(l.text.x >= l.art.right() && l.text.right() <= l.bars.x);

            let m = shape_for(style, ShapeKind::MediaLyric);
            let text = lyric_layout(m);
            assert!(inside(m, text), "{style:?} lyric {text:?}");
            assert_eq!((text.w, text.h), (260.0, 34.0));
            assert!(text.x - m.x >= corner_radius(style, m));

            let n = shape_for(style, ShapeKind::Notice);
            let l = notice_layout(n, false);
            for r in [l.badge, l.title, l.body] {
                assert!(inside(n, r), "{style:?} notice {r:?}");
            }
            assert_eq!((l.badge.w, l.badge.h), (40.0, 40.0));
            assert!(l.title.x >= l.badge.right() && l.title.bottom() <= l.body.y);
            assert_eq!(l.action, None);
            // With an action chip the text stops short of it.
            let a = notice_layout(n, true);
            let chip = a.action.expect("action chip");
            assert!(inside(n, chip), "{style:?} chip {chip:?}");
            assert!(a.title.right() < chip.x && a.body.right() < chip.x);
            assert!(chip.right() <= n.x + n.w - corner_radius(style, n) / 2.0);
            assert_eq!((a.badge, a.title.x), (l.badge, l.title.x));
        }
    }

    #[test]
    fn hit_maps_buttons_body_and_outside() {
        let e = shape_for(Style::Pill, ShapeKind::Panel);
        let l = super::super::panel_layout::media_panel_layout(e, false);
        let regions = HitRegions {
            shape: Some((Style::Pill, e)),
            buttons: Some(l.buttons),
            ..Default::default()
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
    fn hit_maps_panel_regions() {
        let p = shape_for(Style::Notch, ShapeKind::Panel);
        let strip = Rect {
            x: p.x + 20.0,
            y: 8.0,
            w: 100.0,
            h: 24.0,
        };
        let regions = HitRegions {
            shape: Some((Style::Notch, p)),
            regions: vec![Region {
                rect: strip,
                hit: Hit::Strip,
            }],
            ..Default::default()
        };
        assert_eq!(regions.hit(strip.x + 1.0, strip.y + 1.0), Some(Hit::Strip));
        assert_eq!(regions.hit(p.x + p.w / 2.0, 200.0), Some(Hit::Body));
        assert_eq!(regions.hit(p.x - 5.0, 200.0), None);
    }

    #[test]
    fn badge_colors_by_source_and_kind() {
        use crate::theme::{Accent, Surface, island};
        let p = island(Accent::Plum, Surface::Espresso);
        let style = |n: &Notice| badge_style(n, &p, Phase::ShortBreak);
        let fill = |n: &Notice| {
            let b = style(n);
            (b.fill, b.ink, b.icon)
        };
        let app = |s: &str| notice(s, NoticeKind::App { app_id: None });
        assert_eq!(fill(&app("WhatsApp")), (WHATSAPP, WHATSAPP, None));
        assert_eq!(fill(&app("whatsapp beta")), (WHATSAPP, WHATSAPP, None));
        assert_eq!(fill(&app("Discord")), (p.raised_hi, p.text, None));
        assert_eq!(style(&app("Discord")).alpha, 1.0);
        let done = NoticeKind::ClaudeDone {
            session_id: "s".into(),
        };
        // Claude on a tint of the accent, as in the design.
        let claude = (0xB07A9A, 0xB07A9A, Some(Icon::Sparkle));
        assert_eq!(fill(&notice("Claude", done)), claude);
        let waiting = NoticeKind::ClaudeWaiting {
            session_id: "s".into(),
        };
        assert_eq!(fill(&notice("Claude", waiting)), claude);
        let battery = notice("Baterai", NoticeKind::BatteryLow);
        assert_eq!(fill(&battery), (0xD9A441, 0xD9A441, Some(Icon::Battery)));
        assert!(style(&battery).alpha < 0.5, "a tint");
        // Pomodoro: the colour of the phase a click starts.
        let pomo = notice("Pomodoro", NoticeKind::Pomodoro);
        assert_eq!(fill(&pomo), (0x9DB59A, 0x9DB59A, Some(Icon::Timer)));
        let focus = badge_style(&pomo, &p, Phase::Focus);
        assert_eq!((focus.fill, focus.ink), (p.accent, p.accent));
        // Reminders follow the accent, solid, with its ink.
        let reminder = NoticeKind::Reminder { id: "r".into() };
        let r = style(&notice("Pengingat", reminder));
        assert_eq!((r.fill, r.alpha), (0xB07A9A, 1.0));
        assert_eq!((r.ink, r.icon), (p.on_accent, Some(Icon::Bell)));
    }

    #[test]
    fn notice_chip_only_where_a_click_acts() {
        let reminder = NoticeKind::Reminder { id: "r".into() };
        assert_eq!(
            notice_action(&reminder, PomoStatus::Idle),
            Some(Key::MarkDone)
        );
        assert_eq!(
            notice_action(&NoticeKind::Pomodoro, PomoStatus::Ready),
            Some(Key::Start)
        );
        // The next phase was already started elsewhere: a click starts nothing.
        let started = [
            PomoStatus::Idle,
            PomoStatus::Running {
                ends_at: std::time::Instant::now(),
            },
            PomoStatus::Paused {
                remaining: std::time::Duration::from_secs(60),
            },
        ];
        for status in started {
            assert_eq!(notice_action(&NoticeKind::Pomodoro, status), None);
        }
        // A click on a Claude notice only dismisses it.
        let done = NoticeKind::ClaudeDone {
            session_id: "s".into(),
        };
        let waiting = NoticeKind::ClaudeWaiting {
            session_id: "s".into(),
        };
        assert_eq!(notice_action(&done, PomoStatus::Ready), None);
        assert_eq!(notice_action(&waiting, PomoStatus::Ready), None);
        assert_eq!(
            notice_action(&NoticeKind::BatteryLow, PomoStatus::Ready),
            None
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
            shape_for(Style::Pill, ShapeKind::MediaCompact),
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
            let size = |kind| {
                let s = shape_for(style, kind);
                assert_eq!(s.x * 2.0 + s.w, WINDOW_W, "{style:?} {s:?} not centered");
                (s.w, s.h)
            };
            assert_eq!(size(ShapeKind::Idle), (190.0, 34.0));
            assert_eq!(size(ShapeKind::MediaCompact), (190.0, 34.0));
            assert_eq!(size(ShapeKind::PomodoroCompact), (210.0, 34.0));
            assert_eq!(size(ShapeKind::MediaLyric), (300.0, 34.0));
            assert_eq!(size(ShapeKind::Notice), (360.0, 76.0));
            assert_eq!(size(ShapeKind::Panel), (560.0, 290.0));
        }
    }

    #[test]
    fn radii_match_plan() {
        let radius = |style, kind| shape_for(style, kind).radius;
        assert_eq!(radius(Style::Pill, ShapeKind::Notice), 26.0);
        assert_eq!(radius(Style::Pill, ShapeKind::Panel), 30.0);
        assert_eq!(radius(Style::Pill, ShapeKind::PomodoroCompact), 17.0);
        assert_eq!(radius(Style::Notch, ShapeKind::MediaCompact), 16.0);
        assert_eq!(radius(Style::Notch, ShapeKind::PomodoroCompact), 16.0);
        assert_eq!(radius(Style::Pill, ShapeKind::MediaLyric), 17.0);
        assert_eq!(radius(Style::Notch, ShapeKind::MediaLyric), 16.0);
        assert_eq!(radius(Style::Notch, ShapeKind::Notice), 26.0);
        assert_eq!(radius(Style::Notch, ShapeKind::Panel), 30.0);
        // Drawn as stored: no clamping at rest.
        for style in [Style::Pill, Style::Notch] {
            for kind in [ShapeKind::Notice, ShapeKind::Panel] {
                let s = shape_for(style, kind);
                assert_eq!(corner_radius(style, s), s.radius, "{style:?} {kind:?}");
            }
        }
    }

    #[test]
    fn equalizer_follows_formula_and_rests_when_paused() {
        let at0 = equalizer_heights(0.0, true);
        let expect = |phase: f32| BAR_MAX_H * (0.35 + 0.65 * phase.sin().abs());
        for (h, phase) in at0.iter().zip([0.0, 1.1, 2.3, 0.6]) {
            assert!((h - expect(phase)).abs() < 1e-4);
        }
        // Bar 0 (1.7 Hz) peaks a quarter period in.
        let peak = equalizer_heights(1.0 / (4.0 * 1.7), true)[0];
        assert!((peak - BAR_MAX_H).abs() < 1e-3);
        for i in 0..200 {
            for h in equalizer_heights(i as f32 * 0.033, true) {
                assert!((0.35 * BAR_MAX_H - 1e-4..=BAR_MAX_H + 1e-4).contains(&h));
            }
        }
        let paused = equalizer_heights(12.3, false);
        for h in paused {
            assert!((h - BAR_MAX_H * 0.675).abs() < 1e-4);
        }
        assert_eq!(paused, equalizer_heights(0.0, false), "static");
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
        let a = shape_for(Style::Pill, ShapeKind::Idle);
        let b = shape_for(Style::Pill, ShapeKind::Panel);
        let m = lerp_shape(a, b, 0.5);
        assert_eq!(m.w, (a.w + b.w) / 2.0);
        assert_eq!(m.x * 2.0 + m.w, WINDOW_W);
        assert_eq!(lerp_shape(a, b, 0.0), a);
        assert_eq!(lerp_shape(a, b, 1.0), b);
    }
}
