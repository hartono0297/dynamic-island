use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1Brush,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
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

use super::geometry::{
    BAR_COUNT, BAR_GAP, BAR_W, HitRegions, Rect, SHOULDER, ShapeRect, badge_color, badge_letter,
    compact_layout, corner_radius, expanded_layout, notice_layout,
};
use super::window::IslandWindow;
use crate::config::Style;
use crate::model::{Hit, MediaInfo, Notice, View};

const TEXT: u32 = 0xF2F2F2;
const SECONDARY_ALPHA: f32 = 0.7;
const BAR_COLOR: u32 = 0x7FD18B;
const ART_PLACEHOLDER: u32 = 0x2C2C2E;
/// Equalizer heights: a static "playing" silhouette, flat while paused.
const BARS_PLAYING: [f32; BAR_COUNT] = [6.0, 12.0, 9.0, 14.0];
const BARS_PAUSED: [f32; BAR_COUNT] = [3.0; BAR_COUNT];
/// Glyphs shared by Segoe Fluent Icons and Segoe MDL2 Assets.
const GLYPH_PREV: &str = "\u{E892}";
const GLYPH_PLAY: &str = "\u{E768}";
const GLYPH_PAUSE: &str = "\u{E769}";
const GLYPH_NEXT: &str = "\u{E893}";
const GLYPH_MUSIC: &str = "\u{E8D6}";

/// Draws the island into an offscreen premultiplied DIB and pushes it to the
/// layered window. Pixels outside the shape stay alpha 0, so they are
/// invisible and click-through.
pub struct Renderer {
    d2d: ID2D1Factory,
    wic: IWICImagingFactory,
    fonts: Fonts,
    surface: Option<Surface>,
    target: Option<Target>,
    art: Option<CachedArt>,
    regions: HitRegions,
    scale: f32,
}

struct Fonts {
    title: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_large: IDWriteTextFormat,
    badge: IDWriteTextFormat,
}

/// Device-dependent drawing state; rebuilt if Direct2D reports the target lost.
struct Target {
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
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

/// Decoded album art, keyed by byte length and the first 64 bytes so a new
/// track's thumbnail is noticed without hashing the whole image every frame.
/// `bitmap` is `None` when decoding failed, so bad bytes are not retried.
struct CachedArt {
    key: (usize, Vec<u8>),
    pixels: u32,
    bitmap: Option<ID2D1Bitmap>,
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
            Ok(Self {
                d2d,
                wic,
                fonts,
                surface: None,
                target: None,
                art: None,
                regions: HitRegions::default(),
                scale: 1.0,
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
        view: &View,
        media: Option<&MediaInfo>,
        opacity: f32,
    ) -> Result<()> {
        let opacity = opacity.clamp(0.0, 1.0);
        self.scale = win.scale();
        self.regions = HitRegions::default();
        self.ensure_surface(win.size())?;
        // A lost target is rebuilt once; a second failure is reported.
        if let Err(error) = self.paint(style, shape, view, media, opacity) {
            if error.code() != D2DERR_RECREATE_TARGET {
                return Err(error);
            }
            self.target = None;
            self.art = None;
            self.paint(style, shape, view, media, opacity)?;
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
            let brush = rt.CreateSolidColorBrush(&color(TEXT, 1.0), None)?;
            self.target = Some(Target { rt, brush });
        }
        Ok(())
    }

    fn paint(
        &mut self,
        style: Style,
        shape: ShapeRect,
        view: &View,
        media: Option<&MediaInfo>,
        opacity: f32,
    ) -> Result<()> {
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
            self.paint_island(style, shape, view, media)
        } else {
            Ok(())
        };
        // EndDraw always runs so a failed frame never leaves the target mid-draw;
        // its error (e.g. a lost target) takes precedence.
        let rt = &self.target.as_ref().expect("target ensured above").rt;
        unsafe { rt.EndDraw(None, None) }.and(painted)
    }

