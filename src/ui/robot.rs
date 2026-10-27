//! Bata, the notch assistant: a little robot drawn from the logo (terracotta
//! tile = head, espresso island = visor, cream dot = eyes). Shapes are copied
//! from `robot()` in `docs/design/studio-robot.html` (a 64-unit grid) and
//! drawn with Direct2D at any size; colours are the fixed brand colours, not
//! the user's accent.
//!
//! Everything here except `RobotPainter` is pure: which face to show
//! (`choose_expr`), when it blinks, sleeps, bounces or lifts, and which
//! one-shot timers that needs (`plan_timers`). `app.rs` only feeds it events
//! and arms the timers it asks for.

use std::time::{Duration, Instant};

use chrono::{NaiveDate, NaiveTime};
// `named_enum!` below names these unqualified.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE, D2D1_LINE_JOIN_ROUND,
    D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, ID2D1Factory,
    ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::core::Result as WinResult;
use windows_numerics::{Matrix3x2, Vector2};

use super::geometry::Rect;
use crate::i18n::Key;
use crate::model::{NoticeKind, View};

/// Head (the logo's tile).
pub const TERRACOTTA: u32 = 0xE2683C;
/// Visor (the logo's island).
pub const ESPRESSO: u32 = 0x17120F;
/// Eyes and extras (the logo's dot).
pub const CREAM: u32 = 0xF4EDE4;

/// The design grid.
pub const GRID: f32 = 64.0;
/// Eye centres and their line.
const L: f32 = 24.5;
const R: f32 = 39.5;
const Y: f32 = 32.0;

/// The ten faces. Blinking is not one of them: it is a `Pose` input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expr {
    Idle,
    Happy,
    Focus,
    /// "Mendengar": music plays.
    Music,
    Sing,
    Alert,
    Think,
    Sleep,
    /// "Lesu".
    Sad,
    /// "Menyapa".
    Wave,
}

impl Expr {
    // The demo sheet (debug builds) and tests list and name every face.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub const ALL: [Expr; 10] = [
        Expr::Idle,
        Expr::Happy,
        Expr::Focus,
        Expr::Music,
        Expr::Sing,
        Expr::Alert,
        Expr::Think,
        Expr::Sleep,
        Expr::Sad,
        Expr::Wave,
    ];

    /// Its eyes are open (pills or round): a blink shows. Arcs do not blink.
    pub fn blinks(self) -> bool {
        matches!(
            self,
            Expr::Idle | Expr::Focus | Expr::Alert | Expr::Think | Expr::Sad
        )
    }

    /// The name the demo sheet labels it with.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub fn key(self) -> Key {
        match self {
            Expr::Idle => Key::ExprIdle,
            Expr::Happy => Key::ExprHappy,
            Expr::Focus => Key::ExprFocus,
            Expr::Music => Key::ExprMusic,
            Expr::Sing => Key::ExprSing,
            Expr::Alert => Key::ExprAlert,
            Expr::Think => Key::ExprThink,
            Expr::Sleep => Key::ExprSleep,
            Expr::Sad => Key::ExprSad,
            Expr::Wave => Key::ExprWave,
        }
    }
}

/// Moment-to-moment state on top of the face: eyes shut for a blink, and
/// how far the whole robot is raised (hover lift, alert bounce), in px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub blink: bool,
    pub lift_px: f32,
    pub bounce_px: f32,
}

// ---- Shapes (pure data, in grid units) ----

/// A stroked line of the face, drawn in cream with round caps and joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    /// The happy eyes: two arcs bending up.
    Happy,
    FocusBrows,
    SadBrows,
    /// Closed eyes bending down.
    Sleep,
    /// The two small z's.
    Zz,
    /// The music note's stem and beam.
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Control point, then end point (an SVG `q`, made absolute).
    Quad(f32, f32, f32, f32),
}

impl Line {
    const ALL: [Line; 6] = [
        Line::Happy,
        Line::FocusBrows,
        Line::SadBrows,
        Line::Sleep,
        Line::Zz,
        Line::Note,
    ];

    fn width(self) -> f32 {
        match self {
            Line::Happy => 2.6,
            Line::FocusBrows | Line::SadBrows => 1.8,
            Line::Sleep => 2.2,
            Line::Zz => 1.5,
            Line::Note => 1.6,
        }
    }

    fn alpha(self) -> f32 {
        if self == Line::Zz { 0.85 } else { 1.0 }
    }

    /// The design's path data, absolute.
    fn segments(self) -> Vec<Seg> {
        use Seg::{Line as To, Move, Quad};
        match self {
            // M L-4 Y+2 q4-6 8 0, twice.
            Line::Happy => vec![
                Move(L - 4.0, Y + 2.0),
                Quad(L, Y - 4.0, L + 4.0, Y + 2.0),
                Move(R - 4.0, Y + 2.0),
                Quad(R, Y - 4.0, R + 4.0, Y + 2.0),
            ],
            // M L-4 Y-4.5 l7 1.6 M R+4 Y-4.5 l-7 1.6
            Line::FocusBrows => vec![
                Move(L - 4.0, Y - 4.5),
                To(L + 3.0, Y - 2.9),
                Move(R + 4.0, Y - 4.5),
                To(R - 3.0, Y - 2.9),
            ],
            // M L-4 Y-2 l7-2 M R+4 Y-2 l-7-2
            Line::SadBrows => vec![
                Move(L - 4.0, Y - 2.0),
                To(L + 3.0, Y - 4.0),
                Move(R + 4.0, Y - 2.0),
                To(R - 3.0, Y - 4.0),
            ],
            // M L-4 Y q4 3.5 8 0, twice.
            Line::Sleep => vec![
                Move(L - 4.0, Y),
                Quad(L, Y + 3.5, L + 4.0, Y),
                Move(R - 4.0, Y),
                Quad(R, Y + 3.5, R + 4.0, Y),
            ],
            // M52 4h5l-5 5.5h5 M59-3h3.6l-3.6 4h3.6
            Line::Zz => vec![
                Move(52.0, 4.0),
                To(57.0, 4.0),
                To(52.0, 9.5),
                To(57.0, 9.5),
                Move(59.0, -3.0),
                To(62.6, -3.0),
                To(59.0, 1.0),
                To(62.6, 1.0),
            ],
            // note(57, 9): M57 9 v-8 l5-1.5 v8
            Line::Note => vec![
                Move(57.0, 9.0),
                To(57.0, 1.0),
                To(62.0, -0.5),
                To(62.0, 7.5),
            ],
        }
    }
}

