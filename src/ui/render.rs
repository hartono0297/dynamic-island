use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND,
    D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
    D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_LINE_JOIN_ROUND,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1Geometry, ID2D1Layer,
    ID2D1PathGeometry, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
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
use windows::core::{BOOL, Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use std::time::Instant;

use chrono::NaiveDate;

use super::geometry::{
    BAR_GAP, BAR_W, HitRegions, Rect, SHOULDER, ShapeRect, badge_color, badge_letter,
    compact_layout, corner_radius, equalizer_heights, lyric_layout, notice_layout,
};
use super::window::IslandWindow;
use crate::calendar::CalendarSnapshot;
use crate::cli::Shell;
use crate::config::Style;
use crate::lyrics::ShownLine;
use crate::model::{CalView, Hit, MediaInfo, Notice, NoticeKind, Page, View};
use crate::notes::Note;
use crate::pomodoro::Pomodoro;
use crate::reminders::Reminder;
use crate::theme::{self, Accent, Palette};

// Page drawing lives in its own file; as a child module it shares the
// renderer's private state and helpers.
#[path = "render_panel.rs"]
mod panel;

const SECONDARY_ALPHA: f32 = 0.7;
/// Decoded album art kept at once: one per size drawn (compact and media page).
const ART_SLOTS: usize = 2;
/// Glyphs shared by Segoe Fluent Icons and Segoe MDL2 Assets.
const GLYPH_PREV: &str = "\u{E892}";
const GLYPH_PLAY: &str = "\u{E768}";
const GLYPH_PAUSE: &str = "\u{E769}";
const GLYPH_NEXT: &str = "\u{E893}";
const GLYPH_MUSIC: &str = "\u{E8D6}";
/// "Ringer" in Segoe Fluent Icons and Segoe MDL2 Assets.
const GLYPH_BELL: &str = "\u{EA8F}";
/// Shown for an instrumental gap and before the first lyric line.
const LYRIC_REST: &str = "\u{266A}";
/// How far a lyric line rises while it crossfades into the next.
const LYRIC_RISE: f32 = 5.0;

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
    /// "Lirik di notch" is on: the Musik page's button is lit.
    pub lyrics_on: bool,
    /// The CLI page's card.
    pub cli: CliCard<'a>,
}

/// What the CLI page shows.
pub struct CliCard<'a> {
    /// The last command entered, `None` before any (or after `cls`).
    pub command: Option<&'a str>,
    /// Its last lines, oldest first (at most `cli::CARD_LINES`).
    pub tail: Vec<&'a str>,
    pub running: bool,
    pub shell: Shell,
    /// Where commands run; shown while the card is empty.
    pub cwd: &'a str,
}

/// Draws the island into an offscreen premultiplied DIB and pushes it to the
/// layered window. Pixels outside the shape stay alpha 0, so they are
/// invisible and click-through.
pub struct Renderer {
    d2d: ID2D1Factory,
    wic: IWICImagingFactory,
    fonts: Fonts,
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
}

struct Fonts {
    title: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    badge: IDWriteTextFormat,
    /// The reminder notice's bell.
    badge_icon: IDWriteTextFormat,
    /// Segoe Fluent Icons or Segoe MDL2 Assets is installed.
    has_icons: bool,
    /// The idle clock.
    clock: IDWriteTextFormat,
    /// The lyric line in the notch.
    lyric: IDWriteTextFormat,
    /// The rest sign of an instrumental gap, larger than the words.
    lyric_rest: IDWriteTextFormat,
    /// Page titles ("Oktober 2026", "Pomodoro", "Notes").
    heading: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_center: IDWriteTextFormat,
    small_center_bold: IDWriteTextFormat,
    tiny: IDWriteTextFormat,
    tiny_center: IDWriteTextFormat,
    body_center: IDWriteTextFormat,
    body_right: IDWriteTextFormat,
    big_time: IDWriteTextFormat,
    /// The pomodoro page's phase beside the ring.
    phase: IDWriteTextFormat,
    media_title: IDWriteTextFormat,
    media_artist: IDWriteTextFormat,
    icon_small: IDWriteTextFormat,
    icon_panel: IDWriteTextFormat,
    icon_panel_large: IDWriteTextFormat,
    /// Shortcut tile icons.
    icon_tile: IDWriteTextFormat,
    /// The CLI card: Cascadia Mono, else Consolas.
    mono: IDWriteTextFormat,
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
                fonts,
                surface: None,
                target: None,
                art: Vec::new(),
                regions: HitRegions::default(),
                scale: 1.0,
                round_cap,
                epoch: Instant::now(),
                palette: theme::island(Accent::default(), theme::Surface::default()),
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
        let surface = self.surface.as_ref().expect("surface ensured above");
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