    fn paint_island(
        &mut self,
        style: Style,
        shape: ShapeRect,
        view: &View,
        media: Option<&MediaInfo>,
    ) -> Result<()> {
        let radius = corner_radius(style, shape);
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.brush.SetColor(&color(0, 1.0));
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
        unsafe {
            target.rt.PushAxisAlignedClip(
                &d2d_rect(shape_rect(shape)),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        let content = match view {
            View::MediaCompact => media.map_or(Ok(()), |m| self.paint_compact(shape, m)),
            // Task 8 draws each panel page; until then every page shows the media layout.
            View::Panel(_) => media.map_or(Ok(()), |m| self.paint_expanded(shape, m)),
            View::Notice(notice) => self.paint_notice(shape, notice),
            // Empty shapes until Task 8 draws the pomodoro ring.
            View::Idle | View::PomodoroCompact | View::Hidden => Ok(()),
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.PopAxisAlignedClip() };
        content
    }

    fn paint_compact(&mut self, shape: ShapeRect, media: &MediaInfo) -> Result<()> {
        let layout = compact_layout(shape);
        self.paint_art(layout.art, 5.0, media.art.as_deref())?;
        let label = if media.artist.is_empty() {
            &media.title
        } else {
            &media.artist
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        self.text(label, &self.fonts.secondary, layout.text, SECONDARY_ALPHA);
        let heights = if media.playing {
            BARS_PLAYING
        } else {
            BARS_PAUSED
        };
        unsafe { target.brush.SetColor(&color(BAR_COLOR, 1.0)) };
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

    fn paint_expanded(&mut self, shape: ShapeRect, media: &MediaInfo) -> Result<()> {
        let layout = expanded_layout(shape);
        self.paint_art(layout.art, 8.0, media.art.as_deref())?;
        self.text(&media.title, &self.fonts.title, layout.title, 1.0);
        self.text(
            &media.artist,
            &self.fonts.secondary,
            layout.artist,
            SECONDARY_ALPHA,
        );
        let play = if media.playing {
            GLYPH_PAUSE
        } else {
            GLYPH_PLAY
        };
        let [prev, middle, next] = layout.buttons;
        self.text(GLYPH_PREV, &self.fonts.icon, prev, 1.0);
        self.text(play, &self.fonts.icon_large, middle, 1.0);
        self.text(GLYPH_NEXT, &self.fonts.icon, next, 1.0);
        self.regions.buttons = Some(layout.buttons);
        Ok(())
    }

    fn paint_notice(&mut self, shape: ShapeRect, notice: &Notice) -> Result<()> {
        let layout = notice_layout(shape);
        let fill = badge_color(notice);
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
        // Dark letter on the light fills (WhatsApp green, amber), white otherwise.
        let ink = match fill {
            0x25D366 => 0x0B3D1F,
            0xF5A524 => 0x3D2A00,
            _ => 0xFFFFFF,
        };
        let letter = badge_letter(&notice.source);
        self.text_colored(&letter, &self.fonts.badge, layout.badge, ink, 1.0);
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
            Some(bytes) => self.art_bitmap(bytes)?,
            None => None,
        };
        let target = self.target.as_ref().expect("target ensured by paint");
        let Some(bitmap) = bitmap else {
            unsafe { target.brush.SetColor(&color(ART_PLACEHOLDER, 1.0)) };
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

    /// The cached bitmap for these bytes, decoding (and downscaling to the
    /// largest size drawn, 40 logical px) only when the art or DPI changed.
    fn art_bitmap(&mut self, bytes: &[u8]) -> Result<Option<ID2D1Bitmap>> {
        let key = (bytes.len(), bytes[..bytes.len().min(64)].to_vec());
        let pixels = (40.0 * self.scale).ceil() as u32;
        if let Some(art) = &self.art
            && art.key == key
            && art.pixels == pixels
        {
            return Ok(art.bitmap.clone());
        }
        let bitmap = self.decode_art(bytes, pixels).ok();
        self.art = Some(CachedArt {
            key,
            pixels,
            bitmap: bitmap.clone(),
        });
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
        self.text_colored(text, format, area, TEXT, alpha);
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
        let icon_family =
            first_installed(dwrite, &[w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")])?;
        let line = |weight, size| line_format(dwrite, text_family, weight, size);
        let centered = |family, weight, size| -> Result<IDWriteTextFormat> {
            let format = line_format(dwrite, family, weight, size)?;
            unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
            Ok(format)
        };
        Ok(Self {
            title: line(DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0)?,
            secondary: line(DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
            icon: centered(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 16.0)?,
            icon_large: centered(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 22.0)?,
            badge: centered(text_family, DWRITE_FONT_WEIGHT_BOLD, 17.0)?,
        })
    }
}

/// The first family in `names` present in the system collection (else the last name,
/// letting DirectWrite's own fallback pick).
fn first_installed(dwrite: &IDWriteFactory, names: &[PCWSTR]) -> Result<PCWSTR> {
    unsafe {
        let mut collection = None;
        dwrite.GetSystemFontCollection(&mut collection, false)?;
        if let Some(collection) = collection {
            for &name in names {
                let (mut index, mut exists) = (0u32, BOOL(0));
                collection.FindFamilyName(name, &mut index, &mut exists)?;
                if exists.as_bool() {
                    return Ok(name);
                }
            }
        }
    }
    Ok(*names.last().expect("at least one family name"))
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
