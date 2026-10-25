//! Drawing for the settings window: Direct2D on an `ID2D1HwndRenderTarget`
//! (software, so no GPU driver joins the process), DirectWrite text, and
//! every control the layout places. `settings.rs` owns the window and state
//! and hands each frame over as a `Scene`.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT,
    D2D1_SWEEP_DIRECTION, D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
    ID2D1LinearGradientBrush, ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{BOOL, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use super::geometry::{BAR_GAP, BAR_W, SHOULDER, ShapeKind, compact_layout, shape_for};
use super::settings_layout::{
    Align, Button, ButtonKind, CARD_RADIUS, Control, Field, Font, Group, Hit, LINK_GLYPH_W, Layout,
    Measure, Rect, Section, Segment, Text, Tone, accent_key,
};
use crate::config::Style;
use crate::i18n::{Key, Lang, text};
use crate::theme::{Accent, Palette, WindowPalette};

const GLYPH_MINUS: &str = "\u{E738}";
const GLYPH_PLUS: &str = "\u{E710}";
const GLYPH_OPEN: &str = "\u{E8A7}";
const GLYPH_INFO: &str = "\u{E946}";
const GLYPH_MUSIC: &str = "\u{E8D6}";
/// The logo's "camera" dot: the brand blue, whatever the accent.
const LOGO_DOT: u32 = 0x4F8CFF;
/// The equalizer of the preview, frozen mid-beat.
const PREVIEW_BARS: [f32; 4] = [8.0, 13.0, 6.0, 11.0];

/// One frame's worth of what to draw.
pub struct Scene<'a> {
    pub layout: &'a Layout,
    pub p: WindowPalette,
    pub island: Palette,
    pub style: Style,
    pub lang: Lang,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    /// The sidebar selection, in section indexes (fractional while it slides).
    pub nav_pos: f32,
    pub section: Section,
    /// Section crossfade: content opacity and its upward offset.
    pub content_alpha: f32,
    pub content_dy: f32,
    /// Segmented selections in segment indexes, while they slide.
    pub segments: Vec<(Group, f32)>,
    /// The autostart toggle, 0 (off) to 1 (on).
    pub toggle: f32,
    pub focused: Option<Field>,
}

/// Device-independent resources plus the render target.
pub struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    formats: HashMap<(Font, Align), IDWriteTextFormat>,
    /// Segoe Fluent Icons at 16 (sidebar), 12 (steppers, links) and 14 px.
    glyph16: IDWriteTextFormat,
    glyph12: IDWriteTextFormat,
    glyph14: IDWriteTextFormat,
    /// The preview island's line of text.
    island_text: IDWriteTextFormat,
    widths: RefCell<HashMap<(String, Font), f32>>,
    /// The logo's tile outline in its 256-unit design space.
    tile: ID2D1PathGeometry,
    target: Option<Target>,
}

