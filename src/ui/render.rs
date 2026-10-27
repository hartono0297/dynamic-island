use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND,
    D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINE_JOIN_ROUND,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION, D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1Brush,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1Geometry, ID2D1Layer, ID2D1PathGeometry,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_MEDIUM,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
    IDWriteTextLayout, IDWriteTypography,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

use std::mem::ManuallyDrop;
use std::time::Instant;

use chrono::NaiveDate;

use super::fonts::{self, Face, Role};
use super::geometry::{
    BAR_GAP, BAR_W, HitRegions, LYRIC_FADE_W, Rect, SHOULDER, ShapeRect, badge_letter, badge_style,
    compact_layout, compact_layout_robot, corner_radius, equalizer_heights, idle_layout,
    lyric_layout, lyric_layout_robot, lyric_overflow, notice_action, notice_action_w,
    notice_layout,
};
use super::icons::{Icon, IconSet};
use super::robot::{Expr, Pose, RobotPainter};
use super::window::IslandWindow;
use crate::calendar::CalendarSnapshot;
use crate::clean::{CleanCat, CleanMode, ScanResult};
use crate::cli::{Output, Shell};
use crate::cli_panel::{Echoes, RunState};
use crate::config::Style;
use crate::i18n;
use crate::lyrics::{self, ShownLine};
use crate::model::{CalView, Hit, MediaInfo, Notice, NoticeKind, Page, Timeline, View};
use crate::mole::{GroupSize, MoleGroup, MoleOpts, MoleReport, PreviewSummary};
use crate::notes::Note;
use crate::pomodoro::{PomoStatus, Pomodoro};
use crate::reminders::Reminder;
use crate::state::MolePhase;
use crate::system::SystemSample;
use crate::theme::{self, Accent, Palette, radius};

// Page drawing lives in its own file; as a child module it shares the
// renderer's private state and helpers.
#[path = "render_panel.rs"]
mod panel;

const SECONDARY_ALPHA: f32 = 0.7;
/// Decoded album art kept at once: one per size drawn (compact and media page).
const ART_SLOTS: usize = 2;
/// Shown for an instrumental gap and before the first lyric line.
const LYRIC_REST: &str = "\u{266A}";
/// How far a lyric line rises while it crossfades into the next.
const LYRIC_RISE: f32 = 5.0;
/// Layout width for measuring one line: wider than any line drawn.
const MEASURE_W: f32 = 4096.0;

/// Everything a frame draws besides the shape: the view and the data behind it.
pub struct Frame<'a> {
    pub view: &'a View,
    pub media: Option<&'a MediaInfo>,
    pub pomodoro: &'a Pomodoro,
    /// Drives the pomodoro countdown and the equalizer animation.
    pub now: Instant,
    /// The local date the island's state is on (today markers, note times).
    pub today: NaiveDate,
    /// The panel page drawn while the view is `Panel`.
    pub page: Page,
    /// The Claude waiting notice shown as a strip at the top of the panel.
    pub strip: Option<&'a Notice>,
    pub cal_view: CalView,
    pub cal_anchor: NaiveDate,
    pub cal_selected: NaiveDate,
    pub calendar: &'a CalendarSnapshot,
    pub notes: &'a [Note],
    pub notes_page: usize,
    /// Upcoming (not done) reminders, earliest first.
    pub reminders: &'a [Reminder],
    pub reminders_page: usize,
    /// Day view: chunk of the day's events ("+N lagi" clicks).
    pub day_page: usize,
    /// -1..1 during a page transition, 0 at rest: the page is drawn shifted
    /// right by `page_slide × panel width`, with the neighbouring page it
    /// came from beside it. Positive = it came in from the right (scrolled to
    /// a later page).
    pub page_slide: f32,
    /// Island colours (surface and accent).
    pub palette: Palette,
    /// "HH:MM" drawn while the view is `Idle`; `None` when the clock is off.
    pub clock: Option<String>,
    /// The line drawn while the view is `MediaLyric`.
    pub lyric: Option<ShownLine<'a>>,
    /// How far a line too long for its box is scrolled left
    /// (`geometry::lyric_scroll_offset`); 0 while it holds or fits.
    pub lyric_scroll: f32,
    /// The fading-out line's scroll, frozen when the line changed.
    pub lyric_prev_scroll: f32,
    /// "Lirik di notch" is on: the Musik page's button is lit.
    pub lyrics_on: bool,
    /// The CLI page.
    pub cli: CliView<'a>,
    /// Where the playing track is: the notch's progress line and the Music
    /// page's bar. `None` without a timeline or a known length.
    pub progress: Option<Progress>,
    /// Bata in the compact island or in place of a notice's badge; `None`
    /// when it is off or the view has none (the panel, other apps' notices).
    pub robot: Option<RobotFrame>,
    /// The System page's readings; `None` draws it without numbers.
    pub system: Option<&'a SystemView<'a>>,
    /// The cleaner, while it shows over the System page.
    pub cleaner: Option<&'a CleanerView<'a>>,
}

/// What the cleaner shows: its mode, the ticks and the armed button (the
/// reducer's), the last scan and what runs (the app's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CleanerView<'a> {
    pub mode: CleanMode,
    /// By `CleanCat::index`.
    pub checked: [bool; 7],
    /// The button (either mode's) is armed: "Yakin? Hapus …".
    pub armed: bool,
    /// The last scan; `None` before the first one ends.
    pub scan: Option<&'a ScanResult>,
    pub scanning: bool,
    /// The own mode's clean runs: rows and button take no clicks.
    pub busy: bool,
    pub mole: MoleView<'a>,
}

/// The Mole mode: where it stands and its options (the reducer's), what
/// was found, read and measured (the app's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoleView<'a> {
    pub phase: MolePhase,
    pub opts: MoleOpts,
    /// The preview's log shows in place of the groups.
    pub show_log: bool,
    /// The Mole found: its version, and whether it is the tested 1.30.x.
    pub version: Option<&'a str>,
    pub tested: bool,
    /// The last dry-run that finished well, read into groups.
    pub summary: Option<&'a PreviewSummary>,
    /// Mole's log, newest last.
    pub log: &'a std::collections::VecDeque<String>,
    /// The last clean's measured result.
    pub report: Option<&'a MoleReport>,
}

/// A Mole group's size column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupShown {
    /// "memindai…": Mole looks or previews.
    Scanning,
    Bytes(u64),
    /// "lihat log": nothing readable (never a guess).
    SeeLog,
}

/// A row's size column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSize {
    /// "—": not measured (no scan yet, or a folder that cannot be read).
    Unknown,
    /// "memindai…".
    Scanning,
    Bytes(u64),
}