/// One mark of a face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    /// A filled rounded rect (the eyes are pills).
    Pill {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
    },
    /// A filled ellipse (round eyes, the note's heads, thinking dots, the mouth).
    Dot {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
    },
    Stroke(Line),
}

/// A face: its marks, the vertical bob and the tilt (degrees, clockwise).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub parts: Vec<Part>,
    pub bob: f32,
    pub tilt: f32,
}

/// The design's `eye(x, y, w, h, r)`: a pill centred on (x, y).
fn eye(x: f32, y: f32, w: f32, h: f32, r: f32) -> Part {
    Part::Pill {
        x: x - w / 2.0,
        y: y - h / 2.0,
        w,
        h,
        r,
    }
}

fn dot(cx: f32, cy: f32, r: f32) -> Part {
    Part::Dot {
        cx,
        cy,
        rx: r,
        ry: r,
    }
}

/// The note's two heads (the stem and beam are `Line::Note`).
fn note_heads() -> [Part; 2] {
    [dot(57.0 - 1.6, 9.0, 1.9), dot(57.0 + 3.4, 9.0 - 1.5, 1.9)]
}

/// The marks of `expr`; with `blink`, open eyes (pills and round ones) become
/// the design's `blink` eyes: 7 × 2.4 slits at (L, Y + 1) and (R, Y + 1),
/// wherever the open eyes sat. Arcs (happy, asleep) stay.
pub fn face(expr: Expr, blink: bool) -> Face {
    let happy = Part::Stroke(Line::Happy);
    let (mut parts, bob, tilt) = match expr {
        Expr::Idle => (
            vec![eye(L, Y, 6.0, 9.0, 3.0), eye(R, Y, 6.0, 9.0, 3.0)],
            0.0,
            0.0,
        ),
        Expr::Happy => (vec![happy], 0.0, 0.0),
        Expr::Focus => (
            vec![
                eye(L, Y + 1.0, 7.0, 3.4, 1.7),
                eye(R, Y + 1.0, 7.0, 3.4, 1.7),
                Part::Stroke(Line::FocusBrows),
            ],
            0.0,
            0.0,
        ),
        Expr::Music => {
            let [a, b] = note_heads();
            (vec![happy, Part::Stroke(Line::Note), a, b], 0.0, 0.0)
        }
        Expr::Sing => {
            let [a, b] = note_heads();
            let mouth = Part::Dot {
                cx: 32.0,
                cy: Y + 6.5,
                rx: 2.6,
                ry: 2.0,
            };
            (vec![happy, mouth, Part::Stroke(Line::Note), a, b], 0.0, 0.0)
        }
        Expr::Alert => (vec![dot(L, Y, 4.6), dot(R, Y, 4.6)], -1.5, 0.0),
        Expr::Think => (
            vec![
                eye(L + 2.5, Y - 2.5, 5.5, 7.5, 2.75),
                eye(R + 2.5, Y - 2.5, 5.5, 7.5, 2.75),
                dot(54.0, 6.0, 1.4),
                dot(58.5, 3.0, 1.8),
                dot(63.5, 0.0, 2.2),
            ],
            0.0,
            0.0,
        ),
        Expr::Sleep => (
            vec![Part::Stroke(Line::Sleep), Part::Stroke(Line::Zz)],
            1.5,
            0.0,
        ),
        Expr::Sad => (
            vec![
                eye(L, Y + 2.5, 6.0, 5.0, 2.5),
                eye(R, Y + 2.5, 6.0, 5.0, 2.5),
                Part::Stroke(Line::SadBrows),
            ],
            1.5,
            0.0,
        ),
        Expr::Wave => (vec![happy], 0.0, -9.0),
    };
    if blink && expr.blinks() {
        // Every blinking face starts with its two eyes; brows, dots and
        // other extras stay. The design's `case 'blink'` uses fixed spots.
        parts[0] = eye(L, Y + 1.0, 7.0, 2.4, 1.2);
        parts[1] = eye(R, Y + 1.0, 7.0, 2.4, 1.2);
    }
    Face { parts, bob, tilt }
}

/// Head: the tile, x 6 y 9, 52 × 46, radius 18.
pub const HEAD: (f32, f32, f32, f32, f32) = (6.0, 9.0, 52.0, 46.0, 18.0);
/// Visor: the island, x 12 y 21, 40 × 22, radius 11.
pub const VISOR: (f32, f32, f32, f32, f32) = (12.0, 21.0, 40.0, 22.0, 11.0);

/// The head's rounded rect for a robot drawn in `area` (a square), in px:
/// what must stay inside the island. Extras may draw outside it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn head_rect(area: Rect) -> (Rect, f32) {
    let k = area.w / GRID;
    let (x, y, w, h, r) = HEAD;
    (
        Rect {
            x: area.x + x * k,
            y: area.y + y * k,
            w: w * k,
            h: h * k,
        },
        r * k,
    )
}

// ---- Drawing ----

/// The face strokes as Direct2D paths in grid units, built once per factory.
/// The render target's transform scales them (and their widths) to any size.
pub struct RobotPainter {
    lines: Vec<(Line, ID2D1PathGeometry)>,
    round: ID2D1StrokeStyle,
}

impl RobotPainter {
    pub fn new(factory: &ID2D1Factory) -> WinResult<Self> {
        let mut lines = Vec::new();
        for line in Line::ALL {
            lines.push((line, path(factory, &line.segments())?));
        }
        let round = unsafe {
            factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?
        };
        Ok(Self { lines, round })
    }