struct Target {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    tile_fill: ID2D1LinearGradientBrush,
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text_family = first_installed(
                &dwrite,
                &[
                    w!("Segoe UI Variable Text"),
                    w!("Segoe UI Variable"),
                    w!("Segoe UI"),
                ],
            )?;
            let display_family = first_installed(
                &dwrite,
                &[
                    w!("Segoe UI Variable Display"),
                    w!("Segoe UI Variable"),
                    w!("Segoe UI"),
                ],
            )?;
            let icon_family = first_installed(
                &dwrite,
                &[w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")],
            )?;
            let mut formats = HashMap::new();
            for font in Font::ALL {
                let family = if font.size() >= 20.0 {
                    display_family
                } else {
                    text_family
                };
                let weight = if font.semibold() {
                    DWRITE_FONT_WEIGHT_SEMI_BOLD
                } else {
                    DWRITE_FONT_WEIGHT_NORMAL
                };
                for (align, a) in [
                    (Align::Left, DWRITE_TEXT_ALIGNMENT_LEADING),
                    (Align::Center, DWRITE_TEXT_ALIGNMENT_CENTER),
                ] {
                    let format = line_format(&dwrite, family, weight, font.size(), a)?;
                    formats.insert((font, align), format);
                }
            }
            let center = DWRITE_TEXT_ALIGNMENT_CENTER;
            let normal = DWRITE_FONT_WEIGHT_NORMAL;
            Ok(Gfx {
                glyph16: line_format(&dwrite, icon_family, normal, 16.0, center)?,
                glyph12: line_format(&dwrite, icon_family, normal, 12.0, center)?,
                glyph14: line_format(&dwrite, icon_family, normal, 14.0, center)?,
                island_text: line_format(
                    &dwrite,
                    text_family,
                    normal,
                    12.0,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                )?,
                tile: tile_geometry(&d2d)?,
                d2d,
                dwrite,
                formats,
                widths: RefCell::new(HashMap::new()),
                target: None,
            })
        }
    }

    /// The window was resized (physical pixels).
    pub fn resize(&self, width: u32, height: u32) {
        if let Some(t) = &self.target {
            unsafe {
                let _ = t.rt.Resize(&D2D_SIZE_U { width, height });
            }
        }
    }

    /// Forgets the target, e.g. after a DPI change; the next frame makes a new one.
    pub fn reset_target(&mut self) {
        self.target = None;
    }

    fn ensure_target(&mut self, hwnd: HWND, size: (u32, u32), dpi: f32) -> Result<&Target> {
        if self.target.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U {
                    width: size.0,
                    height: size.1,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            unsafe {
                let rt = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                let brush = rt.CreateSolidColorBrush(&color(0, 1.0), None)?;
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: color(0x2A2A2E, 1.0),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: color(0x0E0E10, 1.0),
                    },
                ];
                let stops = rt.CreateGradientStopCollection(
                    &stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;
                let tile_fill = rt.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2::new(0.0, 12.0),
                        endPoint: Vector2::new(0.0, 244.0),
                    },
                    None,
                    &stops,
                )?;
                self.target = Some(Target {
                    rt,
                    brush,
                    tile_fill,
                });
            }
        }
        Ok(self.target.as_ref().expect("target just made"))
    }

    /// Draws one frame. A lost target is dropped and remade on the next one.
    pub fn paint(&mut self, hwnd: HWND, size: (u32, u32), dpi: f32, scene: &Scene) -> Result<()> {
        self.ensure_target(hwnd, size, dpi)?;
        let target = self.target.as_ref().expect("ensured");
        let rt: &ID2D1RenderTarget = &target.rt;
        unsafe {
            rt.SetDpi(dpi, dpi);
            rt.BeginDraw();
            rt.SetTransform(&Matrix3x2::identity());
            rt.Clear(Some(&color(scene.p.bg, 1.0)));
        }
        let painter = Painter {
            rt,
            brush: &target.brush,
            gfx: self,
            tile_fill: &target.tile_fill,
            p: scene.p,
            alpha: 1.0,
        };
        painter.sidebar(scene);
        let content = Painter {
            alpha: scene.content_alpha,
            ..painter
        };
        unsafe {
            rt.SetTransform(&Matrix3x2::translation(0.0, scene.content_dy));
        }
        content.content(scene);
        let result = unsafe { rt.EndDraw(None, None) };
        if result.is_err() {
            self.target = None;
        }
        result
    }
}

impl Measure for Gfx {
    fn width(&self, s: &str, font: Font) -> f32 {
        let key = (s.to_owned(), font);
        if let Some(w) = self.widths.borrow().get(&key) {
            return *w;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let w = self
            .formats
            .get(&(font, Align::Left))
            .and_then(|format| unsafe {
                let layout = self
                    .dwrite
                    .CreateTextLayout(&wide, format, 4096.0, 64.0)
                    .ok()?;
                let mut m = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut m).ok()?;
                Some(m.widthIncludingTrailingWhitespace.ceil())
            })
            .unwrap_or_else(|| super::settings_layout::Estimate.width(s, font));
        self.widths.borrow_mut().insert(key, w);
        w
    }
}

#[derive(Clone, Copy)]
struct Painter<'a> {
    rt: &'a ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    gfx: &'a Gfx,
    tile_fill: &'a ID2D1LinearGradientBrush,
    p: WindowPalette,
    /// Multiplies every colour's alpha (the section crossfade).
    alpha: f32,
}

