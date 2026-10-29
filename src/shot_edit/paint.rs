//! Drawing the annotations. The canvas and the export both go through
//! `draw_annotations`, so what is saved is what was seen: the canvas passes
//! its `View` (scale and offset), the export the identity at full size.

use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_OPEN};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, ID2D1Bitmap, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::core::Result;
use windows_numerics::Vector2;

use super::gfx::{Gfx, color, rect_f};
use super::{
    Annotation, FRect, MARKER_ALPHA, MARKER_WIDTH_X, Pt, View, clip_to_image, pixel_block, pixelate,
};
use crate::capture::Shot;

/// Weight of annotation text (a medium DM Sans).
const TEXT_WEIGHT: i32 = 500;

/// The mosaic bitmaps made so far, by the region they cover. Valid for one
/// render target and one picture; dropped when either goes.
#[derive(Default)]
pub struct Pix {
    map: HashMap<(u32, u32, u32, u32), ID2D1Bitmap>,
}

impl Pix {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Mosaics kept at most; a drag makes a new one each step.
    const KEEP: usize = 24;

    fn get(
        &mut self,
        rt: &ID2D1RenderTarget,
        base: &Shot,
        region: (u32, u32, u32, u32),
    ) -> Option<ID2D1Bitmap> {
        if let Some(b) = self.map.get(&region) {
            return Some(b.clone());
        }
        let block = pixel_block(base.w, base.h);
        let px = pixelate(&base.bgra, base.w, base.h, region, block)?;
        let bitmap = super::gfx::bitmap(rt, region.2, region.3, &px).ok()?;
        if self.map.len() >= Self::KEEP {
            self.map.clear();
        }
        self.map.insert(region, bitmap.clone());
        Some(bitmap)
    }
}

/// Draws `doc` over a picture already drawn: each annotation placed by
/// `view`. `skip` leaves one out (the text being edited in a native EDIT).
pub fn draw_annotations(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &Gfx,
    base: &Shot,
    doc: &[Annotation],
    view: &View,
    skip: Option<usize>,
    pix: &mut Pix,
) {
    for (i, ann) in doc.iter().enumerate() {
        if skip == Some(i) {
            continue;
        }
        let _ = draw_one(rt, brush, gfx, base, ann, view, pix);
    }
}

fn v(view: &View, p: Pt) -> Vector2 {
    let (x, y) = view.to_view(p);
    Vector2::new(x, y)
}

fn draw_one(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &Gfx,
    base: &Shot,
    ann: &Annotation,
    view: &View,
    pix: &mut Pix,
) -> Result<()> {
    unsafe {
        match ann {
            Annotation::Rect { r, style } => {
                brush.SetColor(&color(style.color, 1.0));
                let (x, y) = view.to_view(Pt::new(r.x, r.y));
                rt.DrawRectangle(
                    &rect_f(x, y, r.w * view.scale, r.h * view.scale),
                    brush,
                    style.width * view.scale,
                    &gfx.round,
                );
            }
            Annotation::Arrow { a, b, style } => {
                brush.SetColor(&color(style.color, 1.0));
                let w = style.width * view.scale;
                rt.DrawLine(v(view, *a), v(view, *b), brush, w, &gfx.round);
                let [l, r] = arrow_head(*a, *b, style.width);
                let head = [l, *b, r];
                stroke_path(rt, brush, gfx, view, &head, w)?;
            }
            Annotation::Pen { pts, style } => {
                brush.SetColor(&color(style.color, 1.0));
                stroke_path(rt, brush, gfx, view, pts, style.width * view.scale)?;
            }
            Annotation::Marker { pts, style } => {
                brush.SetColor(&color(style.color, MARKER_ALPHA));
                let w = style.width * MARKER_WIDTH_X * view.scale;
                stroke_path(rt, brush, gfx, view, pts, w)?;
            }
            Annotation::Text {
                pos,
                text,
                size,
                color: rgb,
            } => {
                let (x, y) = view.to_view(*pos);
                gfx.text(
                    rt,
                    brush,
                    text,
                    (x, y),
                    size * view.scale,
                    TEXT_WEIGHT,
                    *rgb,
                    1.0,
                );
            }
            Annotation::Pixelate { r } => {
                let Some(region) = clip_to_image(*r, base.w, base.h) else {
                    return Ok(());
                };
                let Some(bitmap) = pix.get(rt, base, region) else {
                    return Ok(());
                };
                let (x, y) = view.to_view(Pt::new(region.0 as f32, region.1 as f32));
                let dest = rect_f(
                    x,
                    y,
                    region.2 as f32 * view.scale,
                    region.3 as f32 * view.scale,
                );
                rt.DrawBitmap(
                    &bitmap,
                    Some(&dest),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                );
            }
        }
    }
    Ok(())
}