    /// Draws Bata filling the square `area` (its 64-unit grid scaled to
    /// `area.w`), raised by the pose's lift and bounce, at `alpha`. The
    /// render target's transform is restored; the brush colour is not.
    pub fn paint(
        &self,
        rt: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        area: Rect,
        expr: Expr,
        pose: Pose,
        alpha: f32,
    ) {
        if area.w <= 0.0 {
            return;
        }
        let face = face(expr, pose.blink);
        let k = area.w / GRID;
        let centre = Vector2::new(GRID / 2.0, GRID / 2.0);
        let local = Matrix3x2::rotation_around(face.tilt, centre)
            * Matrix3x2::translation(0.0, face.bob)
            * Matrix3x2::scale(k, k)
            * Matrix3x2::translation(area.x, area.y - pose.lift_px - pose.bounce_px);
        let color = |rgb: u32, a: f32| {
            let ch = |s: u32| ((rgb >> s) & 0xFF) as f32 / 255.0;
            windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                r: ch(16),
                g: ch(8),
                b: ch(0),
                a: a * alpha,
            }
        };
        let rounded = |(x, y, w, h, r): (f32, f32, f32, f32, f32)| D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            },
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            let mut previous = Matrix3x2::default();
            rt.GetTransform(&mut previous);
            rt.SetTransform(&(local * previous));
            brush.SetColor(&color(TERRACOTTA, 1.0));
            rt.FillRoundedRectangle(&rounded(HEAD), brush);
            brush.SetColor(&color(ESPRESSO, 1.0));
            rt.FillRoundedRectangle(&rounded(VISOR), brush);
            for part in &face.parts {
                match *part {
                    Part::Pill { x, y, w, h, r } => {
                        brush.SetColor(&color(CREAM, 1.0));
                        rt.FillRoundedRectangle(&rounded((x, y, w, h, r)), brush);
                    }
                    Part::Dot { cx, cy, rx, ry } => {
                        brush.SetColor(&color(CREAM, 1.0));
                        rt.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: Vector2::new(cx, cy),
                                radiusX: rx,
                                radiusY: ry,
                            },
                            brush,
                        );
                    }
                    Part::Stroke(line) => {
                        if let Some((_, g)) = self.lines.iter().find(|(l, _)| *l == line) {
                            brush.SetColor(&color(CREAM, line.alpha()));
                            rt.DrawGeometry(g, brush, line.width(), &self.round);
                        }
                    }
                }
            }
            rt.SetTransform(&previous);
        }
    }
}

/// Open figures from `segs` (each `Move` starts one).
fn path(factory: &ID2D1Factory, segs: &[Seg]) -> WinResult<ID2D1PathGeometry> {
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        let mut open = false;
        for seg in segs {
            match *seg {
                Seg::Move(x, y) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(Vector2::new(x, y), D2D1_FIGURE_BEGIN_HOLLOW);
                    open = true;
                }
                Seg::Line(x, y) => sink.AddLine(Vector2::new(x, y)),
                Seg::Quad(cx, cy, x, y) => {
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                        point1: Vector2::new(cx, cy),
                        point2: Vector2::new(x, y),
                    })
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        Ok(path)
    }
}

// ---- Which face ----

/// What a notice is, as far as the robot cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeSort {
    /// Another app's notification (or the island's own messages, which use
    /// the same kind): keeps its badge, no robot.
    App,
    Reminder,
    PomodoroDone,
    ClaudeWaiting,
    ClaudeDone,
    BatteryLow,
    Greeting,
}

impl NoticeSort {
    pub fn of(kind: &NoticeKind) -> Self {
        match kind {
            NoticeKind::App { .. } => NoticeSort::App,
            NoticeKind::Reminder { .. } => NoticeSort::Reminder,
            NoticeKind::Pomodoro => NoticeSort::PomodoroDone,
            NoticeKind::ClaudeWaiting { .. } => NoticeSort::ClaudeWaiting,
            NoticeKind::ClaudeDone { .. } => NoticeSort::ClaudeDone,
            NoticeKind::BatteryLow => NoticeSort::BatteryLow,
            NoticeKind::Greeting => NoticeSort::Greeting,
        }
    }
}

/// Where the island is, as far as the robot cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// Fullscreen: nothing shows.
    Hidden,
    /// The open panel: never a robot.
    Panel,
    Idle,
    Pomodoro,
    Media,
    Lyric,
    Notice(NoticeSort),
}

impl Spot {
    pub fn of(view: &View) -> Self {
        match view {
            View::Hidden => Spot::Hidden,
            View::Panel(_) => Spot::Panel,
            View::Idle => Spot::Idle,
            View::PomodoroCompact => Spot::Pomodoro,
            View::MediaCompact => Spot::Media,
            View::MediaLyric => Spot::Lyric,
            View::Notice(n) => Spot::Notice(NoticeSort::of(&n.kind)),
        }
    }

    /// A compact island: the pointer over it turns the face Happy and lifts
    /// it. Notices keep their face, the panel and hidden show no robot.
    pub fn compact(self) -> bool {
        matches!(
            self,
            Spot::Idle | Spot::Pomodoro | Spot::Media | Spot::Lyric
        )
    }
}

/// The CLI's last word, for the idle face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cli {
    #[default]
    None,
    Running,
    /// Ended with exit 0, a few seconds ago.
    Done,
    /// Ended otherwise, a few seconds ago.
    Failed,
}

/// Everything the face depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RobotInputs {
    pub spot: Spot,
    /// The pointer is over the island (before the panel opens).
    pub hover: bool,
    /// No activity for the sleep time.
    pub asleep: bool,
    /// A Claude Code session is busy.
    pub claude_working: bool,
    pub cli: Cli,
    /// Below the battery threshold and discharging.
    pub battery_low: bool,
}