impl CleanerView<'_> {
    pub fn size(&self, cat: CleanCat) -> RowSize {
        if self.scanning {
            return RowSize::Scanning;
        }
        match self.scan.map(|s| s.cats[cat.index()]) {
            Some(c) if c.measured => RowSize::Bytes(c.bytes),
            _ => RowSize::Unknown,
        }
    }

    /// The measured size of the ticked rows.
    pub fn picked_bytes(&self) -> u64 {
        CleanCat::ALL
            .into_iter()
            .filter(|c| self.checked[c.index()])
            .filter_map(|c| match self.size(c) {
                RowSize::Bytes(b) => Some(b),
                _ => None,
            })
            .sum()
    }

    /// The button takes a click: something ticked, nothing running.
    pub fn go_live(&self) -> bool {
        !self.scanning
            && !self.busy
            && self.mole.phase != MolePhase::Running
            && self.checked.iter().any(|c| *c)
    }

    /// Mole mode: what shows and what takes a click, by the phase.
    pub fn mole_look(&self) -> crate::ui::panel_layout::MoleLook {
        use crate::ui::panel_layout::{MoleBody, MoleLook};
        let m = &self.mole;
        let running = m.phase == MolePhase::Running;
        let body = match m.phase {
            MolePhase::Missing => MoleBody::Card,
            MolePhase::Running => MoleBody::Log { result: false },
            MolePhase::Finished { .. } => MoleBody::Log { result: true },
            MolePhase::Previewed { .. } if m.show_log => MoleBody::Log { result: false },
            _ => MoleBody::Rows,
        };
        let previewed = matches!(m.phase, MolePhase::Previewed { .. });
        let ready = m.phase
            == MolePhase::Previewed {
                opts: m.opts,
                ok: true,
            };
        let unreadable = MoleGroup::ALL.map(|g| {
            previewed && body == MoleBody::Rows && self.group(g) == GroupShown::SeeLog
        });
        MoleLook {
            body,
            seg_live: !self.busy && !running,
            opts_live: !running && body != MoleBody::Card,
            protect: matches!(
                m.phase,
                MolePhase::Scanning(_) | MolePhase::Previewed { .. } | MolePhase::Finished { .. }
            ),
            go: (ready && !self.busy) || matches!(m.phase, MolePhase::Finished { .. }),
            unreadable,
            log_toggle: previewed && m.show_log,
        }
    }

    /// A Mole group's size: read from the preview, "lihat log" when it
    /// could not be (or the preview failed), "memindai…" before.
    pub fn group(&self, g: MoleGroup) -> GroupShown {
        match self.mole.phase {
            MolePhase::Previewed { ok: false, .. } => GroupShown::SeeLog,
            MolePhase::Previewed { ok: true, .. } => {
                match self.mole.summary.map(|s| s.groups[g.index()]) {
                    Some(GroupSize::Bytes(b)) => GroupShown::Bytes(b),
                    _ => GroupShown::SeeLog,
                }
            }
            _ => GroupShown::Scanning,
        }
    }
}

/// What the System page shows.
pub struct SystemView<'a> {
    /// The last page sample; `None` before the first (dashes, not zeros).
    pub sample: Option<&'a SystemSample>,
    /// The CPU line's points, oldest first (`system::CpuHistory`).
    pub history: &'a [f32],
    /// The warning limits: a drive under `disk_warn_gb` free or RAM over
    /// `ram_warn_percent` draws its meter in ochre.
    pub disk_warn_gb: u32,
    pub ram_warn_percent: u8,
}

/// The robot's face and pose for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotFrame {
    pub expr: Expr,
    pub pose: Pose,
}

/// The playback position within a track of known length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub position_ms: u64,
    pub end_ms: u64,
}

impl Progress {
    /// The position at `now` (frozen while paused, clamped to the end);
    /// `None` without a timeline or when the player reports no length.
    pub fn at(timeline: Option<&Timeline>, playing: bool, now: Instant) -> Option<Self> {
        let t = timeline.filter(|t| t.end_ms > 0)?;
        Some(Self {
            position_ms: lyrics::position_at(t, playing, now),
            end_ms: t.end_ms,
        })
    }

    /// How far through the track, 0..=1; `None` when the length is unknown.
    pub fn fraction(&self) -> Option<f32> {
        (self.end_ms > 0)
            .then(|| (self.position_ms as f64 / self.end_ms as f64).clamp(0.0, 1.0) as f32)
    }
}

/// The compact media's progress line: its full track and the played part.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LineRects {
    track: Rect,
    fill: Rect,
}

/// The notch's progress line is this thick,
const PROGRESS_LINE_H: f32 = 2.0;
/// this far above the shape's bottom edge,
const PROGRESS_LINE_BOTTOM: f32 = 3.0;
/// and its ends this far inside the rounded corners at that height.
const PROGRESS_LINE_MARGIN: f32 = 3.0;

/// The progress line along the compact shape's bottom edge, its ends where
/// the rounded corners leave room at the line's height (so it never leaves
/// the shape); the fill is `fraction` of the track. `None` when the length
/// is unknown or the shape too narrow.
fn compact_progress(style: Style, s: ShapeRect, progress: Progress) -> Option<LineRects> {
    let fraction = progress.fraction()?;
    let r = corner_radius(style, s);
    let bottom = s.y + s.h - PROGRESS_LINE_BOTTOM;
    // The line's lowest point against the bottom corners' circles.
    let dy = (bottom - (s.y + s.h - r)).max(0.0);
    let dx = (r * r - dy * dy).max(0.0).sqrt();
    let inset = r - dx + PROGRESS_LINE_MARGIN;
    let w = s.w - 2.0 * inset;
    if w <= 0.0 {
        return None;
    }
    let track = Rect {
        x: s.x + inset,
        y: bottom - PROGRESS_LINE_H,
        w,
        h: PROGRESS_LINE_H,
    };
    let fill = Rect {
        w: w * fraction,
        ..track
    };
    Some(LineRects { track, fill })
}

/// What the CLI page shows.
pub struct CliView<'a> {
    /// Everything printed so far (echoed commands included).
    pub output: &'a Output,
    /// Which of those lines are echoed commands.
    pub echoes: &'a Echoes,
    /// Lines scrolled up from the newest.
    pub scroll: usize,
    /// The header chip: idle, running since, or how the last run ended.
    pub run: RunState,
    pub shell: Shell,
    /// Where commands run.
    pub cwd: &'a str,
    /// The hidden EDIT's text and selection (UTF-16 offsets, start ≤ end;
    /// the caret is at the end).
    pub input: &'a str,
    pub selection: (usize, usize),
    /// The input has the keyboard: caret, focus ring and the lock banner.
    pub focused: bool,
    /// Earlier commands starting with the input, newest first, and the
    /// highlighted one.
    pub suggestions: Vec<&'a str>,
    pub selected: usize,
    /// The enlarged panel.
    pub tall: bool,
    /// The folder came from the last active Explorer window: the accent
    /// "dari Explorer" chip shows before it.
    pub from_explorer: bool,
    /// The folder menu, while it is open.
    pub menu: Option<&'a FolderMenuView>,
}

/// What the open folder menu lists, top to bottom (`cli_folders::menu_items`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderMenuView {
    pub items: Vec<crate::cli_folders::MenuItem>,
}

/// Draws the island into an offscreen premultiplied DIB and pushes it to the
/// layered window. Pixels outside the shape stay alpha 0, so they are
/// invisible and click-through.
pub struct Renderer {
    d2d: ID2D1Factory,
    wic: IWICImagingFactory,
    /// The shared DirectWrite factory: text layouts are made per draw.
    dwrite: IDWriteFactory,
    /// Tabular figures, applied to every line of text.
    tabular: IDWriteTypography,
    fonts: Fonts,
    /// Studio icon geometries, built on `d2d`.
    icons: IconSet,
    /// Bata's face strokes, built on `d2d`.
    robot: RobotPainter,
    surface: Option<Surface>,
    target: Option<Target>,
    /// Most recently used first; at most `ART_SLOTS`.
    art: Vec<CachedArt>,
    regions: HitRegions,
    scale: f32,
    /// Round caps for the pomodoro progress arcs.
    round_cap: ID2D1StrokeStyle,
    /// Origin of the equalizer animation's clock.
    epoch: Instant,
    /// The palette of the frame being drawn.
    palette: Palette,
    /// Where the last frame drew the CLI input's caret, in logical px; the
    /// IME's composition window goes there.
    cli_caret: std::cell::Cell<Option<Rect>>,
}