/// The two ends of an arrow's head: back from `b` along the shaft, spread
/// to either side (image px).
pub fn arrow_head(a: Pt, b: Pt, width: f32) -> [Pt; 2] {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy).max(f32::EPSILON);
    let (ux, uy) = (dx / len, dy / len);
    // Never longer than the shaft itself.
    let head = (width * 3.0 + 10.0).min(len);
    let spread = 0.5f32;
    let rot = |sign: f32| {
        let (s, c) = (spread.sin() * sign, spread.cos());
        Pt::new(
            b.x - head * (ux * c - uy * s),
            b.y - head * (ux * s + uy * c),
        )
    };
    [rot(1.0), rot(-1.0)]
}

/// An open path through `pts` stroked `width` px wide (one geometry, so a
/// translucent marker does not darken where it crosses itself).
fn stroke_path(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &Gfx,
    view: &View,
    pts: &[Pt],
    width: f32,
) -> Result<()> {
    let Some(first) = pts.first() else {
        return Ok(());
    };
    unsafe {
        if pts.len() == 1 {
            // A dot: a zero-length line with round caps.
            rt.DrawLine(v(view, *first), v(view, *first), brush, width, &gfx.round);
            return Ok(());
        }
        let geometry = gfx.d2d.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.BeginFigure(v(view, *first), D2D1_FIGURE_BEGIN_FILLED);
        for p in &pts[1..] {
            sink.AddLine(v(view, *p));
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        rt.DrawGeometry(&geometry, brush, width, &gfx.round);
    }
    Ok(())
}

/// The outline and handles of the selected annotation, in `accent`.
pub fn draw_selection(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &Gfx,
    ann: &Annotation,
    view: &View,
    accent: u32,
    ui: f32,
) {
    let b: FRect = ann.bounds();
    let (x, y) = view.to_view(Pt::new(b.x, b.y));
    unsafe {
        brush.SetColor(&color(accent, 0.9));
        rt.DrawRectangle(
            &rect_f(x, y, b.w * view.scale, b.h * view.scale),
            brush,
            1.0 * ui,
            None,
        );
        let hs = 4.5 * ui;
        for (_, p) in ann.handles() {
            let (hx, hy) = view.to_view(p);
            let r = rect_f(hx - hs, hy - hs, hs * 2.0, hs * 2.0);
            brush.SetColor(&color(0xFFFFFF, 1.0));
            rt.FillRectangle(&r, brush);
            brush.SetColor(&color(accent, 1.0));
            rt.DrawRectangle(&r, brush, 1.5 * ui, None);
        }
    }
    let _ = gfx;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrow_head_sits_behind_the_tip_on_both_sides() {
        let [l, r] = arrow_head(Pt::new(0.0, 0.0), Pt::new(100.0, 0.0), 4.0);
        // Behind the tip (x < 100), mirrored around the shaft.
        assert!(l.x < 100.0 && r.x < 100.0);
        assert!((l.x - r.x).abs() < 1e-3);
        assert!((l.y + r.y).abs() < 1e-3);
        assert!(l.y.abs() > 1.0);
        // The same head turned by 90 degrees.
        let [l2, r2] = arrow_head(Pt::new(0.0, 0.0), Pt::new(0.0, 100.0), 4.0);
        assert!(l2.y < 100.0 && r2.y < 100.0);
        assert!((l2.y - r2.y).abs() < 1e-3);
    }

    #[test]
    fn a_short_arrow_has_a_short_head() {
        let [l, _] = arrow_head(Pt::new(0.0, 0.0), Pt::new(5.0, 0.0), 12.0);
        assert!((l.x - 5.0).abs() <= 5.0 && l.y.abs() <= 5.0);
        // A zero-length arrow does not produce NaN.
        let [l, r] = arrow_head(Pt::new(3.0, 3.0), Pt::new(3.0, 3.0), 4.0);
        assert!(l.x.is_finite() && r.y.is_finite());
    }
}