/// The face for `i`, or `None` where no robot shows. Priority, highest first:
///
/// 1. Hidden and the open panel: no robot.
/// 2. A notice decides by its kind: reminder and Claude waiting → Alert,
///    pomodoro phase done and Claude done → Happy, low battery → Sad,
///    greeting → Wave. Other apps' notifications (`App`) → no robot: they
///    keep their app badge.
/// 3. The pointer over a compact island → Happy (it also wakes it).
/// 4. Pomodoro → Focus, media → Music, lyric line → Sing.
/// 5. Idle: CLI just done → Happy, just failed → Sad; Claude working or a
///    CLI command running → Think; low battery → Sad; asleep → Sleep;
///    otherwise Idle.
pub fn choose_expr(i: &RobotInputs) -> Option<Expr> {
    let compact = match i.spot {
        Spot::Hidden | Spot::Panel => return None,
        Spot::Notice(sort) => {
            return match sort {
                NoticeSort::App => None,
                NoticeSort::Reminder | NoticeSort::ClaudeWaiting => Some(Expr::Alert),
                NoticeSort::PomodoroDone | NoticeSort::ClaudeDone => Some(Expr::Happy),
                NoticeSort::BatteryLow => Some(Expr::Sad),
                NoticeSort::Greeting => Some(Expr::Wave),
            };
        }
        compact => compact,
    };
    if i.hover {
        return Some(Expr::Happy);
    }
    Some(match compact {
        Spot::Pomodoro => Expr::Focus,
        Spot::Media => Expr::Music,
        Spot::Lyric => Expr::Sing,
        _ => match i.cli {
            Cli::Done => Expr::Happy,
            Cli::Failed => Expr::Sad,
            _ if i.claude_working || i.cli == Cli::Running => Expr::Think,
            _ if i.battery_low => Expr::Sad,
            _ if i.asleep => Expr::Sleep,
            _ => Expr::Idle,
        },
    })
}

// ---- Motion policy ----

crate::theme::named_enum! {
    /// "Animasi": Penuh (blink and move), Hemat (no blink or motion; faces
    /// and sleep still change), Mati (static: no robot timers at all).
    Motion { Full, Saver, Off }
}

impl Motion {
    /// What runs with Windows' Energy Saver on: Full drops to Saver.
    pub fn effective(self, energy_saver: bool) -> Motion {
        match self {
            Motion::Full if energy_saver => Motion::Saver,
            m => m,
        }
    }

    /// Eyes blink (Full only).
    pub fn blinks(self) -> bool {
        self == Motion::Full
    }

    /// Hover lift and alert bounce (Full only).
    pub fn moves(self) -> bool {
        self == Motion::Full
    }

    /// Any robot timer at all (sleep, transient faces): not when Off.
    pub fn timed(self) -> bool {
        self != Motion::Off
    }
}

// ---- Animation math ----

pub const BLINK_MS: u32 = 120;
pub const BLINK_MIN_MS: u32 = 4_000;
pub const BLINK_MAX_MS: u32 = 7_000;
/// Hover: the robot rises this far over `LIFT_MS`.
pub const LIFT_PX: f32 = 2.0;
pub const LIFT_MS: u64 = 140;
/// Alert: two bounces this high, each `BOUNCE_MS` long, then still.
pub const BOUNCE_PX: f32 = 3.0;
pub const BOUNCE_MS: u64 = 260;
pub const BOUNCES: u64 = 2;
/// A Claude session counts as working this long after its last activity.
pub const CLAUDE_WORKING_FOR: Duration = Duration::from_secs(120);
/// A finished CLI command shows Happy or Sad this long.
pub const CLI_MOOD_FOR: Duration = Duration::from_secs(4);
/// The sleep setting's range, in minutes.
pub const SLEEP_MIN: u32 = 1;
pub const SLEEP_MAX: u32 = 60;

/// A small xorshift generator: enough for blink delays, seedable in tests.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
}

/// The wait before the next blink: 4 to 7 s.
pub fn blink_delay_ms(rng: &mut Rng) -> u32 {
    BLINK_MIN_MS + rng.next_u32() % (BLINK_MAX_MS - BLINK_MIN_MS + 1)
}

/// The sleep setting kept in its range (hand-edited files).
pub fn clamp_sleep(minutes: u32) -> u32 {
    minutes.clamp(SLEEP_MIN, SLEEP_MAX)
}

/// The sleep stepper one step up (`dir > 0`) or down: by 1 up to 5 minutes,
/// by 5 above.
pub fn sleep_step(minutes: u32, dir: i32) -> u32 {
    let m = clamp_sleep(minutes);
    let next = if dir > 0 {
        if m < 5 { m + 1 } else { (m / 5 + 1) * 5 }
    } else if m <= 5 {
        m.saturating_sub(1)
    } else {
        (m - 1) / 5 * 5
    };
    clamp_sleep(next)
}

/// Nothing happened for `after`.
pub fn asleep(last_activity: Instant, now: Instant, after: Duration) -> bool {
    now.saturating_duration_since(last_activity) >= after
}

/// How long until `asleep` turns true; `None` once it is.
pub fn time_to_sleep(last_activity: Instant, now: Instant, after: Duration) -> Option<Duration> {
    let idle = now.saturating_duration_since(last_activity);
    after.checked_sub(idle).filter(|d| !d.is_zero())
}

/// How high the alert bounce lifts the robot `elapsed` after it began: two
/// half-sine hops, then 0.
pub fn bounce_offset(elapsed: Duration) -> f32 {
    let ms = elapsed.as_millis() as u64;
    if ms >= BOUNCE_MS * BOUNCES {
        return 0.0;
    }
    let t = (ms % BOUNCE_MS) as f32 / BOUNCE_MS as f32;
    BOUNCE_PX * (std::f32::consts::PI * t).sin()
}

/// The hover lift `elapsed` after the pointer arrived: eases up to `LIFT_PX`.
pub fn lift_offset(elapsed: Duration) -> f32 {
    let t = (elapsed.as_millis() as f32 / LIFT_MS as f32).clamp(0.0, 1.0);
    LIFT_PX * (1.0 - (1.0 - t).powi(3))
}

// ---- The robot's state ----

