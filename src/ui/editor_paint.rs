//! Direct2D drawing for the editors and their popovers, in the pattern of
//! the settings window (`settings_controls`): a software
//! `ID2D1HwndRenderTarget` on an opaque surface, grayscale text
//! antialiasing (no colour fringes), DirectWrite layouts with tabular
//! figures on digit runs only, and the Studio icon set.
//!
//! `Gfx` holds the device-independent resources and is shared by an editor
//! and its popover; each window keeps its own `Target`.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
    ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_WEIGHT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, IDWriteTypography,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Result;
use windows_numerics::{Matrix3x2, Vector2};

use super::editor_layout::Text;
use super::fonts::{self, Face, Role};
use super::geometry::Rect;
use super::icons::{Icon, IconSet};
use super::settings_controls::{Pen, color, d2d_rect, draw_toggle, line_format};
use crate::theme::WindowPalette;

/// Horizontal alignment of a line of text in its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Device-independent resources: factories, text formats, icons.
pub struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    formats: HashMap<(Text, Align), IDWriteTextFormat>,
    /// Tabular figures, laid on digit runs only (`fonts::text_layout`).
    tabular: IDWriteTypography,
    icons: IconSet,
    widths: RefCell<HashMap<(String, Text), f32>>,
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let body = Face::for_role(&dwrite, Role::Text);
            let display = Face::for_role(&dwrite, Role::Display);
            let mut formats = HashMap::new();
            for text in Text::ALL {
                let face = if text.display() { &display } else { &body };
                let weight = DWRITE_FONT_WEIGHT(i32::from(text.weight()));
                for (align, a) in [
                    (Align::Left, DWRITE_TEXT_ALIGNMENT_LEADING),
                    (Align::Center, DWRITE_TEXT_ALIGNMENT_CENTER),
                    (Align::Right, DWRITE_TEXT_ALIGNMENT_TRAILING),
                ] {
                    // One line, vertically centred, trimmed with "…".
                    let f = line_format(&dwrite, face, weight, text.size(), a)?;
                    formats.insert((text, align), f);
                }
            }
            Ok(Gfx {
                icons: IconSet::new(&d2d)?,
                tabular: fonts::tabular_figures(&dwrite)?,
                d2d,
                dwrite,
                formats,
                widths: RefCell::new(HashMap::new()),
            })
        }
    }

    /// `s` (UTF-16) in `text` laid out in a `w` × `h` box: tabular figures on
    /// its digits and, for display text, the display tracking. `width` and
    /// `Painter::text` both go through here, so what is measured is drawn.
    fn layout(
        &self,
        s: &[u16],
        text: Text,
        align: Align,
        w: f32,
        h: f32,
    ) -> Result<IDWriteTextLayout> {
        let format = &self.formats[&(text, align)];
        let layout = fonts::text_layout(&self.dwrite, s, format, w, h, &self.tabular)?;
        if text.display() {
            fonts::apply_tracking(&layout, s.len() as u32, text.size())?;
        }
        Ok(layout)
    }

    /// One-line width of `s` in `text`, in logical px.
    pub fn width(&self, s: &str, text: Text) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let key = (s.to_owned(), text);
        if let Some(w) = self.widths.borrow().get(&key) {
            return *w;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let w = self
            .layout(&wide, text, Align::Left, 4096.0, 64.0)
            .ok()
            .and_then(|layout| unsafe {
                let mut m = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut m).ok()?;
                Some(m.widthIncludingTrailingWhitespace.ceil())
            })
            .unwrap_or(s.chars().count() as f32 * text.size() * 0.6);
        self.widths.borrow_mut().insert(key, w);
        w
    }

    /// Draws one frame into `slot` (made on first use or after a loss) for a
    /// window of `size` physical px at `dpi`, cleared to `bg`.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        slot: &mut Option<Target>,
        hwnd: HWND,
        size: (u32, u32),
        dpi: f32,
        p: WindowPalette,
        bg: u32,
        draw: impl FnOnce(&Painter),
    ) -> Result<()> {
        if slot.is_none() {
            *slot = Some(self.target(hwnd, size, dpi)?);
        }
        let target = slot.as_ref().expect("target just made");
        let rt: &ID2D1RenderTarget = &target.rt;
        unsafe {
            rt.SetDpi(dpi, dpi);
            rt.BeginDraw();
            rt.SetTransform(&Matrix3x2::identity());
            rt.Clear(Some(&color(bg, 1.0)));
        }
        draw(&Painter {
            rt,
            brush: &target.brush,
            gfx: self,
            p,
        });
        let result = unsafe { rt.EndDraw(None, None) };
        if result.is_err() {
            // D2DERR_RECREATE_TARGET and the like: a new target next frame.
            *slot = None;
        }
        result
    }

    fn target(&self, hwnd: HWND, size: (u32, u32), dpi: f32) -> Result<Target> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                // Opaque: text is never blended against a transparent pixel.
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default()
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U {
                width: size.0.max(1),
                height: size.1.max(1),
            },
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        unsafe {
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
            // Grayscale: no ClearType colour fringes on any background.
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&color(0, 1.0), None)?;
            Ok(Target { rt, brush })
        }
    }
}