struct Fonts {
    /// Notice titles, the compact pomodoro's countdown: DM Sans 12.5 medium.
    title: IDWriteTextFormat,
    /// Body text: DM Sans 12.
    secondary: IDWriteTextFormat,
    /// Event, note and reminder titles: DM Sans 12 medium.
    body_medium: IDWriteTextFormat,
    /// A notice badge's letter.
    badge: IDWriteTextFormat,
    /// The idle clock: DM Sans 600 16.
    clock: IDWriteTextFormat,
    /// The lyric line in the notch: DM Sans 600 16.
    lyric: IDWriteTextFormat,
    /// The rest sign of an instrumental gap, larger than the words.
    lyric_rest: IDWriteTextFormat,
    /// Page titles ("Oktober 2026", "Pomodoro", "Notes"): DM Sans 600 20.
    heading: IDWriteTextFormat,
    /// Captions: DM Sans 10.5.
    small: IDWriteTextFormat,
    small_center: IDWriteTextFormat,
    small_center_bold: IDWriteTextFormat,
    /// Hour labels, group headings, weekday names: DM Sans 9.5.
    micro: IDWriteTextFormat,
    micro_center: IDWriteTextFormat,
    /// The Music page's elapsed and total times: DM Sans 10.
    track_time: IDWriteTextFormat,
    track_time_right: IDWriteTextFormat,
    /// Week view chips: title and time.
    chip_title: IDWriteTextFormat,
    chip_time: IDWriteTextFormat,
    /// Month grid days, and today's in bold: DM Sans 11.5.
    cell: IDWriteTextFormat,
    cell_bold: IDWriteTextFormat,
    /// Week view day numbers: DM Sans 15 medium.
    day_number: IDWriteTextFormat,
    /// Tile labels, times, small buttons: DM Sans 11.
    eleven: IDWriteTextFormat,
    eleven_center: IDWriteTextFormat,
    button: IDWriteTextFormat,
    button_center: IDWriteTextFormat,
    /// The compact media title: DM Sans 12.
    compact: IDWriteTextFormat,
    /// A note card's text under its title: wraps, trimmed at the bottom.
    snippet: IDWriteTextFormat,
    body_center: IDWriteTextFormat,
    body_right: IDWriteTextFormat,
    /// The pomodoro countdown: DM Sans 600 48.
    big_time: IDWriteTextFormat,
    /// The media page's title (DM Sans 600 28) and artist.
    song: IDWriteTextFormat,
    artist: IDWriteTextFormat,
    /// The CLI's output and input: DM Mono 11, else Cascadia Mono or Consolas.
    mono: IDWriteTextFormat,
    /// The CLI's suggestions: the matched part in DM Mono 11 medium.
    mono_medium: IDWriteTextFormat,
    /// The CLI's folder chip: DM Mono 10.5; a folder too long is cut in the
    /// middle, keeping its last part (path ellipsis).
    chip_mono: IDWriteTextFormat,
    /// The CLI's shell chip: DM Mono 10.5.
    mono_small: IDWriteTextFormat,
    /// The CLI's Stop button: DM Sans 10.5 medium.
    small_medium: IDWriteTextFormat,
    /// The System page's CPU figure: DM Sans 600 30, centred.
    sys_big: IDWriteTextFormat,
    /// A meter's name ("RAM", "Disk C:"): DM Sans 600 11.5.
    meter_name: IDWriteTextFormat,
    /// A meter's figures: DM Sans 11, right-aligned.
    eleven_right: IDWriteTextFormat,
    /// A process row's CPU and memory: DM Sans 10.5, right-aligned.
    small_right: IDWriteTextFormat,
    /// A cleaner row's name: DM Sans 11.5.
    row_name: IDWriteTextFormat,
    /// A cleaner row's size: DM Sans 600 11.5, right-aligned.
    row_size: IDWriteTextFormat,
    /// Mole's log: DM Mono 9.5.
    mono_log: IDWriteTextFormat,
}

/// Device-dependent drawing state; rebuilt if Direct2D reports the target lost.
struct Target {
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    /// Clips panel content to the rounded shape.
    layer: ID2D1Layer,
}

/// 32-bpp top-down DIB selected into a memory DC; the render target draws here.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    size: SIZE,
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Album art decoded at the size it is drawn (`pixels` on the short side),
/// keyed by byte length and the first 64 bytes so a new track's thumbnail is
/// noticed without hashing the whole image every frame. Drawing it is then a
/// near 1:1 copy, which matters while the equalizer redraws ~30 times a second.
/// `bitmap` is `None` when decoding failed, so bad bytes are not retried.
struct CachedArt<B = ID2D1Bitmap> {
    key: ArtKey,
    pixels: u32,
    bitmap: Option<B>,
}

type ArtKey = (usize, Vec<u8>);

fn art_key(bytes: &[u8]) -> ArtKey {
    (bytes.len(), bytes[..bytes.len().min(64)].to_vec())
}

/// The cached decode of `key` at `pixels` (`Some(None)`: it failed before),
/// moved to the front as most recently used.
fn art_lookup<B: Clone>(
    cache: &mut Vec<CachedArt<B>>,
    key: &ArtKey,
    pixels: u32,
) -> Option<Option<B>> {
    let i = cache
        .iter()
        .position(|a| &a.key == key && a.pixels == pixels)?;
    let hit = cache.remove(i);
    let bitmap = hit.bitmap.clone();
    cache.insert(0, hit);
    Some(bitmap)
}

/// Adds a decode in front, dropping other tracks' art (never drawn again)
/// and the least recently used size beyond `ART_SLOTS`.
fn art_store<B>(cache: &mut Vec<CachedArt<B>>, key: ArtKey, pixels: u32, bitmap: Option<B>) {
    cache.retain(|a| a.key == key && a.pixels != pixels);
    cache.truncate(ART_SLOTS - 1);
    cache.insert(
        0,
        CachedArt {
            key,
            pixels,
            bitmap,
        },
    );
}