/// What the robot remembers between events. Pure: the caller passes the time.
#[derive(Clone, Debug)]
pub struct Brain {
    last_activity: Instant,
    /// Busy Claude sessions and when each was last heard from.
    claude: Vec<(String, Instant)>,
    cli_running: bool,
    /// A finished command's face and until when it shows.
    cli_mood: Option<(Cli, Instant)>,
    pub battery_low: bool,
    /// The pointer arrived over the island at this time.
    hover: Option<Instant>,
    /// An alert bounce began.
    bounce: Option<Instant>,
    /// The face last drawn.
    shown: Option<Expr>,
}

impl Brain {
    pub fn new(now: Instant) -> Self {
        Self {
            last_activity: now,
            claude: Vec::new(),
            cli_running: false,
            cli_mood: None,
            battery_low: false,
            hover: None,
            bounce: None,
            shown: None,
        }
    }

    /// Something happened: the sleep clock starts over.
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = now;
    }

    pub fn claude_activity(&mut self, session: &str, now: Instant) {
        self.claude.retain(|(s, _)| s != session);
        self.claude.push((session.to_owned(), now));
    }

    /// The session finished or waits for the user: not working.
    pub fn claude_ended(&mut self, session: &str) {
        self.claude.retain(|(s, _)| s != session);
    }

    /// A session showed activity within `CLAUDE_WORKING_FOR`.
    pub fn claude_working(&self, now: Instant) -> bool {
        self.claude
            .iter()
            .any(|(_, at)| now.saturating_duration_since(*at) < CLAUDE_WORKING_FOR)
    }

    pub fn cli_started(&mut self) {
        self.cli_running = true;
        self.cli_mood = None;
    }

    pub fn cli_finished(&mut self, ok: bool, now: Instant) {
        self.cli_running = false;
        let mood = if ok { Cli::Done } else { Cli::Failed };
        self.cli_mood = Some((mood, now + CLI_MOOD_FOR));
    }

    pub fn cli(&self, now: Instant) -> Cli {
        match self.cli_mood {
            Some((mood, until)) if now < until => mood,
            _ if self.cli_running => Cli::Running,
            _ => Cli::None,
        }
    }

    /// The pointer came over the island (`true`) or left.
    pub fn hover(&mut self, on: bool, now: Instant) {
        if on {
            self.hover.get_or_insert(now);
            self.last_activity = now;
        } else {
            self.hover = None;
        }
    }

    /// The inputs `choose_expr` needs at `now`. With motion `Off` nothing
    /// is timed: no sleep and no transient CLI faces.
    pub fn inputs(
        &self,
        spot: Spot,
        now: Instant,
        sleep_after: Duration,
        motion: Motion,
    ) -> RobotInputs {
        let timed = motion.timed();
        let cli = match self.cli(now) {
            Cli::Done | Cli::Failed if !timed => {
                if self.cli_running {
                    Cli::Running
                } else {
                    Cli::None
                }
            }
            c => c,
        };
        RobotInputs {
            spot,
            hover: self.hover.is_some(),
            asleep: timed && asleep(self.last_activity, now, sleep_after),
            claude_working: self.claude_working(now),
            cli,
            battery_low: self.battery_low,
        }
    }

    /// Records the face drawn; an alert that just appeared starts a bounce
    /// when motion is allowed. True when a bounce started.
    pub fn observe(&mut self, expr: Option<Expr>, now: Instant, moves: bool) -> bool {
        let was = std::mem::replace(&mut self.shown, expr);
        let started = moves && expr == Some(Expr::Alert) && was != Some(Expr::Alert);
        if started {
            self.bounce = Some(now);
        }
        if expr != Some(Expr::Alert) {
            self.bounce = None;
        }
        started
    }

    /// When the hover lift began, if it applies at `spot`: only a compact
    /// island lifts (its face turns Happy); a notice keeps its face still.
    fn lift_since(&self, spot: Spot) -> Option<Instant> {
        self.hover.filter(|_| spot.compact())
    }

    /// Lift and bounce at `spot` and `now` (none without motion), and the
    /// blink.
    pub fn pose(&self, spot: Spot, now: Instant, moves: bool, blink: bool) -> Pose {
        if !moves {
            return Pose {
                blink: false,
                ..Pose::default()
            };
        }
        Pose {
            blink,
            lift_px: self
                .lift_since(spot)
                .map_or(0.0, |at| lift_offset(now.saturating_duration_since(at))),
            bounce_px: self
                .bounce
                .map_or(0.0, |at| bounce_offset(now.saturating_duration_since(at))),
        }
    }

    /// A lift or bounce at `spot` is still moving: the frame clock must run.
    pub fn moving(&self, spot: Spot, now: Instant, moves: bool) -> bool {
        let within = |at: Option<Instant>, ms: u64| {
            at.is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(ms))
        };
        moves
            && (within(self.lift_since(spot), LIFT_MS) || within(self.bounce, BOUNCE_MS * BOUNCES))
    }

    /// The next moment the face may change by itself (falling asleep, a
    /// Claude session going quiet, a CLI face ending), for a one-shot timer.
    pub fn next_change(&self, now: Instant, sleep_after: Duration) -> Option<Instant> {
        let sleep = time_to_sleep(self.last_activity, now, sleep_after).map(|d| now + d);
        let claude = self
            .claude
            .iter()
            .map(|(_, at)| *at + CLAUDE_WORKING_FOR)
            .filter(|t| *t > now);
        let cli = self.cli_mood.map(|(_, until)| until).filter(|t| *t > now);
        sleep.into_iter().chain(claude).chain(cli).min()
    }
}

/// Which robot timers run. `face`: the robot on screen (compact view or
/// notice), if any. `motion`: the effective policy. `animating`: the frame
/// clock runs anyway (morphs, lifts), so no blink starts meanwhile. Only
/// open eyes blink: a sleeping, happy or singing robot runs no blink timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerPlan {
    /// The one-shot blink timer (4–7 s, then 120 ms).
    pub blink: bool,
    /// The one-shot for `Brain::next_change`.
    pub change: bool,
}

pub fn plan_timers(face: Option<Expr>, motion: Motion, animating: bool) -> TimerPlan {
    TimerPlan {
        blink: face.is_some_and(Expr::blinks) && motion.blinks() && !animating,
        change: face.is_some() && motion.timed(),
    }
}