impl Painter<'_> {
    fn set(&self, rgb: u32, a: f32) {
        unsafe { self.brush.SetColor(&color(rgb, a * self.alpha)) };
    }

    /// `0xRRGGBBAA`.
    fn set_rgba(&self, rgba: u32) {
        self.set(rgba >> 8, (rgba & 0xFF) as f32 / 255.0);
    }

    fn fill_round(&self, r: Rect, radius: f32, rgb: u32, a: f32) {
        self.set(rgb, a);
        unsafe {
            self.rt
                .FillRoundedRectangle(&rounded(r, radius), self.brush);
        }
    }

    /// A hairline just inside `r`.
    fn stroke_round(&self, r: Rect, radius: f32, rgba: u32) {
        self.set_rgba(rgba);
        let inner = Rect {
            x: r.x + 0.5,
            y: r.y + 0.5,
            w: r.w - 1.0,
            h: r.h - 1.0,
        };
        unsafe {
            self.rt
                .DrawRoundedRectangle(&rounded(inner, radius - 0.5), self.brush, 1.0, None);
        }
    }

    fn circle(&self, cx: f32, cy: f32, radius: f32, rgb: u32, a: f32) {
        self.set(rgb, a);
        unsafe {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2::new(cx, cy),
                    radiusX: radius,
                    radiusY: radius,
                },
                self.brush,
            );
        }
    }

    fn ring(&self, cx: f32, cy: f32, radius: f32, width: f32, rgb: u32, a: f32) {
        self.set(rgb, a);
        unsafe {
            self.rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2::new(cx, cy),
                    radiusX: radius,
                    radiusY: radius,
                },
                self.brush,
                width,
                None,
            );
        }
    }

    fn draw_str(&self, s: &str, format: &IDWriteTextFormat, r: Rect, rgb: u32, a: f32) {
        if s.is_empty() || r.w <= 0.0 {
            return;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        self.set(rgb, a);
        unsafe {
            self.rt.DrawText(
                &wide,
                format,
                &d2d_rect(r),
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn format(&self, font: Font, align: Align) -> &IDWriteTextFormat {
        &self.gfx.formats[&(font, align)]
    }

    fn text(&self, t: &Text) {
        let rgb = self.tone(t.tone);
        self.draw_str(&t.text, self.format(t.font, t.align), t.rect, rgb, 1.0);
    }

    fn tone(&self, tone: Tone) -> u32 {
        let p = &self.p;
        match tone {
            Tone::Primary => p.text,
            Tone::Secondary => p.secondary,
            Tone::Good => {
                if p.light {
                    0x1A7F37
                } else {
                    0x30D158
                }
            }
            Tone::Warn => {
                if p.light {
                    0x9A6700
                } else {
                    0xFFB340
                }
            }
            Tone::Bad => p.error,
        }
    }

    /// Accent text on the window: darkened in light mode, where the lighter
    /// accents would not read on white.
    fn link(&self) -> u32 {
        if self.p.light {
            mix(self.p.accent, 0x000000, 0.3)
        } else {
            self.p.accent
        }
    }

    /// A fill one step off `base` toward the text colour (hover, press).
    fn tint(&self, base: u32, amount: f32) -> u32 {
        mix(base, self.p.text, amount)
    }

    // ---- Sidebar ----

    fn sidebar(&self, s: &Scene) {
        let l = s.layout;
        self.logo(l.logo);
        if let Some(brand) = &l.brand {
            self.text(brand);
        }
        if l.nav.is_empty() {
            return;
        }
        // The selection pill slides between rows.
        let pill = rect_at(&l.nav.iter().map(|n| n.pill).collect::<Vec<_>>(), s.nav_pos);
        let tint = if self.p.light { 0.12 } else { 0.18 };
        for n in &l.nav {
            let selected = n.section == s.section;
            if !selected && s.hover == Some(Hit::Nav(n.section)) {
                let a = if s.pressed == s.hover { 0.09 } else { 0.06 };
                self.fill_round(n.pill, 8.0, self.p.text, a);
            }
        }
        self.fill_round(pill, 8.0, self.p.accent, tint);
        // The indicator: a short accent bar at the pill's left edge.
        let bar = Rect {
            x: pill.x + 3.0,
            y: pill.y + (pill.h - 16.0) / 2.0,
            w: 3.0,
            h: 16.0,
        };
        self.fill_round(bar, 1.5, self.p.accent, 1.0);
        for (i, n) in l.nav.iter().enumerate() {
            // How much the pill covers this row: colours follow it.
            let on = (1.0 - (s.nav_pos - i as f32).abs()).clamp(0.0, 1.0);
            let glyph = mix(self.p.secondary, self.link(), on);
            self.draw_str(n.section.glyph(), &self.gfx.glyph16, n.glyph, glyph, 1.0);
            let label = mix(mix(self.p.text, self.p.secondary, 0.25), self.p.text, on);
            self.draw_str(
                &n.label.text,
                self.format(Font::Nav, Align::Left),
                n.label.rect,
                label,
                1.0,
            );
        }
    }

    /// The app logo (assets/logo.svg) scaled into `r`.
    fn logo(&self, r: Rect) {
        if r.w <= 0.0 {
            return;
        }
        let k = r.w / 256.0;
        let rt = self.rt;
        unsafe {
            let mut previous = Matrix3x2::default();
            rt.GetTransform(&mut previous);
            let local = Matrix3x2 {
                M11: k,
                M12: 0.0,
                M21: 0.0,
                M22: k,
                M31: r.x,
                M32: r.y,
            };
            rt.SetTransform(&(local * previous));
            self.tile_fill.SetOpacity(self.alpha);
            rt.FillGeometry(&self.gfx.tile, self.tile_fill, None);
            if r.w >= 48.0 {
                // The inner hairline only shows at large sizes.
                self.set(0xFFFFFF, 0.1);
                rt.DrawGeometry(&self.gfx.tile, self.brush, 2.0 / k.max(0.25) * k, None);
            }
            self.fill_round(
                Rect {
                    x: 60.0,
                    y: 60.0,
                    w: 136.0,
                    h: 44.0,
                },
                22.0,
                0xFFFFFF,
                1.0,
            );
            self.circle(174.0, 82.0, 10.0, LOGO_DOT, 1.0);
            rt.SetTransform(&previous);
        }
    }

    // ---- Content ----

    fn content(&self, s: &Scene) {
        let l = s.layout;
        if let Some(title) = &l.title {
            self.text(title);
        }
        for card in &l.cards {
            self.fill_round(*card, CARD_RADIUS, self.p.card, 1.0);
            self.stroke_round(*card, CARD_RADIUS, self.p.border);
        }
        for d in &l.dividers {
            self.set_rgba(self.p.border);
            unsafe { self.rt.FillRectangle(&d2d_rect(*d), self.brush) };
        }
        if let Some(r) = l.preview {
            self.preview(r, s);
        }
        if let Some(r) = l.about_logo {
            self.logo(r);
        }
        if let Some(r) = l.note_glyph {
            self.draw_str(GLYPH_INFO, &self.gfx.glyph14, r, self.p.secondary, 1.0);
        }
        for (i, t) in l.texts.iter().enumerate() {
            match s.hover {
                // The accent row names the swatch under the pointer.
                Some(Hit::Accent(a)) if l.accent_desc == Some(i) => {
                    let name = text(s.lang, accent_key(a));
                    let format = self.format(t.font, t.align);
                    self.draw_str(name, format, t.rect, self.p.text, 1.0);
                }
                _ => self.text(t),
            }
        }
        for (r, tone) in &l.dots {
            let rgb = self.tone(*tone);
            self.circle(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, rgb, 1.0);
        }
        for (field, r) in &l.fields {
            self.field(*r, s.focused == Some(*field));
        }
        for c in &l.controls {
            match c {
                Control::Segmented {
                    group,
                    track,
                    items,
                    selected,
                } => {
                    let pos = s
                        .segments
                        .iter()
                        .find(|(g, _)| g == group)
                        .map_or(*selected as f32, |(_, v)| *v);
                    self.segmented(*track, items, pos, s);
                }
                Control::Toggle { hit, rect, .. } => {
                    self.toggle(*rect, s.toggle, s.hover == Some(*hit))
                }
                Control::Stepper {
                    minus,
                    plus,
                    value,
                    enabled,
                } => {
                    self.step_button(minus.0, GLYPH_MINUS, minus.1, s);
                    self.step_button(plus.0, GLYPH_PLUS, plus.1, s);
                    let a = if *enabled { 1.0 } else { 0.45 };
                    let format = self.format(value.font, value.align);
                    self.draw_str(&value.text, format, value.rect, self.p.text, a);
                }
                Control::Swatches { items, selected } => {
                    for (accent, r) in items {
                        self.swatch(
                            *r,
                            *accent,
                            *accent == *selected,
                            s.hover == Some(Hit::Accent(*accent)),
                        );
                    }
                }
                Control::Button(b) => self.button(b, s),
            }
        }
    }

    fn segmented(&self, track: Rect, items: &[Segment], pos: f32, s: &Scene) {
        let track_fill = if self.p.light { 0xEEEEF1 } else { self.p.bg };
        self.fill_round(track, 8.0, track_fill, 1.0);
        self.stroke_round(track, 8.0, self.p.border);
        let rects: Vec<Rect> = items.iter().map(|i| i.rect).collect();
        for item in items {
            let current = (pos - index_of(items, item) as f32).abs() < 0.5;
            if !current && s.hover == Some(item.hit) {
                let a = if s.pressed == s.hover { 0.12 } else { 0.08 };
                self.fill_round(item.rect, 6.0, self.p.text, a);
            }
        }
        let sel = rect_at(&rects, pos);
        self.fill_round(sel, 6.0, self.p.accent, 1.0);
        for item in items {
            // Text turns to ink as the selection slides under it.
            let covered = overlap(sel, item.rect) / item.rect.w.max(1.0);
            let rgb = mix(self.p.text, self.p.on_accent, covered.clamp(0.0, 1.0));
            let format = self.format(Font::Control, Align::Center);
            self.draw_str(&item.label, format, item.rect, rgb, 1.0);
        }
    }

    fn toggle(&self, r: Rect, v: f32, hover: bool) {
        let radius = r.h / 2.0;
        // Off: an outlined track; on: an accent fill. Cross-faded by `v`.
        let off_line = if self.p.light { 0x8A8A90 } else { 0x9A9AA0 };
        if v < 1.0 {
            let fill = if hover { 0.06 } else { 0.0 };
            if fill > 0.0 {
                self.fill_round(r, radius, self.p.text, fill * (1.0 - v));
            }
            self.set(off_line, 1.0 - v);
            let inner = Rect {
                x: r.x + 0.5,
                y: r.y + 0.5,
                w: r.w - 1.0,
                h: r.h - 1.0,
            };
            unsafe {
                self.rt
                    .DrawRoundedRectangle(&rounded(inner, radius - 0.5), self.brush, 1.0, None);
            }
        }
        if v > 0.0 {
            let fill = if hover {
                mix(self.p.accent, self.p.text, 0.12)
            } else {
                self.p.accent
            };
            self.fill_round(r, radius, fill, v);
        }
        let knob = 16.0;
        let travel = r.w - knob - 6.0;
        let cx = r.x + 3.0 + knob / 2.0 + travel * ease(v);
        let rgb = mix(off_line, self.p.on_accent, v);
        let grow = if hover { 1.0 } else { 0.0 };
        // The off knob is a little smaller, as in Windows 11.
        let radius = knob / 2.0 - 2.0 * (1.0 - v) + grow * 0.5;
        self.circle(cx, r.y + r.h / 2.0, radius, rgb, 1.0);
    }

    fn step_button(&self, r: Rect, glyph: &str, hit: Option<Hit>, s: &Scene) {
        let hovered = hit.is_some() && s.hover == hit;
        let pressed = hovered && s.pressed == hit;
        let fill = if pressed {
            self.tint(self.p.button_hover, 0.06)
        } else if hovered {
            self.p.button_hover
        } else {
            self.p.button
        };
        let a = if hit.is_some() { 1.0 } else { 0.45 };
        self.fill_round(r, 6.0, fill, a);
        if self.p.light {
            self.stroke_round(r, 6.0, self.p.border);
        }
        self.draw_str(glyph, &self.gfx.glyph12, r, self.p.text, a);
    }

    fn swatch(&self, r: Rect, accent: Accent, selected: bool, hover: bool) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rgb = accent.rgb();
        if selected {
            self.ring(cx, cy, r.w / 2.0 + 3.0, 2.0, rgb, 1.0);
            self.circle(cx, cy, r.w / 2.0 - 1.0, rgb, 1.0);
        } else {
            if hover {
                self.ring(cx, cy, r.w / 2.0 + 3.0, 2.0, rgb, 0.45);
            }
            self.circle(cx, cy, r.w / 2.0, rgb, 1.0);
        }
    }

    fn button(&self, b: &Button, s: &Scene) {
        let hovered = b.enabled && s.hover == Some(b.hit);
        let pressed = hovered && s.pressed == Some(b.hit);
        let a = if b.enabled { 1.0 } else { 0.4 };
        let format = self.format(Font::Control, Align::Center);
        match b.kind {
            ButtonKind::Primary | ButtonKind::Confirm => {
                let (base, ink) = if b.kind == ButtonKind::Confirm {
                    (if self.p.light { 0xC42B1C } else { 0xE5484D }, 0xFFFFFF)
                } else {
                    (self.p.accent, self.p.on_accent)
                };
                let shift = if self.p.light { 0x000000 } else { 0xFFFFFF };
                let fill = if pressed {
                    mix(base, 0x000000, 0.12)
                } else if hovered {
                    mix(base, shift, 0.1)
                } else {
                    base
                };
                self.fill_round(b.rect, 8.0, fill, a);
                self.draw_str(&b.label, format, b.rect, ink, a);
            }
            ButtonKind::Secondary => {
                let fill = if pressed {
                    self.tint(self.p.button_hover, 0.06)
                } else if hovered {
                    self.p.button_hover
                } else {
                    self.p.button
                };
                self.fill_round(b.rect, 8.0, fill, a);
                self.stroke_round(b.rect, 8.0, self.p.border);
                self.draw_str(&b.label, format, b.rect, self.p.text, a);
            }
            ButtonKind::Link => {
                if hovered {
                    let fa = if pressed { 0.1 } else { 0.06 };
                    self.fill_round(b.rect, 6.0, self.p.text, fa);
                }
                let rgb = self.link();
                let label = Rect {
                    w: b.rect.w - LINK_GLYPH_W,
                    ..b.rect
                };
                let left = self.format(Font::Control, Align::Left);
                let inset = Rect {
                    x: label.x + 8.0,
                    w: label.w - 8.0,
                    ..label
                };
                self.draw_str(&b.label, left, inset, rgb, a);
                let glyph = Rect {
                    x: b.rect.right() - LINK_GLYPH_W - 6.0,
                    w: LINK_GLYPH_W,
                    ..b.rect
                };
                self.draw_str(GLYPH_OPEN, &self.gfx.glyph12, glyph, rgb, a);
            }
        }
    }

    /// A rounded rect filled with a top-to-bottom gradient.
    fn gradient_round(&self, r: Rect, radius: f32, top: u32, bottom: u32) {
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: color(top, self.alpha),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: color(bottom, self.alpha),
            },
        ];
        unsafe {
            let Ok(stops) = self.rt.CreateGradientStopCollection(
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) else {
                return self.fill_round(r, radius, bottom, 1.0);
            };
            let Ok(brush) = self.rt.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: Vector2::new(r.x, r.y),
                    endPoint: Vector2::new(r.x, r.bottom()),
                },
                None,
                &stops,
            ) else {
                return self.fill_round(r, radius, bottom, 1.0);
            };
            self.rt.FillRoundedRectangle(&rounded(r, radius), &brush);
        }
    }

    /// The rounded box a native edit sits in; an accent underline while focused.
    fn field(&self, r: Rect, focused: bool) {
        self.fill_round(r, 6.0, self.p.field, 1.0);
        self.stroke_round(r, 6.0, self.p.border);
        if focused {
            // Clipped to the box's bottom edge, so the line follows its corners.
            unsafe {
                self.rt.PushAxisAlignedClip(
                    &d2d_rect(Rect {
                        y: r.bottom() - 2.0,
                        h: 2.0,
                        ..r
                    }),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
            }
            self.fill_round(r, 6.0, self.p.accent, 1.0);
            unsafe { self.rt.PopAxisAlignedClip() };
        } else if self.p.light {
            // A slightly darker bottom edge, as Windows 11 text boxes have.
            unsafe {
                self.rt.PushAxisAlignedClip(
                    &d2d_rect(Rect {
                        y: r.bottom() - 1.0,
                        h: 1.0,
                        ..r
                    }),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
            }
            self.fill_round(r, 6.0, 0x8A8A90, 0.6);
            unsafe { self.rt.PopAxisAlignedClip() };
        }
    }

    /// The compact island on a sketch of the top of a screen, in the chosen
    /// style, fill and accent.
    fn preview(&self, card: Rect, s: &Scene) {
        let screen = Rect {
            x: card.x + 12.0,
            y: card.y + 12.0,
            w: card.w - 24.0,
            h: card.h - 24.0,
        };
        // A wallpaper-like wash that picks up the accent toward the bottom.
        let (top, bottom) = if self.p.light {
            (0xDDE1E8, mix(0xEEF0F4, s.island.accent, 0.16))
        } else {
            (0x2A2C31, mix(0x1E1F23, s.island.accent, 0.14))
        };
        self.gradient_round(screen, 8.0, top, bottom);
        self.stroke_round(screen, 8.0, self.p.border);

        // The island, at its real size, centered at the top of the "screen".
        let base = shape_for(s.style, ShapeKind::MediaCompact);
        let shape = super::geometry::ShapeRect {
            x: screen.x + (screen.w - base.w) / 2.0,
            y: screen.y + base.y,
            ..base
        };
        let pal = s.island;
        self.set(pal.surface, 1.0);
        unsafe {
            match s.style {
                Style::Pill => {
                    self.rt.FillRoundedRectangle(
                        &rounded(
                            Rect {
                                x: shape.x,
                                y: shape.y,
                                w: shape.w,
                                h: shape.h,
                            },
                            shape.h / 2.0,
                        ),
                        self.brush,
                    );
                }
                Style::Notch => {
                    if let Ok(path) = notch_path(&self.gfx.d2d, shape) {
                        self.rt.FillGeometry(&path, self.brush, None);
                    }
                }
            }
        }
        let l = compact_layout(shape);
        // Album art: an accent gradient tile with a note.
        let art = l.art;
        self.fill_round(art, 5.0, mix(pal.accent, 0x000000, 0.35), 1.0);
        self.fill_round(
            Rect {
                h: art.h * 0.55,
                ..art
            },
            5.0,
            pal.accent,
            0.9,
        );
        self.draw_str(GLYPH_MUSIC, &self.gfx.glyph12, art, 0xFFFFFF, 0.95);
        let label = text(s.lang, Key::PreviewTrack);
        self.draw_str(label, &self.gfx.island_text, l.text, pal.text, 0.7);
        for (i, h) in PREVIEW_BARS.into_iter().enumerate() {
            let bar = Rect {
                x: l.bars.x + i as f32 * (BAR_W + BAR_GAP),
                y: l.bars.bottom() - h,
                w: BAR_W,
                h,
            };
            self.fill_round(bar, 1.0, pal.accent, 1.0);
        }
    }
}

fn index_of(items: &[Segment], item: &Segment) -> usize {
    items
        .iter()
        .position(|i| std::ptr::eq(i, item))
        .unwrap_or(0)
}

/// The rect at fractional index `pos` along `rects` (blending neighbours).
pub fn rect_at(rects: &[Rect], pos: f32) -> Rect {
    if rects.is_empty() {
        return Rect::default();
    }
    let max = (rects.len() - 1) as f32;
    let pos = pos.clamp(0.0, max);
    let i = pos.floor() as usize;
    let j = (i + 1).min(rects.len() - 1);
    let t = pos - i as f32;
    let (a, b) = (rects[i], rects[j]);
    let m = |x: f32, y: f32| x + (y - x) * t;
    Rect {
        x: m(a.x, b.x),
        y: m(a.y, b.y),
        w: m(a.w, b.w),
        h: m(a.h, b.h),
    }
}

fn overlap(a: Rect, b: Rect) -> f32 {
    (a.right().min(b.right()) - a.x.max(b.x)).max(0.0)
}

/// Cubic ease-out.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// `a` blended toward `b` by `t` (0xRRGGBB).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: alpha,
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

fn rounded(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    D2D1_ROUNDED_RECT {
        rect: d2d_rect(r),
        radiusX: radius,
        radiusY: radius,
    }
}

/// The logo tile (assets/logo.svg): a squircle from 12 to 244.
fn tile_geometry(d2d: &ID2D1Factory) -> Result<ID2D1PathGeometry> {
    let pt = |x: f32, y: f32| Vector2::new(x, y);
    let bez = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| D2D1_BEZIER_SEGMENT {
        point1: pt(a.0, a.1),
        point2: pt(b.0, b.1),
        point3: pt(c.0, c.1),
    };
    unsafe {
        let path = d2d.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(pt(82.0, 12.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(pt(174.0, 12.0));
        sink.AddBezier(&bez((227.2, 12.0), (244.0, 28.8), (244.0, 82.0)));
        sink.AddLine(pt(244.0, 174.0));
        sink.AddBezier(&bez((244.0, 227.2), (227.2, 244.0), (174.0, 244.0)));
        sink.AddLine(pt(82.0, 244.0));
        sink.AddBezier(&bez((28.8, 244.0), (12.0, 227.2), (12.0, 174.0)));
        sink.AddLine(pt(12.0, 82.0));
        sink.AddBezier(&bez((12.0, 28.8), (28.8, 12.0), (82.0, 12.0)));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(path)
    }
}

/// The island's Notch outline (as `render.rs` draws it) for a compact shape.
fn notch_path(d2d: &ID2D1Factory, s: super::geometry::ShapeRect) -> Result<ID2D1PathGeometry> {
    let r = 16.0f32.min(s.h - SHOULDER);
    let (left, right, top, bottom) = (s.x, s.x + s.w, s.y, s.y + s.h);
    let arc = |x: f32, y: f32, radius: f32, sweep: D2D1_SWEEP_DIRECTION| D2D1_ARC_SEGMENT {
        point: Vector2::new(x, y),
        size: windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_F {
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
        let path = d2d.CreatePathGeometry()?;
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

/// One line, vertically centered, trimmed with "…".
fn line_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    align: DWRITE_TEXT_ALIGNMENT,
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
        format.SetTextAlignment(align)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, w: f32) -> Rect {
        Rect {
            x,
            y: 0.0,
            w,
            h: 10.0,
        }
    }

    #[test]
    fn rect_at_blends_neighbours() {
        let rects = [r(0.0, 10.0), r(10.0, 20.0), r(30.0, 10.0)];
        assert_eq!(rect_at(&rects, 0.0), rects[0]);
        assert_eq!(rect_at(&rects, 2.0), rects[2]);
        assert_eq!(rect_at(&rects, 0.5), r(5.0, 15.0));
        assert_eq!(rect_at(&rects, 9.0), rects[2], "clamped");
    }

    #[test]
    fn mix_and_ease_ends() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(mix(0x102030, 0x102030, 0.5), 0x102030);
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!(ease(0.5) > 0.5, "ease-out runs ahead");
    }

    #[test]
    fn link_text_reads_on_white_for_every_accent() {
        // Light mode darkens the accent for text; check the worst case.
        for a in Accent::ALL {
            let ink = mix(a.rgb(), 0x000000, 0.3);
            let lum = |rgb: u32| {
                let c = |s: u32| {
                    let v = ((rgb >> s) & 0xFF) as f64 / 255.0;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * c(16) + 0.7152 * c(8) + 0.0722 * c(0)
            };
            let contrast = 1.05 / (lum(ink) + 0.05);
            assert!(contrast >= 3.0, "{a:?}: {contrast:.2}");
        }
    }
}