impl Renderer {
    /// Needs COM initialized on the calling thread (for WIC).
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let fonts = Fonts::new(&dwrite)?;
            let icons = IconSet::new(&d2d)?;
            let robot = RobotPainter::new(&d2d)?;
            let tabular = fonts::tabular_figures(&dwrite)?;
            let round_cap = d2d.CreateStrokeStyle(
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
            )?;
            Ok(Self {
                d2d,
                wic,
                dwrite,
                tabular,
                fonts,
                icons,
                robot,
                surface: None,
                target: None,
                art: Vec::new(),
                regions: HitRegions::default(),
                scale: 1.0,
                round_cap,
                epoch: Instant::now(),
                palette: theme::island(Accent::default(), theme::Surface::default()),
                cli_caret: std::cell::Cell::new(None),
            })
        }
    }

    /// Renders one frame. `shape` is in logical pixels; `opacity` 0 clears
    /// every pixel so the window is fully invisible and click-through.
    pub fn draw(
        &mut self,
        win: &IslandWindow,
        style: Style,
        shape: ShapeRect,
        frame: &Frame,
        opacity: f32,
    ) -> Result<()> {
        let opacity = opacity.clamp(0.0, 1.0);
        self.palette = frame.palette;
        self.scale = win.scale();
        self.regions = HitRegions::default();
        self.cli_caret.set(None);
        self.ensure_surface(win.size())?;
        // A lost target is rebuilt once; a second failure is reported.
        if let Err(error) = self.paint(style, shape, frame, opacity) {
            if error.code() != D2DERR_RECREATE_TARGET {
                return Err(error);
            }
            self.target = None;
            self.art.clear();
            self.regions = HitRegions::default();
            self.paint(style, shape, frame, opacity)?;
        }
        self.present(win, opacity)
    }

    /// Pushes the drawn surface to the layered window at `opacity`.
    fn present(&self, win: &IslandWindow, opacity: f32) -> Result<()> {
        let surface = self
            .surface
            .as_ref()
            .expect("surface ensured by the caller");
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (opacity * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                win.hwnd(),
                None,
                Some(&win.origin()),
                Some(&surface.size),
                Some(surface.dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }

    /// `--demo-robot`'s sheet (studio-robot.html page 2): the ten faces at
    /// 48 px with their names, then a blink, the notch sizes (48, 32, 26,
    /// 20 px) and three faces at 26 px, on a dark card filling the window.
    #[cfg(debug_assertions)]
    pub fn draw_robot_sheet(&mut self, win: &IslandWindow, palette: Palette) -> Result<()> {
        self.palette = palette;
        self.scale = win.scale();
        self.regions = HitRegions::default();
        self.ensure_surface(win.size())?;
        self.ensure_target()?;
        let surface = self.surface.as_ref().expect("surface ensured above");
        let target = self.target.as_ref().expect("target ensured above");
        let bounds = RECT {
            left: 0,
            top: 0,
            right: surface.size.cx,
            bottom: surface.size.cy,
        };
        unsafe {
            target.rt.BindDC(surface.dc, &bounds)?;
            target.rt.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            target.rt.BeginDraw();
            target.rt.Clear(Some(&color(0, 0.0)));
        }
        self.paint_robot_sheet();
        let target = self.target.as_ref().expect("target ensured above");
        unsafe { target.rt.EndDraw(None, None) }?;
        self.present(win, 1.0)
    }

    #[cfg(debug_assertions)]
    fn paint_robot_sheet(&self) {
        use super::geometry::ROBOT_COMPACT;
        use super::robot::Expr;
        let card = Rect {
            x: 20.0,
            y: 8.0,
            w: 580.0,
            h: 440.0,
        };
        self.set_color(self.palette.surface, 1.0);
        self.fill_round(card, radius::PANEL);
        let title = Rect {
            x: card.x + 24.0,
            y: card.y + 14.0,
            w: 300.0,
            h: 28.0,
        };
        let lang = i18n::lang();
        let name = i18n::text(lang, i18n::Key::SourceBata);
        self.text(name, &self.fonts.heading, title, 1.0);
        let pose = Pose::default();
        let col = (card.w - 40.0) / 5.0;
        for (i, expr) in Expr::ALL.into_iter().enumerate() {
            let (row, c) = (i / 5, i % 5);
            let x = card.x + 20.0 + c as f32 * col;
            let y = card.y + 56.0 + row as f32 * 118.0;
            let cell = Rect {
                x: x + 4.0,
                y,
                w: col - 8.0,
                h: 108.0,
            };
            self.set_color(self.palette.raised, 1.0);
            self.fill_round(cell, radius::NOTICE / 1.5);
            let size = 56.0;
            let area = Rect {
                x: cell.x + (cell.w - size) / 2.0,
                y: cell.y + 14.0,
                w: size,
                h: size,
            };
            self.paint_robot(area, RobotFrame { expr, pose });
            let label = Rect {
                y: cell.y + 80.0,
                h: 18.0,
                ..cell
            };
            let name = i18n::text(lang, expr.key());
            self.text(name, &self.fonts.small_center, label, 1.0);
        }
        // Sizes: a blink, 48 → 20 px, then three faces at the notch's 26 px.
        let base = card.y + 56.0 + 2.0 * 118.0 + 70.0;
        let face = |expr: Expr| i18n::text(lang, expr.key());
        let blinking = i18n::text(lang, i18n::Key::ExprBlink);
        let items: [(Expr, bool, f32, &str); 8] = [
            (Expr::Idle, true, 48.0, blinking),
            (Expr::Idle, false, 48.0, "48 px"),
            (Expr::Idle, false, 32.0, "32 px"),
            (Expr::Idle, false, ROBOT_COMPACT, "26 px"),
            (Expr::Idle, false, 20.0, "20 px"),
            (Expr::Happy, false, ROBOT_COMPACT, face(Expr::Happy)),
            (Expr::Alert, false, ROBOT_COMPACT, face(Expr::Alert)),
            (Expr::Sleep, false, ROBOT_COMPACT, face(Expr::Sleep)),
        ];
        let step = (card.w - 40.0) / items.len() as f32;
        for (i, (expr, blink, size, label)) in items.into_iter().enumerate() {
            let cx = card.x + 20.0 + step * (i as f32 + 0.5);
            let area = Rect {
                x: cx - size / 2.0,
                y: base - size,
                w: size,
                h: size,
            };
            let pose = Pose {
                blink,
                ..Pose::default()
            };
            self.paint_robot(area, RobotFrame { expr, pose });
            let text = Rect {
                x: cx - step / 2.0,
                y: base + 8.0,
                w: step,
                h: 16.0,
            };
            self.text_colored(
                label,
                &self.fonts.small_center,
                text,
                self.palette.secondary,
                1.0,
            );
        }
    }

    /// Maps a client point in physical pixels to what the last frame drew there.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Hit> {
        self.regions.hit(x / self.scale, y / self.scale)
    }

    /// Whether the wheel at this client point (physical pixels) scrolls the
    /// CLI output rather than the pages.
    pub fn over_cli_output(&self, x: f32, y: f32) -> bool {
        self.regions.over_cli_output(x / self.scale, y / self.scale)
    }

    /// The CLI caret the last frame drew, in client physical pixels.
    pub fn cli_caret(&self) -> Option<RECT> {
        let r = self.cli_caret.get()?;
        let px = |v: f32| (v * self.scale).round() as i32;
        Some(RECT {
            left: px(r.x),
            top: px(r.y),
            right: px(r.right()),
            bottom: px(r.bottom()),
        })
    }

    fn ensure_surface(&mut self, size: SIZE) -> Result<()> {
        if self.surface.as_ref().is_some_and(|s| s.size == size) {
            return Ok(());
        }
        self.surface = None;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.cx,
                // Negative height: top-down rows.
                biHeight: -size.cy,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            self.surface = Some(Surface {
                dc,
                bitmap,
                previous,
                size,
            });
        }
        Ok(())
    }

    fn ensure_target(&mut self) -> Result<()> {
        if self.target.is_some() {
            return Ok(());
        }
        // Software rendering: the canvas is tiny and GDI-bound anyway, and it
        // keeps GPU drivers out of the idle working set.
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            ..Default::default()
        };
        unsafe {
            let rt = self.d2d.CreateDCRenderTarget(&props)?;
            // Layered windows cannot use ClearType: it has no alpha channel.
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&color(self.palette.text, 1.0), None)?;
            let layer = rt.CreateLayer(None)?;
            self.target = Some(Target { rt, brush, layer });
        }
        Ok(())
    }

    fn paint(&mut self, style: Style, shape: ShapeRect, frame: &Frame, opacity: f32) -> Result<()> {
        self.ensure_target()?;
        let surface = self.surface.as_ref().expect("surface ensured by draw");
        let target = self.target.as_ref().expect("target ensured above");
        let rt = &target.rt;
        let bounds = RECT {
            left: 0,
            top: 0,
            right: surface.size.cx,
            bottom: surface.size.cy,
        };
        unsafe {
            rt.BindDC(surface.dc, &bounds)?;
            // Draw in logical pixels; the target scales to the window's DPI.
            rt.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            rt.BeginDraw();
            rt.Clear(Some(&color(0, 0.0)));
        }
        let painted = if opacity > 0.0 {
            self.paint_island(style, shape, frame)
        } else {
            Ok(())
        };
        // EndDraw always runs so a failed frame never leaves the target mid-draw;
        // its error (e.g. a lost target) takes precedence.
        let rt = &self.target.as_ref().expect("target ensured above").rt;
        unsafe { rt.EndDraw(None, None) }.and(painted)
    }

    fn paint_island(&mut self, style: Style, shape: ShapeRect, frame: &Frame) -> Result<()> {
        let view = frame.view;
        let radius = corner_radius(style, shape);
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.brush.SetColor(&color(self.palette.surface, 1.0));
            match style {
                Style::Pill => target.rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: d2d_rect(shape_rect(shape)),
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &target.brush,
                ),
                Style::Notch => {
                    let path = self.notch_path(shape, radius)?;
                    target.rt.FillGeometry(&path, &target.brush, None);
                }
            }
        }
        if *view == View::Hidden {
            return Ok(());
        }
        self.regions.shape = Some((style, shape));
        // Never a robot in the panel.
        if let View::Panel(_) = view {
            return self.paint_panel(style, shape, frame);
        }
        unsafe {
            target.rt.PushAxisAlignedClip(
                &d2d_rect(shape_rect(shape)),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        let robot = frame.robot;
        let content = match view {
            View::MediaCompact => frame.media.map_or(Ok(()), |m| {
                self.paint_compact(style, shape, m, frame.now, frame.progress, robot)
            }),
            View::MediaLyric => {
                let scroll = (frame.lyric_scroll, frame.lyric_prev_scroll);
                self.paint_lyric(shape, frame.lyric.as_ref(), scroll, robot);
                Ok(())
            }
            View::PomodoroCompact => self.paint_pomodoro_compact(shape, frame),
            // A pomodoro notice shows while the next phase waits (Ready).
            View::Notice(notice) => self.paint_notice(shape, notice, frame.pomodoro, robot),
            View::Idle => {
                // Without the clock or the robot, idle is the empty shape.
                let l = idle_layout(shape, robot.is_some(), frame.clock.is_some());
                if let (Some(area), Some(r)) = (l.robot, robot) {
                    self.paint_robot(area, r);
                }
                if let (Some(area), Some(time)) = (l.clock, &frame.clock) {
                    // A sleeping Bata dims the clock beside it, as in the design.
                    let asleep = robot.is_some_and(|r| r.expr == Expr::Sleep);
                    self.text(
                        time,
                        &self.fonts.clock,
                        area,
                        if asleep { 0.6 } else { 1.0 },
                    );
                }
                Ok(())
            }
            // The panel returned above.
            View::Panel(_) | View::Hidden => Ok(()),
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.PopAxisAlignedClip() };
        content
    }

    /// Bata filling the square `area`.
    fn paint_robot(&self, area: Rect, robot: RobotFrame) {
        let target = self.target.as_ref().expect("target ensured by paint");
        self.robot
            .paint(&target.rt, &target.brush, area, robot.expr, robot.pose, 1.0);
    }

    fn paint_compact(
        &mut self,
        style: Style,
        shape: ShapeRect,
        media: &MediaInfo,
        now: Instant,
        progress: Option<Progress>,
        robot: Option<RobotFrame>,
    ) -> Result<()> {
        let layout = match robot {
            Some(_) => compact_layout_robot(shape),
            None => compact_layout(shape),
        };
        if let Some(line) = progress.and_then(|p| compact_progress(style, shape, p)) {
            let radius = PROGRESS_LINE_H / 2.0;
            self.set_color(self.palette.track, 1.0);
            self.fill_round(line.track, radius);
            if line.fill.w > 0.0 {
                self.set_color(self.palette.accent, 1.0);
                self.fill_round(line.fill, radius);
            }
        }
        // The robot takes the album art's place, as in the design.
        match robot {
            Some(r) => self.paint_robot(layout.art, r),
            None => self.paint_art(layout.art, radius::ART, media.art.as_deref())?,
        }
        // The song, as in the design; the artist when the player gives no title.
        let label = if media.title.is_empty() {
            &media.artist
        } else {
            &media.title
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        self.text(label, &self.fonts.compact, layout.text, 1.0);
        // Every bar's frequency × 10 s is a whole number of cycles, so the
        // clock can wrap at 10 s without a jump and stays precise for f32.
        let t = (now.saturating_duration_since(self.epoch).as_secs_f64() % 10.0) as f32;
        let heights = equalizer_heights(t, media.playing);
        unsafe { target.brush.SetColor(&color(self.palette.accent, 1.0)) };
        for (i, h) in heights.into_iter().enumerate() {
            let bar = Rect {
                x: layout.bars.x + i as f32 * (BAR_W + BAR_GAP),
                y: layout.bars.bottom() - h,
                w: BAR_W,
                h,
            };
            self.fill_round(bar, 1.0);
        }
        Ok(())
    }

    /// How wide the lyric line `text` is drawn, in logical px; `None` for an
    /// empty or blank line, which shows the rest sign instead. Measured
    /// through the same layout as the draw (tabular figures, tracking).
    pub fn lyric_text_width(&self, text: &str) -> Option<f32> {
        (!text.trim().is_empty()).then(|| self.text_width(text, &self.fonts.lyric))
    }

    /// The current lyric line: centred when it fits its box (the notch is
    /// sized to it), else from the box's left edge, shifted left by its
    /// `scroll` and faded at the right. While lines change, the old one
    /// fades out rising and the new one fades in from below.
    fn paint_lyric(
        &self,
        shape: ShapeRect,
        lyric: Option<&ShownLine>,
        (scroll, prev_scroll): (f32, f32),
        robot: Option<RobotFrame>,
    ) {
        let area = match robot {
            Some(r) => {
                let (slot, area) = lyric_layout_robot(shape);
                self.paint_robot(slot, r);
                area
            }
            None => lyric_layout(shape),
        };
        let Some(lyric) = lyric else { return };
        let t = lyric.fade.clamp(0.0, 1.0);
        // Smoothstep: the lines settle gently instead of stopping dead.
        let eased = t * t * (3.0 - 2.0 * t);
        if let Some(prev) = lyric.prev.filter(|_| t < 1.0) {
            let up = Rect {
                y: area.y - LYRIC_RISE * eased,
                ..area
            };
            self.lyric_line(prev, up, area, prev_scroll, 1.0 - t);
        }
        let rising = Rect {
            y: area.y + LYRIC_RISE * (1.0 - eased),
            ..area
        };
        self.lyric_line(lyric.text, rising, area, scroll, t);
    }

    /// One lyric line in `area` (`clip`: the box at rest, which the rising
    /// line may leave vertically). Never trimmed: a line wider than its box
    /// is laid out whole, clipped to the box and faded at the right.
    fn lyric_line(&self, text: &str, area: Rect, clip: Rect, scroll: f32, alpha: f32) {
        if text.trim().is_empty() {
            self.text(LYRIC_REST, &self.fonts.lyric_rest, area, alpha * 0.85);
            return;
        }
        if area.w <= 0.0 {
            return;
        }
        let target = self.target.as_ref().expect("target ensured by paint");
        let wide: Vec<u16> = text.encode_utf16().collect();
        // As wide as `text_width` measures, so the draw matches the measure.
        let Ok(layout) = self.layout(&wide, &self.fonts.lyric, MEASURE_W, area.h) else {
            return;
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        if unsafe { layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING) }.is_err()
            || unsafe { layout.GetMetrics(&mut metrics) }.is_err()
        {
            return;
        }
        let text_w = metrics.widthIncludingTrailingWhitespace;
        unsafe { target.brush.SetColor(&color(self.palette.text, alpha)) };
        let options = D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT;
        if !lyric_overflow(text_w, area.w) {
            // Centred; a box held wider than the line (hysteresis) centres it too.
            let x = area.x + (area.w - text_w) / 2.0;
            unsafe {
                target
                    .rt
                    .DrawTextLayout(Vector2::new(x, area.y), &layout, &target.brush, options)
            };
            return;
        }
        // How much of the line is still hidden past the box sets how strong
        // the fade is: full while a fade's width or more is, none once the
        // end has come into the box.
        let hidden = area.x - scroll + text_w - area.right();
        let fade = (hidden / LYRIC_FADE_W).clamp(0.0, 1.0);
        let opacity = (fade > 0.0).then(|| self.fade_brush(clip, fade)).flatten();
        let params = D2D1_LAYER_PARAMETERS {
            // The box across, the shape (clipped outside) up and down.
            contentBounds: D2D_RECT_F {
                left: clip.x,
                top: clip.y - LYRIC_RISE * 2.0,
                right: clip.right(),
                bottom: clip.bottom() + LYRIC_RISE * 2.0,
            },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            opacityBrush: ManuallyDrop::new(opacity),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe {
            target.rt.PushLayer(&params, &target.layer);
            target.rt.DrawTextLayout(
                Vector2::new(area.x - scroll, area.y),
                &layout,
                &target.brush,
                options,
            );
            target.rt.PopLayer();
        }
        // The layer kept its own reference while pushed.
        drop(ManuallyDrop::into_inner(params.opacityBrush));
    }

    /// An opacity brush over `area`: opaque, then over the last
    /// `LYRIC_FADE_W` px down to `1 - strength` at the right edge.
    fn fade_brush(&self, area: Rect, strength: f32) -> Option<ID2D1Brush> {
        let target = self.target.as_ref().expect("target ensured by paint");
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: color(0, 1.0),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: color(0, 1.0 - strength),
            },
        ];
        unsafe {
            let stops = target
                .rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .ok()?;
            let brush = target
                .rt
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2::new(area.right() - LYRIC_FADE_W, area.y),
                        endPoint: Vector2::new(area.right(), area.y),
                    },
                    None,
                    &stops,
                )
                .ok()?;
            brush.cast().ok()
        }
    }

    /// Badge, title over body, and the chip naming what a click does
    /// (`notice_action`): a reminder is then done, a pomodoro phase waiting
    /// to start starts (in that phase's colour).
    fn paint_notice(
        &mut self,
        shape: ShapeRect,
        notice: &Notice,
        pomo: &Pomodoro,
        robot: Option<RobotFrame>,
    ) -> Result<()> {
        let p = self.palette;
        let next = pomo.phase();
        let action = notice_action(&notice.kind, pomo.status()).map(|key| {
            let fill = match notice.kind {
                NoticeKind::Pomodoro => p.phase(next),
                _ => p.accent,
            };
            (key, fill, p.on_accent)
        });
        let layout = notice_layout(shape, action.map(|_| notice_action_w(&notice.kind)));
        let badge = badge_style(notice, &p, next);
        match robot {
            // The island's own notices: Bata (40 px) instead of the badge.
            Some(r) => self.paint_robot(layout.badge, r),
            None => {
                self.set_color(badge.fill, badge.alpha);
                self.fill_round(layout.badge, radius::BADGE);
                match badge.icon {
                    // Line only: the duotone would vanish on a tint of its own colour.
                    Some(icon) => self.icon_colored(icon, layout.badge, 20.0, badge.ink, 1.0, None),
                    None => {
                        let letter = badge_letter(&notice.source);
                        self.text_colored(&letter, &self.fonts.badge, layout.badge, badge.ink, 1.0);
                    }
                }
            }
        }
        self.text(&notice.title, &self.fonts.title, layout.title, 1.0);
        self.text_colored(
            &notice.body,
            &self.fonts.small,
            layout.body,
            p.secondary,
            1.0,
        );
        if let (Some(chip), Some((key, fill, ink))) = (layout.action, action) {
            self.set_color(fill, 1.0);
            self.fill_round(chip, chip.h / 2.0);
            let label = i18n::tr(key);
            self.text_colored(label, &self.fonts.button_center, chip, ink, 1.0);
        }
        Ok(())
    }

    /// Album art cropped to a rounded square; a music-note tile when absent or undecodable.
    fn paint_art(&mut self, area: Rect, radius: f32, bytes: Option<&[u8]>) -> Result<()> {
        let bitmap = match bytes {
            Some(bytes) => self.art_bitmap(bytes, area.w.min(area.h))?,
            None => None,
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        let Some(bitmap) = bitmap else {
            unsafe { target.brush.SetColor(&color(self.palette.placeholder, 1.0)) };
            self.fill_round(area, radius);
            self.icon(Icon::Music, area, 16.0, SECONDARY_ALPHA, true);
            return Ok(());
        };
        unsafe {
            let D2D_SIZE_F { width, height } = bitmap.GetSize();
            // Cover the square: scale the short side to fit, center the long one.
            let k = area.w / width.min(height).max(1.0);
            let transform = Matrix3x2 {
                M11: k,
                M12: 0.0,
                M21: 0.0,
                M22: k,
                M31: area.x + (area.w - width * k) / 2.0,
                M32: area.y + (area.h - height * k) / 2.0,
            };
            let brush = target.rt.CreateBitmapBrush(&bitmap, None, None)?;
            brush.SetTransform(&transform);
            let brush: ID2D1Brush = brush.cast()?;
            target.rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: d2d_rect(area),
                    radiusX: radius,
                    radiusY: radius,
                },
                &brush,
            );
        }
        Ok(())
    }

    /// The cached bitmap for these bytes drawn `side` logical px square,
    /// decoding (and downscaling to that size) only when the art, the size or
    /// the DPI changed.
    fn art_bitmap(&mut self, bytes: &[u8], side: f32) -> Result<Option<ID2D1Bitmap>> {
        let key = art_key(bytes);
        let pixels = (side * self.scale).ceil().max(1.0) as u32;
        if let Some(bitmap) = art_lookup(&mut self.art, &key, pixels) {
            return Ok(bitmap);
        }
        let bitmap = self.decode_art(bytes, pixels).ok();
        art_store(&mut self.art, key, pixels, bitmap.clone());
        Ok(bitmap)
    }

    fn decode_art(&self, bytes: &[u8], pixels: u32) -> Result<ID2D1Bitmap> {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            // The stream borrows `bytes`; everything reading it is done before returning.
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder = self.wic.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = decoder.GetFrame(0)?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h)?;
            let factor = pixels as f32 / w.min(h).max(1) as f32;
            let source: IWICBitmapSource = if factor < 1.0 {
                let scaler = self.wic.CreateBitmapScaler()?;
                let sw = ((w as f32 * factor).round() as u32).max(1);
                let sh = ((h as f32 * factor).round() as u32).max(1);
                scaler.Initialize(&frame, sw, sh, WICBitmapInterpolationModeFant)?;
                scaler.cast()?
            } else {
                frame.cast()?
            };
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(
                &source,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            target.rt.CreateBitmapFromWicBitmap(&converter, None)
        }
    }

    /// Notch outline: square top corners extended by inward-curved shoulders,
    /// rounded bottom corners.
    fn notch_path(&self, s: ShapeRect, r: f32) -> Result<ID2D1PathGeometry> {
        let (left, right, top, bottom) = (s.x, s.x + s.w, s.y, s.y + s.h);
        let arc = |x: f32, y: f32, radius: f32, sweep: D2D1_SWEEP_DIRECTION| D2D1_ARC_SEGMENT {
            point: Vector2::new(x, y),
            size: D2D_SIZE_F {
                width: radius,
                height: radius,
            },
            rotationAngle: 0.0,
            sweepDirection: sweep,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        let (cw, ccw) = (
            D2D1_SWEEP_DIRECTION_CLOCKWISE,
            D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
        );
        unsafe {
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(left - SHOULDER, top), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddArc(&arc(left, top + SHOULDER, SHOULDER, cw));
            sink.AddLine(Vector2::new(left, bottom - r));
            sink.AddArc(&arc(left + r, bottom, r, ccw));
            sink.AddLine(Vector2::new(right - r, bottom));
            sink.AddArc(&arc(right, bottom - r, r, ccw));
            sink.AddLine(Vector2::new(right, top + SHOULDER));
            sink.AddArc(&arc(right + SHOULDER, top, SHOULDER, cw));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            Ok(path)
        }
    }

    /// The shape's outline as a geometry (for clipping panel content).
    fn shape_geometry(&self, style: Style, s: ShapeRect) -> Result<ID2D1Geometry> {
        let radius = corner_radius(style, s);
        match style {
            Style::Pill => unsafe {
                self.d2d
                    .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                        rect: d2d_rect(shape_rect(s)),
                        radiusX: radius,
                        radiusY: radius,
                    })?
                    .cast()
            },
            Style::Notch => self.notch_path(s, radius)?.cast(),
        }
    }

    /// Fills a rounded rect with the shared brush's current color.
    fn fill_round(&self, r: Rect, radius: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: d2d_rect(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                &target.brush,
            );
        }
    }

    /// A Studio icon `size` px square centred in `area`, in the text colour;
    /// `duo` adds the accent's duotone fill.
    fn icon(&self, icon: Icon, area: Rect, size: f32, alpha: f32, duo: bool) {
        let accent = duo.then_some(self.palette.accent);
        self.icon_colored(icon, area, size, self.palette.text, alpha, accent);
    }

    fn icon_colored(
        &self,
        icon: Icon,
        area: Rect,
        size: f32,
        ink: u32,
        alpha: f32,
        accent: Option<u32>,
    ) {
        let target = self.target.as_ref().expect("target ensured by paint");
        // Like text, a failed icon is skipped; a lost target surfaces at EndDraw.
        let _ = self.icons.draw(
            &target.rt,
            &target.brush,
            icon,
            area,
            size,
            ink,
            alpha,
            accent,
        );
    }

    fn text(&self, text: &str, format: &IDWriteTextFormat, area: Rect, alpha: f32) {
        self.text_colored(text, format, area, self.palette.text, alpha);
    }

    /// Text in the palette's secondary colour (captions, times, labels).
    fn text2(&self, text: &str, format: &IDWriteTextFormat, area: Rect) {
        self.text_colored(text, format, area, self.palette.secondary, 1.0);
    }

    /// Sets the shared brush for the fills that follow.
    fn set_color(&self, rgb: u32, alpha: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.brush.SetColor(&color(rgb, alpha)) };
    }

    /// `text` in `format` laid out in a `w` × `h` box: tabular figures on its
    /// digits and, for a display format, the display tracking. Both
    /// `text_width` and `text_colored` go through here, so what is measured
    /// is what is drawn.
    fn layout(
        &self,
        text: &[u16],
        format: &IDWriteTextFormat,
        w: f32,
        h: f32,
    ) -> Result<IDWriteTextLayout> {
        let layout = fonts::text_layout(&self.dwrite, text, format, w, h, &self.tabular)?;
        if self.fonts.is_display(format) {
            let size = unsafe { format.GetFontSize() };
            fonts::apply_tracking(&layout, text.len() as u32, size)?;
        }
        Ok(layout)
    }

    /// `text` fits on one line of a notice of `kind` (with its chip, if
    /// any): its title (`title`) or its body, measured as it is drawn.
    pub fn notice_text_fits(&self, kind: &NoticeKind, title: bool, text: &str) -> bool {
        let s = super::geometry::shape_for(Style::Pill, super::geometry::ShapeKind::Notice);
        let chip = notice_action(kind, PomoStatus::Idle).map(|_| notice_action_w(kind));
        let l = notice_layout(s, chip);
        let (format, area) = if title {
            (&self.fonts.title, l.title)
        } else {
            (&self.fonts.small, l.body)
        };
        self.text_width(text, format) <= area.w
    }

    /// How wide `text` is drawn in `format` on one line, in logical px.
    fn text_width(&self, text: &str, format: &IDWriteTextFormat) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Ok(layout) = self.layout(&wide, format, MEASURE_W, 64.0) else {
            return 0.0;
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        match unsafe { layout.GetMetrics(&mut metrics) } {
            Ok(()) => metrics.widthIncludingTrailingWhitespace,
            Err(_) => 0.0,
        }
    }

    /// One line of text in `area`, trimmed with an ellipsis when too long and
    /// clipped to it, with tabular figures (a layout, since `DrawText` cannot
    /// carry OpenType features).
    fn text_colored(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        area: Rect,
        rgb: u32,
        alpha: f32,
    ) {
        if text.is_empty() || area.w <= 0.0 {
            return;
        }
        let target = self.target.as_ref().expect("target ensured by paint");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Ok(layout) = self.layout(&wide, format, area.w, area.h) else {
            return;
        };
        unsafe {
            target.brush.SetColor(&color(rgb, alpha));
            target.rt.DrawTextLayout(
                Vector2::new(area.x, area.y),
                &layout,
                &target.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }
    }
}