// ---- Morning greeting ----

/// The greeting's switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GreetSettings {
    pub robot: bool,
    pub greeting: bool,
}

/// Mornings end at noon: "Selamat pagi" after that would be wrong.
pub const GREET_UNTIL: NaiveTime = match NaiveTime::from_hms_opt(12, 0, 0) {
    Some(t) => t,
    None => NaiveTime::MIN,
};

/// Once per day, in the morning, with the robot and the greeting on.
pub fn should_greet(
    last: Option<NaiveDate>,
    today: NaiveDate,
    now_time: NaiveTime,
    settings: GreetSettings,
) -> bool {
    settings.robot && settings.greeting && last != Some(today) && now_time < GREET_UNTIL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Notice;

    fn inputs(spot: Spot) -> RobotInputs {
        RobotInputs {
            spot,
            hover: false,
            asleep: false,
            claude_working: false,
            cli: Cli::None,
            battery_low: false,
        }
    }

    #[test]
    fn faces_follow_the_design() {
        assert_eq!(
            face(Expr::Idle, false).parts,
            vec![
                Part::Pill {
                    x: 21.5,
                    y: 27.5,
                    w: 6.0,
                    h: 9.0,
                    r: 3.0
                },
                Part::Pill {
                    x: 36.5,
                    y: 27.5,
                    w: 6.0,
                    h: 9.0,
                    r: 3.0
                },
            ]
        );
        // Blink: 7 × 2.4 slits one unit lower.
        let blink = face(Expr::Idle, true);
        assert_eq!(
            blink.parts[0],
            Part::Pill {
                x: 21.0,
                y: 31.8,
                w: 7.0,
                h: 2.4,
                r: 1.2
            }
        );
        assert_eq!(face(Expr::Alert, false).bob, -1.5);
        assert_eq!(face(Expr::Sleep, false).bob, 1.5);
        assert_eq!(face(Expr::Sad, false).bob, 1.5);
        assert_eq!(face(Expr::Wave, false).tilt, -9.0);
        // Music and singing carry the note; singing adds the mouth.
        let music = face(Expr::Music, false).parts;
        assert!(music.contains(&Part::Stroke(Line::Note)));
        assert!(music.contains(&Part::Dot {
            cx: 55.4,
            cy: 9.0,
            rx: 1.9,
            ry: 1.9
        }));
        let sing = face(Expr::Sing, false).parts;
        assert!(sing.contains(&Part::Dot {
            cx: 32.0,
            cy: 38.5,
            rx: 2.6,
            ry: 2.0
        }));
        // Every open-eyed face blinks to the same two slits at Y + 1, as the
        // design's `case 'blink'` draws them.
        for expr in [Expr::Idle, Expr::Focus, Expr::Alert, Expr::Think, Expr::Sad] {
            assert_eq!(
                face(expr, true).parts[..2],
                [
                    eye(L, Y + 1.0, 7.0, 2.4, 1.2),
                    eye(R, Y + 1.0, 7.0, 2.4, 1.2)
                ],
                "{expr:?}"
            );
        }
        // Arcs do not blink; thinking dots never close.
        assert_eq!(face(Expr::Happy, true), face(Expr::Happy, false));
        assert_eq!(
            face(Expr::Think, true).parts[2..],
            face(Expr::Think, false).parts[2..]
        );
        // Happy arcs: the design's q curves made absolute.
        assert_eq!(
            Line::Happy.segments()[..2],
            [Seg::Move(20.5, 34.0), Seg::Quad(24.5, 28.0, 28.5, 34.0)]
        );
        assert_eq!((Line::Zz.alpha(), Line::Happy.width()), (0.85, 2.6));
    }

    #[test]
    fn head_scales_with_the_size() {
        let (head, r) = head_rect(Rect {
            x: 10.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
        });
        assert_eq!(
            (head.x, head.y, head.w, head.h, r),
            (13.0, 4.5, 26.0, 23.0, 9.0)
        );
    }

    #[test]
    fn expression_priority() {
        let base = inputs(Spot::Idle);
        let e = |i: RobotInputs| choose_expr(&i);
        // No robot in the panel, hidden, or on other apps' notifications.
        for spot in [Spot::Panel, Spot::Hidden, Spot::Notice(NoticeSort::App)] {
            assert_eq!(
                e(RobotInputs {
                    hover: true,
                    ..inputs(spot)
                }),
                None
            );
        }
        // Notices by kind, whatever else is going on.
        let busy = RobotInputs {
            hover: true,
            asleep: true,
            claude_working: true,
            cli: Cli::Failed,
            battery_low: true,
            ..base
        };
        for (sort, expr) in [
            (NoticeSort::Reminder, Expr::Alert),
            (NoticeSort::ClaudeWaiting, Expr::Alert),
            (NoticeSort::PomodoroDone, Expr::Happy),
            (NoticeSort::ClaudeDone, Expr::Happy),
            (NoticeSort::BatteryLow, Expr::Sad),
            (NoticeSort::Greeting, Expr::Wave),
        ] {
            let spot = Spot::Notice(sort);
            assert_eq!(e(RobotInputs { spot, ..busy }), Some(expr), "{sort:?}");
            assert_eq!(e(inputs(spot)), Some(expr), "{sort:?}");
        }
        // Hover beats every compact face.
        for spot in [Spot::Idle, Spot::Pomodoro, Spot::Media, Spot::Lyric] {
            assert_eq!(e(RobotInputs { spot, ..busy }), Some(Expr::Happy));
        }
        let busy = RobotInputs {
            hover: false,
            ..busy
        };
        assert_eq!(
            e(RobotInputs {
                spot: Spot::Pomodoro,
                ..busy
            }),
            Some(Expr::Focus)
        );
        assert_eq!(
            e(RobotInputs {
                spot: Spot::Media,
                ..busy
            }),
            Some(Expr::Music)
        );
        assert_eq!(
            e(RobotInputs {
                spot: Spot::Lyric,
                ..busy
            }),
            Some(Expr::Sing)
        );
        // Idle: CLI result > working > battery > sleep > idle.
        assert_eq!(e(busy), Some(Expr::Sad));
        assert_eq!(
            e(RobotInputs {
                cli: Cli::Done,
                ..busy
            }),
            Some(Expr::Happy)
        );
        let working = RobotInputs {
            cli: Cli::None,
            ..busy
        };
        assert_eq!(e(working), Some(Expr::Think));
        let running = RobotInputs {
            cli: Cli::Running,
            claude_working: false,
            ..busy
        };
        assert_eq!(e(running), Some(Expr::Think));
        let low = RobotInputs {
            cli: Cli::None,
            claude_working: false,
            ..busy
        };
        assert_eq!(e(low), Some(Expr::Sad));
        let sleepy = RobotInputs {
            battery_low: false,
            ..low
        };
        assert_eq!(e(sleepy), Some(Expr::Sleep));
        assert_eq!(e(base), Some(Expr::Idle));
    }

    #[test]
    fn notice_sorts_and_spots() {
        let n = |kind| Notice {
            key: "k".into(),
            source: "s".into(),
            title: "t".into(),
            body: "b".into(),
            kind,
        };
        assert_eq!(
            Spot::of(&View::Notice(n(NoticeKind::App { app_id: None }))),
            Spot::Notice(NoticeSort::App)
        );
        assert_eq!(
            Spot::of(&View::Notice(n(NoticeKind::Greeting))),
            Spot::Notice(NoticeSort::Greeting)
        );
        assert_eq!(Spot::of(&View::MediaLyric), Spot::Lyric);
        assert_eq!(Spot::of(&View::Panel(crate::model::Page::Cli)), Spot::Panel);
    }

    #[test]
    fn blink_delay_stays_in_range() {
        let mut rng = Rng::new(42);
        let delays: Vec<u32> = (0..2000).map(|_| blink_delay_ms(&mut rng)).collect();
        assert!(delays.iter().all(|d| (4_000..=7_000).contains(d)));
        // Spread over the range, not stuck.
        assert!(delays.iter().any(|d| *d < 4_500) && delays.iter().any(|d| *d > 6_500));
        // Seeded: the same seed gives the same delays.
        let mut again = Rng::new(42);
        assert_eq!(blink_delay_ms(&mut again), delays[0]);
        assert_eq!(BLINK_MS, 120);
    }

    #[test]
    fn sleep_schedule() {
        let t0 = Instant::now();
        let ten = Duration::from_secs(600);
        assert!(!asleep(t0, t0 + Duration::from_secs(599), ten));
        assert!(asleep(t0, t0 + ten, ten));
        assert_eq!(time_to_sleep(t0, t0, ten), Some(ten));
        assert_eq!(
            time_to_sleep(t0, t0 + Duration::from_secs(60), ten),
            Some(Duration::from_secs(540))
        );
        assert_eq!(time_to_sleep(t0, t0 + ten, ten), None);
        assert_eq!(time_to_sleep(t0, t0 + 2 * ten, ten), None);
        // The brain: activity restarts the clock; hover wakes it.
        let mut b = Brain::new(t0);
        let later = t0 + ten;
        assert!(b.inputs(Spot::Idle, later, ten, Motion::Full).asleep);
        assert!(
            b.inputs(Spot::Idle, later, ten, Motion::Saver).asleep,
            "Hemat still sleeps"
        );
        assert!(
            !b.inputs(Spot::Idle, later, ten, Motion::Off).asleep,
            "Mati never sleeps"
        );
        assert_eq!(b.next_change(t0, ten), Some(t0 + ten));
        b.hover(true, later);
        assert!(!b.inputs(Spot::Idle, later, ten, Motion::Full).asleep);
        b.hover(false, later);
        b.activity(later);
        assert_eq!(b.next_change(later, ten), Some(later + ten));
    }

    #[test]
    fn sleep_setting_steps_and_clamps() {
        assert_eq!(clamp_sleep(0), 1);
        assert_eq!(clamp_sleep(999), 60);
        assert_eq!(sleep_step(10, 1), 15);
        assert_eq!(sleep_step(10, -1), 5);
        assert_eq!(sleep_step(5, -1), 4);
        assert_eq!(sleep_step(1, -1), 1);
        assert_eq!(sleep_step(4, 1), 5);
        assert_eq!(sleep_step(60, 1), 60);
        assert_eq!(sleep_step(12, 1), 15, "off-grid snaps");
        assert_eq!(sleep_step(12, -1), 10);
    }

    #[test]
    fn claude_works_until_done_or_quiet() {
        let t0 = Instant::now();
        let mut b = Brain::new(t0);
        assert!(!b.claude_working(t0));
        b.claude_activity("a", t0);
        assert!(b.claude_working(t0 + Duration::from_secs(119)));
        assert!(!b.claude_working(t0 + CLAUDE_WORKING_FOR), "gone quiet");
        let far = Duration::from_secs(3600);
        assert_eq!(b.next_change(t0, far), Some(t0 + CLAUDE_WORKING_FOR));
        b.claude_ended("a");
        assert!(!b.claude_working(t0));
        assert_eq!(b.next_change(t0, far), Some(t0 + far));
    }

    #[test]
    fn cli_faces_last_a_few_seconds() {
        let t0 = Instant::now();
        let mut b = Brain::new(t0);
        b.cli_started();
        assert_eq!(b.cli(t0), Cli::Running);
        b.cli_finished(false, t0);
        assert_eq!(b.cli(t0 + Duration::from_secs(3)), Cli::Failed);
        assert_eq!(b.cli(t0 + CLI_MOOD_FOR), Cli::None);
        let far = Duration::from_secs(3600);
        assert_eq!(b.next_change(t0, far), Some(t0 + CLI_MOOD_FOR));
        b.cli_finished(true, t0);
        assert_eq!(b.cli(t0), Cli::Done);
        // Mati: no timer could end it, so it never shows.
        assert_eq!(b.inputs(Spot::Idle, t0, far, Motion::Off).cli, Cli::None);
    }

    #[test]
    fn alert_bounces_twice_then_rests() {
        let ms = Duration::from_millis;
        assert_eq!(bounce_offset(ms(0)), 0.0);
        assert!((bounce_offset(ms(BOUNCE_MS / 2)) - BOUNCE_PX).abs() < 1e-3);
        assert!((bounce_offset(ms(BOUNCE_MS + BOUNCE_MS / 2)) - BOUNCE_PX).abs() < 1e-3);
        assert_eq!(bounce_offset(ms(2 * BOUNCE_MS)), 0.0);
        assert_eq!(bounce_offset(ms(10_000)), 0.0);
        // The brain starts it only when an alert appears, and with motion.
        let t0 = Instant::now();
        let mut b = Brain::new(t0);
        assert!(b.observe(Some(Expr::Alert), t0, true));
        assert!(!b.observe(Some(Expr::Alert), t0, true), "already showing");
        let alert = Spot::Notice(NoticeSort::Reminder);
        assert!(b.moving(alert, t0 + ms(100), true));
        assert!(!b.moving(alert, t0 + ms(2 * BOUNCE_MS), true));
        b.observe(Some(Expr::Idle), t0, true);
        assert!(!b.observe(Some(Expr::Alert), t0, false), "Hemat: no bounce");
        assert!(!b.moving(alert, t0, true));
    }

    #[test]
    fn hover_lifts_two_pixels() {
        let t0 = Instant::now();
        let idle = Spot::Idle;
        let mut b = Brain::new(t0);
        b.hover(true, t0);
        assert_eq!(b.pose(idle, t0, true, false).lift_px, 0.0);
        let up = b.pose(idle, t0 + Duration::from_millis(LIFT_MS), true, false);
        assert_eq!(up.lift_px, LIFT_PX);
        assert!(
            b.moving(idle, t0, true) && !b.moving(idle, t0 + Duration::from_millis(LIFT_MS), true)
        );
        // No motion: no lift, no blink.
        assert_eq!(
            b.pose(idle, t0 + Duration::from_secs(1), false, true),
            Pose::default()
        );
        // A second move while hovering keeps the first arrival.
        b.hover(true, t0 + Duration::from_secs(1));
        assert_eq!(
            b.pose(idle, t0 + Duration::from_secs(1), true, true)
                .lift_px,
            LIFT_PX
        );
    }

    #[test]
    fn hover_lifts_only_a_compact_island() {
        let t0 = Instant::now();
        let mut b = Brain::new(t0);
        b.hover(true, t0);
        let later = t0 + Duration::from_millis(LIFT_MS);
        for spot in [Spot::Idle, Spot::Pomodoro, Spot::Media, Spot::Lyric] {
            assert_eq!(
                b.pose(spot, later, true, false).lift_px,
                LIFT_PX,
                "{spot:?}"
            );
            assert!(b.moving(spot, t0, true), "{spot:?}");
        }
        // A notice keeps its own face, still; the panel draws no robot, so
        // re-entering the island there starts no frame clock.
        for spot in [
            Spot::Notice(NoticeSort::Reminder),
            Spot::Notice(NoticeSort::Greeting),
            Spot::Notice(NoticeSort::BatteryLow),
            Spot::Panel,
            Spot::Hidden,
        ] {
            assert_eq!(b.pose(spot, later, true, false).lift_px, 0.0, "{spot:?}");
            assert!(!b.moving(spot, t0, true), "{spot:?}");
        }
    }

    #[test]
    fn motion_policy_and_energy_saver() {
        assert_eq!(Motion::Full.effective(true), Motion::Saver);
        assert_eq!(Motion::Full.effective(false), Motion::Full);
        assert_eq!(Motion::Off.effective(true), Motion::Off);
        assert!(Motion::Full.blinks() && Motion::Full.moves() && Motion::Full.timed());
        assert!(!Motion::Saver.blinks() && !Motion::Saver.moves() && Motion::Saver.timed());
        assert!(!Motion::Off.blinks() && !Motion::Off.timed());
        assert_eq!(Motion::from_name("hemat"), None);
        assert_eq!(Motion::from_name("saver"), Some(Motion::Saver));
    }

    #[test]
    fn timers_run_only_while_a_robot_shows() {
        let plan = |shown: bool, motion, animating| {
            plan_timers(shown.then_some(Expr::Idle), motion, animating)
        };
        // Faces without open eyes never blink, so no blink timer for them.
        for expr in Expr::ALL {
            let blinks = expr.blinks();
            assert_eq!(plan_timers(Some(expr), Motion::Full, false).blink, blinks);
            assert_eq!(face(expr, true) != face(expr, false), blinks, "{expr:?}");
        }
        assert_eq!(
            plan(true, Motion::Full, false),
            TimerPlan {
                blink: true,
                change: true
            }
        );
        // The frame clock draws anyway; the blink waits.
        assert!(!plan(true, Motion::Full, true).blink);
        assert_eq!(
            plan(true, Motion::Saver, false),
            TimerPlan {
                blink: false,
                change: true
            }
        );
        let none = TimerPlan {
            blink: false,
            change: false,
        };
        assert_eq!(plan(true, Motion::Off, false), none);
        for m in [Motion::Full, Motion::Saver, Motion::Off] {
            assert_eq!(plan(false, m, false), none, "{m:?}");
        }
    }

    #[test]
    fn greeting_once_per_morning() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let on = GreetSettings {
            robot: true,
            greeting: true,
        };
        assert!(should_greet(None, day(27), at(7, 30), on));
        assert!(should_greet(Some(day(26)), day(27), at(11, 59), on));
        assert!(
            !should_greet(Some(day(27)), day(27), at(8, 0), on),
            "already today"
        );
        assert!(
            !should_greet(Some(day(26)), day(27), at(12, 0), on),
            "afternoon"
        );
        assert!(!should_greet(
            None,
            day(27),
            at(8, 0),
            GreetSettings {
                greeting: false,
                ..on
            }
        ));
        assert!(!should_greet(
            None,
            day(27),
            at(8, 0),
            GreetSettings { robot: false, ..on }
        ));
    }
}
