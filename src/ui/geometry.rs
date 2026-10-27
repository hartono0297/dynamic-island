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
/// The window's height while the enlarged CLI panel shows or morphs: the
/// Pill's top gap, 440 px and the morph's overshoot.
pub const TALL_WINDOW_H: f32 = 460.0;
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
    /// The CLI page enlarged: the panel 440 px tall.
    PanelTall,
}

/// Pill floats this far below the top edge; Notch is flush with it.
const PILL_TOP: f32 = 8.0;
/// Bottom corners of the Notch's compact shapes.
const NOTCH_COMPACT_RADIUS: f32 = 16.0;

pub fn shape_for(style: Style, kind: ShapeKind) -> ShapeRect {
    let (w, h) = match kind {
        ShapeKind::Idle | ShapeKind::MediaCompact => (190.0, 34.0),
        ShapeKind::PomodoroCompact => (210.0, 34.0),
        // Sized by its line (`lyric_shape`); without one, the narrowest.
        ShapeKind::MediaLyric => (LYRIC_MIN_W, 34.0),
        ShapeKind::Notice => (360.0, 76.0),
        ShapeKind::Panel => (560.0, 290.0),
        ShapeKind::PanelTall => (560.0, 440.0),
    };
    // The panel and notices round the same in both styles; compact shapes
    // are a full pill floating, a gentler curve hanging from the edge.
    let radius = match kind {
        ShapeKind::Panel | ShapeKind::PanelTall => radius::PANEL,
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

/// Bata in a compact island: this big, this far in from the left edge, and
/// this far from what follows. On a notice it takes the badge's 40 px place.
pub const ROBOT_COMPACT: f32 = 26.0;
pub const ROBOT_NOTICE: f32 = 40.0;
const ROBOT_INSET: f32 = 6.0;
const ROBOT_GAP: f32 = 8.0;
/// The idle clock's box beside the robot: "00:00" in DM Sans 600 16,
/// tracked (40.3 px), fits.
pub const CLOCK_W: f32 = 44.0;

/// How much wider a compact shape is with the robot at its left, so its
/// content keeps the room it has without it. Idle holds the robot and the
/// clock as it is; a notice's robot takes the badge's place.
pub fn robot_extra(kind: ShapeKind) -> f32 {
    match kind {
        // The art (22 px, text from +38) becomes the robot (26 px, text from +40).
        ShapeKind::MediaCompact => 2.0,
        // `lyric_width` already counts the robot in its lead.
        // The ring started 14 px in; now after the robot, 40 px in.
        ShapeKind::PomodoroCompact => 26.0,
        _ => 0.0,
    }
}

/// `shape_for`, widened by `robot_extra` (and still centred) with the robot on.
pub fn shape_with_robot(style: Style, kind: ShapeKind, robot: bool) -> ShapeRect {
    let s = shape_for(style, kind);
    if !robot {
        return s;
    }
    let w = s.w + robot_extra(kind);
    ShapeRect {
        x: (WINDOW_W - w) / 2.0,
        w,
        ..s
    }
}

/// The robot's square at the left of a compact shape, vertically centred.
pub fn robot_slot(s: ShapeRect) -> Rect {
    square(
        s.x + ROBOT_INSET,
        s.y + (s.h - ROBOT_COMPACT) / 2.0,
        ROBOT_COMPACT,
    )
}

/// The idle island: the robot, the clock, both side by side (centred as a
/// group), or nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdleLayout {
    pub robot: Option<Rect>,
    /// The clock's box; its text is centred in it.
    pub clock: Option<Rect>,
}

pub fn idle_layout(s: ShapeRect, robot: bool, clock: bool) -> IdleLayout {
    let full = Rect {
        x: s.x,
        y: s.y,
        w: s.w,
        h: s.h,
    };
    let y = s.y + (s.h - ROBOT_COMPACT) / 2.0;
    match (robot, clock) {
        (false, clock) => IdleLayout {
            robot: None,
            clock: clock.then_some(full),
        },
        (true, false) => IdleLayout {
            robot: Some(square(s.x + (s.w - ROBOT_COMPACT) / 2.0, y, ROBOT_COMPACT)),
            clock: None,
        },
        (true, true) => {
            let group = ROBOT_COMPACT + ROBOT_GAP + CLOCK_W;
            let x = s.x + (s.w - group) / 2.0;
            IdleLayout {
                robot: Some(square(x, y, ROBOT_COMPACT)),
                clock: Some(Rect {
                    x: x + ROBOT_COMPACT + ROBOT_GAP,
                    y: s.y,
                    w: CLOCK_W,
                    h: s.h,
                }),
            }
        }
    }
}

/// Compact media with the robot in the album art's place (`art` is the
/// robot's square): title, then the equalizer.
pub fn compact_layout_robot(s: ShapeRect) -> CompactLayout {
    let robot = robot_slot(s);
    let plain = compact_layout(s);
    CompactLayout {
        art: robot,
        text: text_box(robot.right() + ROBOT_GAP, plain.bars.x - 10.0, s.y, s.h),
        bars: plain.bars,
    }
}

/// The lyric view with the robot: its square, then the line's box.
pub fn lyric_layout_robot(s: ShapeRect) -> (Rect, Rect) {
    let robot = robot_slot(s);
    let line = lyric_layout(s);
    let left = robot.right() + ROBOT_GAP;
    (robot, text_box(left, line.right(), s.y, s.h))
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

/// Equalizer (the design's `.eq`): 5 bars, 2.5 px wide, 2 px apart, up to
/// 13 px tall.
pub const BAR_COUNT: usize = 5;
pub const BAR_W: f32 = 2.5;
pub const BAR_GAP: f32 = 2.0;
pub const BAR_MAX_H: f32 = 13.0;

/// Equalizer bar heights `t` seconds into playback: each bar follows
/// `BAR_MAX_H × (0.35 + 0.65 × |sin(2π·f·t + φ)|)`. Paused bars rest at the
/// middle of that range. Every `f × 10 s` is a whole number of cycles, so the
/// render clock can wrap at 10 s without a jump.
pub fn equalizer_heights(t: f32, playing: bool) -> [f32; BAR_COUNT] {
    const FREQ: [f32; BAR_COUNT] = [1.7, 2.3, 1.3, 2.9, 2.1];
    const PHASE: [f32; BAR_COUNT] = [0.0, 1.1, 2.3, 0.6, 1.7];
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
/// corner radius, so the text never runs into the rounded ends).
pub fn lyric_layout(s: ShapeRect) -> Rect {
    text_box(s.x + LYRIC_PAD, s.x + s.w - LYRIC_PAD, s.y, s.h)
}

/// The lyric notch is as wide as its line, between these (logical px).
pub const LYRIC_MIN_W: f32 = 220.0;
pub const LYRIC_MAX_W: f32 = 560.0;
/// On a small screen the notch takes at most this share of its width.
pub const LYRIC_SCREEN_SHARE: f32 = 0.45;
/// A new line this much shorter (or more) shrinks the notch; less keeps it,
/// so it does not wobble on every line.
pub const LYRIC_SHRINK_SLACK: f32 = 40.0;
/// A width change while the lyric view stays.
pub const LYRIC_RESIZE_MS: u64 = 180;
/// A line too long for the widest notch waits this long, then scrolls.
pub const LYRIC_HOLD_MS: u64 = 1000;
/// How fast a long line scrolls.
pub const LYRIC_SCROLL_PX_PER_S: f32 = 30.0;
/// The faded strip at the right of a line running past its box. The scroll
/// stops with the line's end just left of it.
pub const LYRIC_FADE_W: f32 = 32.0;
/// The line's box: this far from the shape's sides (no robot).
const LYRIC_PAD: f32 = 20.0;
/// Room for the measure's rounding, so a measured line never trims.
const LYRIC_SLACK: f32 = 1.0;

/// Where the line's box starts, from the shape's left edge: after the side
/// padding, or after the robot (`lyric_layout_robot`).
fn lyric_lead(robot: bool) -> f32 {
    if robot {
        ROBOT_INSET + ROBOT_COMPACT + ROBOT_GAP
    } else {
        LYRIC_PAD
    }
}

/// The widest lyric notch on a screen `screen_w` logical px wide.
pub fn lyric_max_w(screen_w: f32) -> f32 {
    (LYRIC_SCREEN_SHARE * screen_w).clamp(LYRIC_MIN_W, LYRIC_MAX_W)
}

/// The notch's width for a line measured `text_w` px wide (`None`: the
/// instrumental rest "♪" or nothing, which takes the minimum).
pub fn lyric_width(text_w: Option<f32>, robot: bool, max_w: f32) -> f32 {
    let max_w = max_w.max(LYRIC_MIN_W);
    match text_w {
        Some(w) if w > 0.0 => {
            (w.ceil() + LYRIC_SLACK + lyric_lead(robot) + LYRIC_PAD).clamp(LYRIC_MIN_W, max_w)
        }
        _ => LYRIC_MIN_W,
    }
}

/// The line's box width in a lyric notch `w` wide.
pub fn lyric_box_w(w: f32, robot: bool) -> f32 {
    (w - lyric_lead(robot) - LYRIC_PAD).max(0.0)
}

/// Hysteresis: the width to show after `current` (`None` when the lyric
/// view just opened) for a line that wants `wanted`. Grows at once; shrinks
/// only by `LYRIC_SHRINK_SLACK` or more.
pub fn lyric_target_w(current: Option<f32>, wanted: f32) -> f32 {
    match current {
        Some(c) if wanted < c && c - wanted < LYRIC_SHRINK_SLACK => c,
        _ => wanted,
    }
}

/// The lyric notch `w` wide, centred, as tall and round as the compact shapes.
pub fn lyric_shape(style: Style, w: f32) -> ShapeRect {
    let s = shape_for(style, ShapeKind::MediaLyric);
    ShapeRect {
        x: (WINDOW_W - w) / 2.0,
        w,
        ..s
    }
}

/// A line `text_w` wide runs past its box `box_w` wide.
pub fn lyric_overflow(text_w: f32, box_w: f32) -> bool {
    text_w > box_w
}

/// How far a long line scrolls in all: until its end sits just left of the fade.
fn lyric_scroll_max(text_w: f32, box_w: f32) -> f32 {
    if lyric_overflow(text_w, box_w) {
        text_w - box_w + LYRIC_FADE_W
    } else {
        0.0
    }
}

/// How far the line is scrolled left `ms_since_line` after it showed: 0
/// through the hold, then `LYRIC_SCROLL_PX_PER_S` until its end; 0 for a
/// line that fits.
pub fn lyric_scroll_offset(text_w: f32, box_w: f32, ms_since_line: u64) -> f32 {
    let moving = ms_since_line.saturating_sub(LYRIC_HOLD_MS) as f32 / 1000.0;
    (moving * LYRIC_SCROLL_PX_PER_S).min(lyric_scroll_max(text_w, box_w))
}

/// When, after the line showed, its scroll ends; `None` when it fits.
pub fn lyric_scroll_end_ms(text_w: f32, box_w: f32) -> Option<u64> {
    let max = lyric_scroll_max(text_w, box_w);
    (max > 0.0).then(|| LYRIC_HOLD_MS + (max / LYRIC_SCROLL_PX_PER_S * 1000.0).ceil() as u64)
}

/// `action`: room for the chip naming what a click does ("Selesai",
/// "Mulai") at the right, the text stopping short of it.
pub fn notice_layout(s: ShapeRect, action: bool) -> NoticeLayout {
    // The robot takes exactly the badge's place.
    const BADGE: f32 = ROBOT_NOTICE;
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
    /// The CLI page's output, where the wheel scrolls instead of paging.
    pub cli_output: Option<Rect>,
}

impl HitRegions {
    /// Whether the wheel at this point scrolls the CLI output.
    pub fn over_cli_output(&self, x: f32, y: f32) -> bool {
        self.cli_output.is_some_and(|r| r.contains(x, y))
    }

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
        // Drawn by the robot (low); the badge shows when it is off.
        NoticeKind::SystemLow => tint(p.ochre, Icon::Activity),
        // Drawn by the robot; this badge only shows if it is turned off meanwhile.
        NoticeKind::Greeting => tint(p.accent, Icon::Sparkle),
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
        // The click opens the panel on the System page.
        NoticeKind::SystemLow => Some(Key::Open),
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
    fn tall_panel_fits_the_tall_window_even_overshooting() {
        let peak = (0..=100)
            .map(|i| ease_out_back(i as f32 / 100.0))
            .fold(0.0, f32::max);
        for style in [Style::Pill, Style::Notch] {
            let tall = shape_for(style, ShapeKind::PanelTall);
            assert_eq!(
                (tall.x, tall.w, tall.h, tall.radius),
                (30.0, 560.0, 440.0, 30.0)
            );
            assert!(tall.y + tall.h <= TALL_WINDOW_H);
            // Growing from any shape eases a little past the target.
            for from in [ShapeKind::Idle, ShapeKind::Panel, ShapeKind::Notice] {
                let s = lerp_shape(shape_for(style, from), tall, peak);
                assert!(s.y + s.h <= TALL_WINDOW_H, "{style:?} from {from:?}: {s:?}");
            }
            // The small window still holds the small panel's overshoot.
            let idle = shape_for(style, ShapeKind::Idle);
            let small = lerp_shape(idle, shape_for(style, ShapeKind::Panel), peak);
            assert!(small.y + small.h <= WINDOW_H, "{style:?}: {small:?}");
        }
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
            // 5 × 2.5 px bars with 4 × 2 px gaps, 13 px tall.
            assert_eq!((l.bars.w, l.bars.h), (20.5, 13.0));
            assert!(l.text.w >= 100.0, "{style:?} compact text {:?}", l.text);

            let m = shape_for(style, ShapeKind::MediaLyric);
            let text = lyric_layout(m);
            assert!(inside(m, text), "{style:?} lyric {text:?}");
            // Without a line: the narrowest notch.
            assert_eq!((text.w, text.h), (180.0, 34.0));
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

    /// Points along the robot head's rounded outline, drawn in `area`.
    fn head_outline(area: Rect) -> Vec<(f32, f32)> {
        let (h, r) = crate::ui::robot::head_rect(area);
        let mut out = Vec::new();
        for (cx, cy, a0) in [
            (h.x + r, h.y + r, 180.0f32),
            (h.right() - r, h.y + r, 270.0),
            (h.right() - r, h.bottom() - r, 0.0),
            (h.x + r, h.bottom() - r, 90.0),
        ] {
            for i in 0..=8 {
                let a = (a0 + i as f32 * 90.0 / 8.0).to_radians();
                out.push((cx + r * a.cos(), cy + r * a.sin()));
            }
        }
        out
    }

    fn intersects(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    #[test]
    fn robot_and_content_share_every_compact_shape() {
        for style in [Style::Pill, Style::Notch] {
            let head_inside = |s: ShapeRect, robot: Rect| {
                for (x, y) in head_outline(robot) {
                    assert!(
                        shape_contains(style, s, x, y),
                        "{style:?} {robot:?} ({x}, {y})"
                    );
                }
            };
            for (robot, clock) in [(true, true), (true, false), (false, true)] {
                let s = shape_with_robot(style, ShapeKind::Idle, robot);
                let l = idle_layout(s, robot, clock);
                assert_eq!(l.robot.is_some(), robot);
                assert_eq!(l.clock.is_some(), clock);
                if let Some(r) = l.robot {
                    assert_eq!((r.w, r.h), (ROBOT_COMPACT, ROBOT_COMPACT));
                    head_inside(s, r);
                }
                if let (Some(r), Some(c)) = (l.robot, l.clock) {
                    assert!(inside(s, c) && !intersects(r, c), "{style:?} {l:?}");
                    // Side by side, centred as a group.
                    let (left, right) = (r.x - s.x, s.x + s.w - c.right());
                    assert!((left - right).abs() < 0.01);
                }
            }

            let s = shape_with_robot(style, ShapeKind::MediaCompact, true);
            let l = compact_layout_robot(s);
            head_inside(s, l.art);
            for r in [l.text, l.bars] {
                assert!(
                    inside(s, r) && !intersects(l.art, r),
                    "{style:?} media {r:?}"
                );
            }
            assert!(l.text.right() <= l.bars.x);
            assert_eq!(
                l.text.w,
                compact_layout(shape_for(style, ShapeKind::MediaCompact))
                    .text
                    .w
            );

            // The lyric notch counts the robot in its width: the same line
            // gets the same box with or without it.
            for text_w in [None, Some(120.0), Some(300.0), Some(480.0)] {
                let on = lyric_shape(style, lyric_width(text_w, true, LYRIC_MAX_W));
                let off = lyric_shape(style, lyric_width(text_w, false, LYRIC_MAX_W));
                let (robot, text) = lyric_layout_robot(on);
                head_inside(on, robot);
                assert!(inside(on, text) && !intersects(robot, text));
                let plain = lyric_layout(off);
                assert!(inside(off, plain));
                if let Some(t) = text_w {
                    assert!(text.w >= t && plain.w >= t, "{style:?} {t}");
                    // Above the minimum, the robot only adds its own room.
                    if off.w > LYRIC_MIN_W {
                        assert_eq!(text.w, plain.w, "{style:?} {t}");
                    }
                }
            }
            let s = shape_with_robot(style, ShapeKind::MediaLyric, true);
            let (robot, text) = lyric_layout_robot(s);
            head_inside(s, robot);
            assert!(inside(s, text) && !intersects(robot, text));

            let s = shape_with_robot(style, ShapeKind::PomodoroCompact, true);
            let (robot, (cx, cy), r, text) =
                super::super::panel_layout::pomodoro_compact_layout_robot(s);
            head_inside(s, robot);
            let ring = Rect {
                x: cx - r - 1.5,
                y: cy - r - 1.5,
                w: 2.0 * r + 3.0,
                h: 2.0 * r + 3.0,
            };
            for part in [ring, text] {
                assert!(inside(s, part), "{style:?} pomodoro {part:?}");
                assert!(!intersects(robot, part), "{style:?} pomodoro {part:?}");
            }
            assert!(!intersects(ring, text));

            // A notice's robot is its badge: same 40 px square, text after it.
            let n = shape_with_robot(style, ShapeKind::Notice, true);
            let l = notice_layout(n, true);
            assert_eq!((l.badge.w, l.badge.h), (ROBOT_NOTICE, ROBOT_NOTICE));
            head_inside(n, l.badge);
            assert!(l.title.x >= l.badge.right());
        }
    }

    #[test]
    fn robot_widens_compact_shapes_only_when_on() {
        for style in [Style::Pill, Style::Notch] {
            for kind in [
                ShapeKind::Idle,
                ShapeKind::MediaCompact,
                ShapeKind::MediaLyric,
                ShapeKind::PomodoroCompact,
                ShapeKind::Notice,
                ShapeKind::Panel,
                ShapeKind::PanelTall,
            ] {
                // Off: exactly as before.
                assert_eq!(shape_with_robot(style, kind, false), shape_for(style, kind));
                let on = shape_with_robot(style, kind, true);
                assert_eq!(on.x * 2.0 + on.w, WINDOW_W, "{style:?} {kind:?} centred");
                assert_eq!(
                    (on.y, on.h),
                    (shape_for(style, kind).y, shape_for(style, kind).h)
                );
            }
            let w = |kind| shape_with_robot(style, kind, true).w;
            assert_eq!(w(ShapeKind::Idle), 190.0);
            assert_eq!(w(ShapeKind::MediaCompact), 192.0);
            // Sized by its line, robot included (`lyric_width`).
            assert_eq!(w(ShapeKind::MediaLyric), LYRIC_MIN_W);
            assert_eq!(w(ShapeKind::PomodoroCompact), 236.0);
            assert_eq!(w(ShapeKind::Notice), 360.0);
            assert_eq!(w(ShapeKind::Panel), 560.0, "never in the panel");
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
        let system = notice("Sistem", NoticeKind::SystemLow);
        assert_eq!(fill(&system), (0xD9A441, 0xD9A441, Some(Icon::Activity)));
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
        // Disk or RAM low: "Buka" opens the System page.
        assert_eq!(
            notice_action(&NoticeKind::SystemLow, PomoStatus::Idle),
            Some(Key::Open)
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
            assert_eq!(size(ShapeKind::MediaLyric), (220.0, 34.0));
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
        assert_eq!(at0.len(), 5);
        for (h, phase) in at0.iter().zip([0.0, 1.1, 2.3, 0.6, 1.7]) {
            assert!((h - expect(phase)).abs() < 1e-4);
        }
        // The render clock wraps at 10 s: every bar is back where it started.
        for t in [0.0, 0.37, 2.5, 7.9] {
            let (a, b) = (
                equalizer_heights(t, true),
                equalizer_heights(t + 10.0, true),
            );
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1e-3, "t={t}: {x} vs {y}");
            }
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

    #[test]
    fn lyric_width_stays_between_min_and_max() {
        for robot in [false, true] {
            // Instrumental "♪", nothing, or a failed measure: the minimum.
            assert_eq!(lyric_width(None, robot, LYRIC_MAX_W), LYRIC_MIN_W);
            assert_eq!(lyric_width(Some(0.0), robot, LYRIC_MAX_W), LYRIC_MIN_W);
            // A short line keeps the minimum; a huge one caps at the maximum.
            assert_eq!(lyric_width(Some(40.0), robot, LYRIC_MAX_W), LYRIC_MIN_W);
            assert_eq!(lyric_width(Some(2000.0), robot, LYRIC_MAX_W), LYRIC_MAX_W);
            assert_eq!(lyric_width(Some(2000.0), robot, 450.0), 450.0);
            // In between, the notch hugs its line: text, 1 px slack, lead, padding.
            let lead = if robot { 40.0 } else { 20.0 };
            assert_eq!(
                lyric_width(Some(300.4), robot, LYRIC_MAX_W),
                301.0 + 1.0 + lead + 20.0
            );
            for i in 0..200 {
                let w = lyric_width(Some(i as f32 * 4.0), robot, LYRIC_MAX_W);
                assert!((LYRIC_MIN_W..=LYRIC_MAX_W).contains(&w), "{w}");
            }
        }
        // A maximum below the minimum (a tiny screen) never undercuts it.
        assert_eq!(lyric_width(Some(500.0), false, 100.0), LYRIC_MIN_W);
    }

    #[test]
    fn lyric_max_follows_the_screen() {
        assert_eq!(lyric_max_w(1000.0), 450.0);
        // Wide screens cap at 560; tiny ones never go below 220.
        assert_eq!(lyric_max_w(1920.0), LYRIC_MAX_W);
        assert_eq!(lyric_max_w(3840.0), LYRIC_MAX_W);
        assert_eq!(lyric_max_w(400.0), LYRIC_MIN_W);
        assert_eq!(lyric_max_w(0.0), LYRIC_MIN_W);
        // 1366 × 0.45 = 614.7: still 560.
        assert_eq!(lyric_max_w(1366.0), LYRIC_MAX_W);
        assert!((lyric_max_w(1100.0) - 495.0).abs() < 1e-3);
    }

    #[test]
    fn lyric_width_shrinks_only_by_the_slack() {
        // Just opened: whatever the line wants.
        assert_eq!(lyric_target_w(None, 300.0), 300.0);
        assert_eq!(lyric_target_w(None, LYRIC_MIN_W), LYRIC_MIN_W);
        // Longer: grows at once, by any amount.
        assert_eq!(lyric_target_w(Some(300.0), 301.0), 301.0);
        assert_eq!(lyric_target_w(Some(300.0), 560.0), 560.0);
        // Up to 40 px shorter keeps the width; 40 or more shrinks.
        assert_eq!(lyric_target_w(Some(300.0), 300.0), 300.0);
        assert_eq!(lyric_target_w(Some(300.0), 265.0), 300.0);
        assert_eq!(lyric_target_w(Some(300.0), 260.5), 300.0);
        assert_eq!(lyric_target_w(Some(300.0), 260.0), 260.0);
        assert_eq!(lyric_target_w(Some(300.0), 220.0), 220.0);
    }

    #[test]
    fn lyric_shape_is_a_centred_compact_shape() {
        for style in [Style::Pill, Style::Notch] {
            let base = shape_for(style, ShapeKind::MediaLyric);
            for w in [LYRIC_MIN_W, 333.0, LYRIC_MAX_W] {
                let s = lyric_shape(style, w);
                assert_eq!(s.w, w);
                assert_eq!(s.x * 2.0 + s.w, WINDOW_W);
                assert_eq!((s.y, s.h, s.radius), (base.y, base.h, base.radius));
                assert_eq!(s.h, 34.0);
            }
            // The widest notch, overshooting, still fits the window.
            let peak = (0..=100)
                .map(|i| ease_out_back(i as f32 / 100.0))
                .fold(0.0, f32::max);
            let grown = lerp_shape(
                lyric_shape(style, LYRIC_MIN_W),
                lyric_shape(style, LYRIC_MAX_W),
                peak,
            );
            assert!(grown.x >= SHOULDER && grown.x + grown.w <= WINDOW_W - SHOULDER);
        }
    }

    #[test]
    fn a_line_that_fits_the_widest_box_is_never_trimmed() {
        for style in [Style::Pill, Style::Notch] {
            for robot in [false, true] {
                let room = lyric_box_w(LYRIC_MAX_W, robot) - 1.0;
                for i in 0..=100 {
                    let text_w = room * i as f32 / 100.0 + 0.37;
                    let s = lyric_shape(style, lyric_width(Some(text_w), robot, LYRIC_MAX_W));
                    let area = if robot {
                        lyric_layout_robot(s).1
                    } else {
                        lyric_layout(s)
                    };
                    assert!(area.w >= text_w, "{style:?} {robot} {text_w} in {area:?}");
                    assert!((area.w - lyric_box_w(s.w, robot)).abs() < 1e-3);
                    assert!(!lyric_overflow(text_w, area.w));
                    assert_eq!(lyric_scroll_end_ms(text_w, area.w), None);
                }
            }
        }
    }

    #[test]
    fn a_long_line_holds_then_scrolls_to_its_end() {
        let (text_w, box_w) = (700.0, 500.0);
        assert!(lyric_overflow(text_w, box_w));
        // Held still for the first second.
        for ms in [0, 500, LYRIC_HOLD_MS] {
            assert_eq!(lyric_scroll_offset(text_w, box_w, ms), 0.0);
        }
        // Then 30 px/s.
        let after = lyric_scroll_offset(text_w, box_w, LYRIC_HOLD_MS + 1000);
        assert!((after - LYRIC_SCROLL_PX_PER_S).abs() < 1e-3);
        // Rising, never past the end, then resting there.
        let end = lyric_scroll_end_ms(text_w, box_w).expect("scrolls");
        let max = text_w - box_w + LYRIC_FADE_W;
        let mut last = 0.0;
        for ms in (0..end + 3000).step_by(37) {
            let o = lyric_scroll_offset(text_w, box_w, ms);
            assert!(o >= last && o <= max, "{ms}: {o}");
            last = o;
        }
        assert_eq!(lyric_scroll_offset(text_w, box_w, end), max);
        assert_eq!(lyric_scroll_offset(text_w, box_w, end + 60_000), max);
        assert!(lyric_scroll_offset(text_w, box_w, end - 100) < max);
        // At its end the line's last letter is left of the fade.
        assert!(box_w - (text_w - max) >= LYRIC_FADE_W - 1e-3);
        // A line that fits never moves.
        assert_eq!(lyric_scroll_offset(400.0, box_w, 10_000), 0.0);
        assert_eq!(lyric_scroll_end_ms(400.0, box_w), None);
        assert_eq!(lyric_scroll_end_ms(box_w, box_w), None);
    }
}