impl Fonts {
    /// `format` is one of the display formats (DM Sans 600), which are
    /// tracked by `fonts::DISPLAY_TRACKING_EM`.
    fn is_display(&self, format: &IDWriteTextFormat) -> bool {
        [
            &self.clock,
            &self.lyric,
            &self.heading,
            &self.big_time,
            &self.song,
            &self.sys_big,
        ]
        .contains(&format)
    }

    fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        // DM Sans (display: 600, tracked) and DM Mono when embedded, else
        // Segoe UI Variable and Cascadia Mono / Consolas.
        let text = Face::for_role(dwrite, Role::Text);
        let display = Face::for_role(dwrite, Role::Display);
        let mono = Face::for_role(dwrite, Role::Mono);
        let line = |face: &Face, weight, size| line_format(dwrite, face, weight, size);
        let aligned = |face: &Face, weight, size, align: DWRITE_TEXT_ALIGNMENT| -> Result<_> {
            let format = line_format(dwrite, face, weight, size)?;
            unsafe { format.SetTextAlignment(align)? };
            Ok(format)
        };
        let centered =
            |face: &Face, weight, size| aligned(face, weight, size, DWRITE_TEXT_ALIGNMENT_CENTER);
        let (normal, medium, semi) = (
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_WEIGHT_MEDIUM,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
        );
        // Studio scale: body 12, captions 10.5, code 11; display only from 16 px.
        let (body, caption) = (12.0, 10.5);
        let snippet = text.format(dwrite, normal, caption)?;
        unsafe {
            snippet.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            snippet.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            snippet.SetLineSpacing(
                windows::Win32::Graphics::DirectWrite::DWRITE_LINE_SPACING_METHOD_UNIFORM,
                15.0,
                12.0,
            )?;
            let ellipsis = dwrite.CreateEllipsisTrimmingSign(&snippet)?;
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            snippet.SetTrimming(&trimming, &ellipsis)?;
        }
        let chip_mono = line(&mono, normal, caption)?;
        unsafe {
            let ellipsis = dwrite.CreateEllipsisTrimmingSign(&chip_mono)?;
            // Trimmed in the middle, keeping what follows the last `\`.
            let path = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: u32::from('\\'),
                delimiterCount: 1,
            };
            chip_mono.SetTrimming(&path, &ellipsis)?;
        }
        Ok(Self {
            title: line(&text, medium, 12.5)?,
            secondary: line(&text, normal, body)?,
            body_medium: line(&text, medium, body)?,
            badge: centered(&text, semi, 16.0)?,
            clock: centered(&display, semi, 16.0)?,
            lyric: centered(&display, semi, 16.0)?,
            lyric_rest: centered(&text, normal, 19.0)?,
            heading: line(&display, semi, 20.0)?,
            small: line(&text, normal, caption)?,
            small_center: centered(&text, normal, caption)?,
            small_center_bold: centered(&text, semi, caption)?,
            micro: line(&text, normal, 9.5)?,
            micro_center: centered(&text, normal, 9.5)?,
            track_time: line(&text, normal, 10.0)?,
            track_time_right: aligned(&text, normal, 10.0, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            chip_title: line(&text, medium, 9.5)?,
            chip_time: line(&text, normal, 9.0)?,
            cell: centered(&text, normal, 11.5)?,
            cell_bold: centered(&text, semi, 11.5)?,
            day_number: centered(&text, medium, 15.0)?,
            eleven: line(&text, normal, 11.0)?,
            eleven_center: centered(&text, normal, 11.0)?,
            button: line(&text, medium, 11.0)?,
            button_center: centered(&text, medium, 11.0)?,
            compact: line(&text, normal, body)?,
            snippet,
            body_center: centered(&text, normal, body)?,
            body_right: aligned(&text, normal, body, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            big_time: centered(&display, semi, 48.0)?,
            song: line(&display, semi, 28.0)?,
            artist: line(&text, normal, 12.5)?,
            mono: line(&mono, normal, 11.0)?,
            mono_medium: line(&mono, medium, 11.0)?,
            chip_mono,
            mono_small: line(&mono, normal, caption)?,
            small_medium: line(&text, medium, caption)?,
            sys_big: centered(&display, semi, 30.0)?,
            meter_name: line(&text, semi, 11.5)?,
            eleven_right: aligned(&text, normal, 11.0, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            small_right: aligned(&text, normal, caption, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            row_name: line(&text, normal, 11.5)?,
            row_size: aligned(&text, semi, 11.5, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            mono_log: line(&mono, normal, 9.5)?,
        })
    }
}

/// Single line, vertically centered, never wrapping, trimmed per character with "…".
fn line_format(
    dwrite: &IDWriteFactory,
    face: &Face,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
) -> Result<IDWriteTextFormat> {
    let format = face.format(dwrite, weight, size)?;
    unsafe {
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let ellipsis = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        format.SetTrimming(&trimming, &ellipsis)?;
        Ok(format)
    }
}

fn shape_rect(s: ShapeRect) -> Rect {
    Rect {
        x: s.x,
        y: s.y,
        w: s.w,
        h: s.h,
    }
}

fn d2d_rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}

/// `a` moved `t` of the way to `b`, channel by channel (0xRRGGBB): the
/// tinted text on tinted fills (event blocks) without new colours.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// 0xRRGGBB plus alpha as a Direct2D color (straight alpha; Direct2D premultiplies).
fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zz_probe_widths() {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let r = Renderer::new().expect("renderer");
        eprintln!("embedded: {}", fonts::embedded(&r.dwrite).loaded.dm_sans);
        for s in [
            "Mole selesai · 2,4 GB dibebaskan",
            "Mole selesai · 12,4 GB dibebaskan",
            "Mole done · 12.4 GB freed",
            "Mole gagal · kode 1",
            "2,3 GB dibebaskan",
        ] {
            eprintln!("title {s:?} {}", r.text_width(s, &r.fonts.title));
        }
        for s in [
            "Disk C: sekarang sisa 49,5 GB · 3 item dilewati (sedang dipakai)",
            "C: sisa 49,5 GB · 3 item dilewati",
            "C: sisa 49,5 GB · 123 item dilewati · Chrome, Edge terbuka",
            "C: 49.5 GB free · 123 items skipped · Windows Update skipped",
            "Diukur dari sisa disk C:",
            "Measured from free space on C:",
        ] {
            eprintln!("body {s:?} {}", r.text_width(s, &r.fonts.small));
        }
        eprintln!("Log {}", r.text_width("Log", &r.fonts.button_center));
    }

    #[test]
    fn mix_blends_each_channel() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(mix(0xE2683C, 0xF4EDE4, 0.5), 0xEBAB90);
        assert_eq!(mix(0x102030, 0x102030, 0.7), 0x102030);
        assert_eq!(mix(0x00FF00, 0xFF0000, 2.0), 0xFF0000, "clamped");
    }

    fn progress(position_ms: u64, end_ms: u64) -> Progress {
        Progress {
            position_ms,
            end_ms,
        }
    }

    #[test]
    fn progress_follows_the_timeline() {
        let t0 = Instant::now();
        let t = Timeline {
            position_ms: 102_000,
            end_ms: 238_000,
            updated: t0,
        };
        let later = t0 + std::time::Duration::from_millis(1_500);
        assert_eq!(
            Progress::at(Some(&t), true, later),
            Some(progress(103_500, 238_000))
        );
        assert_eq!(
            Progress::at(Some(&t), false, later),
            Some(progress(102_000, 238_000)),
            "frozen while paused"
        );
        let unknown = Timeline { end_ms: 0, ..t };
        assert_eq!(Progress::at(Some(&unknown), true, later), None);
        assert_eq!(Progress::at(None, true, later), None);
        assert_eq!(progress(500, 0).fraction(), None);
        assert_eq!(progress(300_000, 238_000).fraction(), Some(1.0), "clamped");
    }

    #[test]
    fn compact_progress_line_stays_inside_the_shape() {
        use super::super::geometry::{ShapeKind, shape_contains, shape_for};
        for style in [Style::Pill, Style::Notch] {
            let s = shape_for(style, ShapeKind::MediaCompact);
            let line = |pos| compact_progress(style, s, progress(pos, 1_000)).unwrap();

            let empty = line(0);
            assert_eq!(empty.fill.w, 0.0);
            let half = line(500);
            assert!((half.fill.w - half.track.w / 2.0).abs() < 0.01);
            let full = line(1_000);
            assert_eq!(full.fill, full.track);
            assert_eq!(compact_progress(style, s, progress(500, 0)), None);

            let t = full.track;
            assert_eq!(t.h, PROGRESS_LINE_H);
            // Along the bottom edge, under the art, centred, most of the width.
            assert!((t.bottom() - (s.y + s.h - PROGRESS_LINE_BOTTOM)).abs() < 0.01);
            assert!(t.y >= compact_layout(s).art.bottom(), "{style:?} {t:?}");
            assert!(((t.x - s.x) - (s.x + s.w - t.right())).abs() < 0.01);
            assert!(t.w >= s.w * 0.8, "{style:?} {t:?}");
            // Clear of the rounded corners at both ends.
            for (x, y) in [
                (t.x, t.y),
                (t.x, t.bottom()),
                (t.right() - 0.01, t.y),
                (t.right() - 0.01, t.bottom()),
            ] {
                assert!(shape_contains(style, s, x, y), "{style:?} ({x}, {y})");
            }
        }
    }

    fn sizes(cache: &[CachedArt<u32>]) -> Vec<(usize, u32)> {
        cache.iter().map(|a| (a.key.0, a.pixels)).collect()
    }

    #[test]
    fn art_is_cached_per_drawn_size() {
        let mut cache: Vec<CachedArt<u32>> = Vec::new();
        let track = art_key(&[1; 100]);
        assert_eq!(art_lookup(&mut cache, &track, 36), None);
        art_store(&mut cache, track.clone(), 36, Some(1));
        art_store(&mut cache, track.clone(), 128, Some(2));
        // Compact and media page both stay decoded: no decode per frame.
        assert_eq!(art_lookup(&mut cache, &track, 36), Some(Some(1)));
        assert_eq!(art_lookup(&mut cache, &track, 128), Some(Some(2)));
        assert_eq!(sizes(&cache), [(100, 128), (100, 36)], "most recent first");
        // A third size (DPI change) evicts the least recently used one.
        art_store(&mut cache, track.clone(), 54, Some(3));
        assert_eq!(sizes(&cache), [(100, 54), (100, 128)]);
        // A failed decode is remembered, so bad bytes are not retried.
        let bad = art_key(&[9; 10]);
        art_store(&mut cache, bad.clone(), 36, None);
        assert_eq!(art_lookup(&mut cache, &bad, 36), Some(None));
        // A new track drops the old one's art.
        assert_eq!(sizes(&cache), [(10, 36)]);
    }
}