/// A window's render target and its one brush.
pub struct Target {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
}

impl Target {
    /// The window was resized (physical px).
    pub fn resize(&self, width: u32, height: u32) {
        unsafe {
            let _ = self.rt.Resize(&D2D_SIZE_U {
                width: width.max(1),
                height: height.max(1),
            });
        }
    }
}

/// Drawing primitives in the window palette for one frame.
pub struct Painter<'a> {
    rt: &'a ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    gfx: &'a Gfx,
    pub p: WindowPalette,
}

impl Painter<'_> {
    fn pen(&self, alpha: f32) -> Pen<'_> {
        Pen {
            rt: self.rt,
            brush: self.brush,
            alpha,
        }
    }

    pub fn fill_round(&self, r: Rect, radius: f32, rgb: u32, a: f32) {
        self.pen(1.0).fill_round(r, radius, rgb, a);
    }

    /// `rgba` is `0xRRGGBBAA`.
    pub fn fill_rgba(&self, r: Rect, radius: f32, rgba: u32, a: f32) {
        self.fill_round(r, radius, rgba >> 8, (rgba & 0xFF) as f32 / 255.0 * a);
    }

    /// A line of `width` just inside `r`.
    pub fn stroke(&self, r: Rect, radius: f32, width: f32, rgb: u32, a: f32) {
        let pen = self.pen(1.0);
        pen.set(rgb, a);
        pen.outline(r, radius, width);
    }

    /// A hairline just inside `r` in an `0xRRGGBBAA` colour.
    pub fn hairline(&self, r: Rect, radius: f32, rgba: u32, a: f32) {
        self.stroke(r, radius, 1.0, rgba >> 8, (rgba & 0xFF) as f32 / 255.0 * a);
    }

    /// A plain rectangle (rules) in an `0xRRGGBBAA` colour.
    pub fn rule(&self, r: Rect, rgba: u32) {
        let pen = self.pen(1.0);
        pen.set(rgba >> 8, (rgba & 0xFF) as f32 / 255.0);
        unsafe { self.rt.FillRectangle(&d2d_rect(r), self.brush) };
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, rgb: u32, a: f32) {
        self.pen(1.0).circle(cx, cy, radius, rgb, a);
    }

    /// One line of `s` in `r`, clipped, digits tabular.
    pub fn text(&self, s: &str, text: Text, align: Align, r: Rect, rgb: u32, a: f32) {
        if s.is_empty() || r.w <= 0.0 {
            return;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let Ok(layout) = self.gfx.layout(&wide, text, align, r.w, r.h) else {
            return;
        };
        self.pen(1.0).set(rgb, a);
        unsafe {
            self.rt.DrawTextLayout(
                Vector2::new(r.x, r.y),
                &layout,
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    }

    pub fn width(&self, s: &str, text: Text) -> f32 {
        self.gfx.width(s, text)
    }

    /// A Studio icon `size` px square centred in `r`; `duo` adds the accent
    /// fill.
    pub fn icon(&self, icon: Icon, r: Rect, size: f32, rgb: u32, a: f32, duo: bool) {
        let accent = duo.then_some(self.p.accent);
        let _ = self
            .gfx
            .icons
            .draw(self.rt, self.brush, icon, r, size, rgb, a, accent);
    }

    /// The settings window's toggle.
    pub fn toggle(&self, r: Rect, on: bool, hover: bool, a: f32) {
        draw_toggle(
            self.rt,
            self.brush,
            &self.p,
            r,
            f32::from(u8::from(on)),
            hover,
            a,
        );
    }
}