    /// Maps a client point in physical pixels to what the last frame drew there.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Hit> {
        self.regions.hit(x / self.scale, y / self.scale)
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
        if let View::Panel(_) = view {
            return self.paint_panel(style, shape, frame);
        }
        unsafe {
            target.rt.PushAxisAlignedClip(
                &d2d_rect(shape_rect(shape)),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        let content = match view {
            View::MediaCompact => frame
                .media
                .map_or(Ok(()), |m| self.paint_compact(shape, m, frame.now)),
            View::MediaLyric => {
                self.paint_lyric(shape, frame.lyric.as_ref());
                Ok(())
            }
            View::PomodoroCompact => self.paint_pomodoro_compact(shape, frame),
            View::Notice(notice) => self.paint_notice(shape, notice),
            View::Idle => {
                // Without the clock, idle is the empty shape.
                if let Some(time) = &frame.clock {
                    self.text(time, &self.fonts.clock, shape_rect(shape), 1.0);
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

    fn paint_compact(&mut self, shape: ShapeRect, media: &MediaInfo, now: Instant) -> Result<()> {
        let layout = compact_layout(shape);
        self.paint_art(layout.art, 5.0, media.art.as_deref())?;
        let label = if media.artist.is_empty() {
            &media.title
        } else {
            &media.artist
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        self.text(label, &self.fonts.secondary, layout.text, SECONDARY_ALPHA);
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

    /// The current lyric line, centred and trimmed; while lines change, the
    /// old one fades out rising and the new one fades in from below.
    fn paint_lyric(&self, shape: ShapeRect, lyric: Option<&ShownLine>) {
        let Some(lyric) = lyric else { return };
        let area = lyric_layout(shape);
        let t = lyric.fade.clamp(0.0, 1.0);
        // Smoothstep: the lines settle gently instead of stopping dead.
        let eased = t * t * (3.0 - 2.0 * t);
        let line = |text: &str, area: Rect, alpha: f32| {
            if text.trim().is_empty() {
                self.text(LYRIC_REST, &self.fonts.lyric_rest, area, alpha * 0.85);
            } else {
                self.text(text, &self.fonts.lyric, area, alpha);
            }
        };
        if let Some(prev) = lyric.prev.filter(|_| t < 1.0) {
            let up = Rect {
                y: area.y - LYRIC_RISE * eased,
                ..area
            };
            line(prev, up, 1.0 - t);
        }
        let rising = Rect {
            y: area.y + LYRIC_RISE * (1.0 - eased),
            ..area
        };
        line(lyric.text, rising, t);
    }

    fn paint_notice(&mut self, shape: ShapeRect, notice: &Notice) -> Result<()> {
        let layout = notice_layout(shape);
        let fill = badge_color(notice, self.palette.accent);
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.brush.SetColor(&color(fill, 1.0));
            let r = layout.badge.w / 2.0;
            target.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2::new(layout.badge.x + r, layout.badge.y + r),
                    radiusX: r,
                    radiusY: r,
                },
                &target.brush,
            );
        }
        if matches!(notice.kind, NoticeKind::Reminder { .. }) {
            // A bell in the accent's ink; the letter if no icon font is installed.
            let ink = self.palette.on_accent;
            if self.fonts.has_icons {
                let bell = &self.fonts.badge_icon;
                self.text_colored(GLYPH_BELL, bell, layout.badge, ink, 1.0);
            } else {
                let letter = badge_letter(&notice.source);
                self.text_colored(&letter, &self.fonts.badge, layout.badge, ink, 1.0);
            }
        } else {
            // Dark letter on the light fills (WhatsApp green, amber), white otherwise.
            let ink = match fill {
                0x25D366 => 0x0B3D1F,
                0xF5A524 => 0x3D2A00,
                _ => 0xFFFFFF,
            };
            let letter = badge_letter(&notice.source);
            self.text_colored(&letter, &self.fonts.badge, layout.badge, ink, 1.0);
        }
        self.text(&notice.title, &self.fonts.title, layout.title, 1.0);
        self.text(
            &notice.body,
            &self.fonts.secondary,
            layout.body,
            SECONDARY_ALPHA,
        );
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
            self.text(GLYPH_MUSIC, &self.fonts.icon, area, SECONDARY_ALPHA);
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

    fn text(&self, text: &str, format: &IDWriteTextFormat, area: Rect, alpha: f32) {
        self.text_colored(text, format, area, self.palette.text, alpha);
    }

    /// One line of text in `area`, trimmed with an ellipsis when too long.
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
        unsafe {
            target.brush.SetColor(&color(rgb, alpha));
            target.rt.DrawText(
                &wide,
                format,
                &d2d_rect(area),
                &target.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

impl Fonts {
    fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        let text_family = first_installed(
            dwrite,
            &[
                w!("Segoe UI Variable Text"),
                w!("Segoe UI Variable"),
                w!("Segoe UI"),
            ],
        )?;
        let icon_names = [w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")];
        let has_icons = find_installed(dwrite, &icon_names)?.is_some();
        let icon_family = first_installed(dwrite, &icon_names)?;
        let mono_family = first_installed(dwrite, &[w!("Cascadia Mono"), w!("Consolas")])?;
        let line = |weight, size| line_format(dwrite, text_family, weight, size);
        let aligned = |family, weight, size, align: DWRITE_TEXT_ALIGNMENT| -> Result<_> {
            let format = line_format(dwrite, family, weight, size)?;
            unsafe { format.SetTextAlignment(align)? };
            Ok(format)
        };
        let centered =
            |family, weight, size| aligned(family, weight, size, DWRITE_TEXT_ALIGNMENT_CENTER);
        let (normal, semi) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        Ok(Self {
            title: line(semi, 13.0)?,
            secondary: line(normal, 12.0)?,
            icon: centered(icon_family, normal, 16.0)?,
            badge: centered(text_family, DWRITE_FONT_WEIGHT_BOLD, 17.0)?,
            badge_icon: centered(icon_family, normal, 18.0)?,
            has_icons,
            clock: centered(text_family, semi, 13.0)?,
            lyric: centered(text_family, semi, 13.0)?,
            lyric_rest: centered(text_family, normal, 19.0)?,
            heading: line(semi, 15.0)?,
            small: line(normal, 11.0)?,
            small_center: centered(text_family, normal, 11.0)?,
            small_center_bold: centered(text_family, semi, 11.0)?,
            tiny: line(normal, 10.0)?,
            tiny_center: centered(text_family, normal, 10.0)?,
            body_center: centered(text_family, normal, 12.0)?,
            body_right: aligned(text_family, normal, 12.0, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            big_time: centered(text_family, semi, 32.0)?,
            phase: centered(text_family, semi, 15.0)?,
            media_title: line(semi, 17.0)?,
            media_artist: line(normal, 13.0)?,
            icon_small: centered(icon_family, normal, 12.0)?,
            icon_panel: centered(icon_family, normal, 20.0)?,
            icon_panel_large: centered(icon_family, normal, 28.0)?,
            icon_tile: centered(icon_family, normal, 22.0)?,
            mono: line_format(dwrite, mono_family, normal, 12.5)?,
        })
    }
}

/// The first family in `names` present in the system collection (else the last name,
/// letting DirectWrite's own fallback pick).
fn first_installed(dwrite: &IDWriteFactory, names: &[PCWSTR]) -> Result<PCWSTR> {
    Ok(find_installed(dwrite, names)?.unwrap_or(*names.last().expect("at least one family name")))
}

/// The first family in `names` present in the system collection, if any.
fn find_installed(dwrite: &IDWriteFactory, names: &[PCWSTR]) -> Result<Option<PCWSTR>> {
    unsafe {
        let mut collection = None;
        dwrite.GetSystemFontCollection(&mut collection, false)?;
        if let Some(collection) = collection {
            for &name in names {
                let (mut index, mut exists) = (0u32, BOOL(0));
                collection.FindFamilyName(name, &mut index, &mut exists)?;
                if exists.as_bool() {
                    return Ok(Some(name));
                }
            }
        }
    }
    Ok(None)
}

/// Single line, vertically centered, never wrapping, trimmed per character with "…".
fn line_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
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
