//! Drawing of the open panel's pages, the compact pomodoro, the waiting strip
//! and the page dots. A child module of `render`, so it shares the renderer's
//! private state and drawing helpers. Positions come from `panel_layout`.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::mem::ManuallyDrop;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL,
    D2D1_ELLIPSE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, ID2D1Geometry,
};
use windows::core::Result;
use windows_numerics::{Matrix3x2, Vector2};

use super::{
    Frame, GLYPH_NEXT, GLYPH_PAUSE, GLYPH_PLAY, GLYPH_PREV, Renderer, SECONDARY_ALPHA, color,
    d2d_rect,
};
use crate::calendar::{CalEvent, CalStatus, EventTime, events_on, range_for};
use crate::config::Style;
use crate::i18n::{self, Key, Lang, text};
use crate::model::{CalView, Hit, MediaInfo, Notice, Page};
use crate::notes::Note;
use crate::pomodoro::{Phase, PomoStatus, Pomodoro};
use crate::ui::geometry::{ExpandedLayout, Rect, ShapeKind, ShapeRect, shape_for};
use crate::ui::panel_layout::{
    CalendarLayout, NotesLayout, PageLayout, PomodoroLayout, WEEK_HEAD_H, calendar_layout,
    media_panel_layout, notes_layout, page_dots, pomodoro_compact_layout, pomodoro_layout, regions,
    waiting_strip,
};

/// Pomodoro phases keep their red and green whatever the accent.
const FOCUS: u32 = 0xEF5350;
const BREAK: u32 = 0x2ECC71;
const CLAUDE: u32 = 0xD97757;
const OFFLINE_W: f32 = 52.0;
const GLYPH_LEFT: &str = "\u{E76B}";
const GLYPH_RIGHT: &str = "\u{E76C}";
/// Height of the compact shapes the panel grows from.
const COMPACT_H: f32 = 34.0;

impl Renderer {
    /// The panel: content laid out for the resting panel shape, clipped to the
    /// current (possibly still morphing) shape and faded in while it grows.
    /// Clickable regions exist only once the shape has settled and no page
    /// is sliding.
    pub(super) fn paint_panel(
        &mut self,
        style: Style,
        shape: ShapeRect,
        frame: &Frame,
    ) -> Result<()> {
        let rest = shape_for(style, ShapeKind::Panel);
        let settled = shape == rest;
        // Content is fully visible by ~70 % of the growth, so the panel reads
        // as open while the shape is still easing into place.
        let fade = if settled {
            1.0
        } else {
            ((shape.h - COMPACT_H) / (rest.h - COMPACT_H) / 0.7).clamp(0.0, 1.0)
        };
        if fade <= 0.0 {
            return Ok(());
        }
        let mask = self.shape_geometry(style, shape)?;
        self.push_mask(&mask, fade);
        let painted = self.paint_panel_content(rest, frame, settled);
        self.set_offset(0.0);
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.PopLayer() };
        painted
    }

    fn paint_panel_content(&mut self, s: ShapeRect, frame: &Frame, settled: bool) -> Result<()> {
        let strip = frame.strip.is_some();
        let page = frame.page;
        let layout = page_layout(s, frame, page, strip);
        let slide = frame.page_slide.clamp(-1.0, 1.0);
        if slide != 0.0 {
            // The page it came from sits one panel width further on.
            let index = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
            let from = if slide > 0.0 {
                index.checked_sub(1)
            } else {
                Some(index + 1).filter(|i| *i < Page::ALL.len())
            };
            if let Some(from) = from {
                let old = page_layout(s, frame, Page::ALL[from], strip);
                self.set_offset((slide - slide.signum()) * s.w);
                self.paint_page(frame, &old)?;
            }
        }
        self.set_offset(slide * s.w);
        self.paint_page(frame, &layout)?;
        self.set_offset(0.0);

        if let Some(notice) = frame.strip {
            self.paint_strip(s, notice);
        }
        let (dots, active) = page_dots(s, page);
        for (i, dot) in dots.into_iter().enumerate() {
            let p = &self.palette;
            self.set_color(if i == active { p.accent } else { p.dot_off }, 1.0);
            self.fill_round(dot, dot.w / 2.0);
        }
        // No clicks on a page still sliding in: its controls are not where
        // the regions say yet, and the old page's are still under the pointer.
        if settled && slide == 0.0 {
            let mut hits = regions(s, &layout, strip);
            if matches!(layout, PageLayout::Media(_)) && frame.media.is_none() {
                hits.retain(|r| !matches!(r.hit, Hit::Prev | Hit::PlayPause | Hit::Next));
            }
            self.regions.regions = hits;
            // The v1 media buttons must never fire on another page.
            self.regions.buttons = None;
        }
        Ok(())
    }

    fn paint_page(&mut self, frame: &Frame, layout: &PageLayout) -> Result<()> {
        match layout {
            PageLayout::Calendar(l) => {
                self.paint_calendar(l, frame);
                Ok(())
            }
            PageLayout::Pomodoro(l) => self.paint_pomodoro(l, frame),
            PageLayout::Notes(l) => {
                self.paint_notes(l, frame.notes, frame.today);
                Ok(())
            }
            PageLayout::Media(l) => self.paint_media(l, frame.media),
        }
    }

    fn paint_calendar(&mut self, l: &CalendarLayout, frame: &Frame) {
        let lang = i18n::lang();
        let today = frame.today;
        let snap = frame.calendar;
        let events = &snap.events;
        let title = match l.view {
            CalView::Month => i18n::month_title(lang, frame.cal_anchor),
            CalView::Week => i18n::week_title(lang, range_for(CalView::Week, frame.cal_anchor).0),
            CalView::Day => i18n::day_title(lang, l.selected),
        };
        let mut header = l.header;
        if snap.status == CalStatus::Offline {
            let tag = Rect {
                x: header.right() - OFFLINE_W,
                y: header.y + 3.0,
                w: OFFLINE_W,
                h: header.h - 6.0,
            };
            header.w = (header.w - OFFLINE_W - 8.0).max(0.0);
            self.set_color(self.palette.track, 1.0);
            self.fill_round(tag, tag.h / 2.0);
            self.text(
                text(lang, Key::Offline),
                &self.fonts.tiny_center,
                tag,
                SECONDARY_ALPHA,
            );
        }
        self.text(&title, &self.fonts.heading, header, 1.0);
        self.text(GLYPH_LEFT, &self.fonts.icon_small, l.prev, 0.8);
        self.text(GLYPH_RIGHT, &self.fonts.icon_small, l.next, 0.8);

        // View selector: one dark track, the active segment lit.
        let first = l.view_buttons[0].1;
        let last = l.view_buttons[2].1;
        let track = Rect {
            x: first.x - 2.0,
            w: last.right() - first.x + 4.0,
            ..first
        };
        self.set_color(self.palette.raised, 1.0);
        self.fill_round(track, 7.0);
        for (view, r) in l.view_buttons {
            let on = view == l.view;
            if on {
                self.set_color(self.palette.raised_hi, 1.0);
                self.fill_round(inset(r, 0.0, 2.0), 5.0);
            }
            let label = text(
                lang,
                match view {
                    CalView::Day => Key::ViewDay,
                    CalView::Week => Key::ViewWeek,
                    CalView::Month => Key::ViewMonth,
                },
            );
            self.text(
                label,
                &self.fonts.small_center,
                r,
                if on { 1.0 } else { SECONDARY_ALPHA },
            );
        }

        match l.view {
            CalView::Month => {
                for (label, r) in i18n::weekday_initials(lang).iter().zip(&l.weekday_labels) {
                    self.text(label, &self.fonts.tiny_center, *r, 0.5);
                }
                for (d, r) in &l.month_cells {
                    self.paint_day_cell(*d, *r, today, l.selected, frame.cal_anchor, events);
                }
                // Divider between the grid and the list.
                self.set_color(self.palette.track, 1.0);
                self.fill_round(
                    Rect {
                        x: l.list.x - 8.5,
                        y: l.list.y,
                        w: 1.0,
                        h: l.list.h,
                    },
                    0.0,
                );
                if let Some(caption) = l.caption {
                    self.text(
                        &i18n::day_title(lang, l.selected),
                        &self.fonts.small,
                        caption,
                        0.6,
                    );
                }
                self.paint_rows(lang, l, events, snap.status, false);
            }
            CalView::Day => self.paint_rows(lang, l, events, snap.status, false),
            CalView::Week => {
                for (d, col) in &l.week_columns {
                    let head = Rect {
                        h: WEEK_HEAD_H,
                        ..*col
                    };
                    let body = Rect {
                        y: col.y + WEEK_HEAD_H + 2.0,
                        h: col.h - WEEK_HEAD_H - 2.0,
                        ..*col
                    };
                    let p = self.palette;
                    self.set_color(p.column, 1.0);
                    self.fill_round(body, 6.0);
                    let label = format!("{} {}", i18n::day_short(lang, *d), d.day());
                    if *d == today {
                        self.set_color(p.accent, 1.0);
                        self.fill_round(head, 6.0);
                        let bold = &self.fonts.small_center_bold;
                        self.text_colored(&label, bold, head, p.on_accent, 1.0);
                    } else {
                        if *d == l.selected {
                            self.stroke_round(inset(head, 0.5, 0.5), 6.0, p.accent);
                        }
                        self.text(&label, &self.fonts.small_center, head, 1.0);
                    }
                }
                for (_, n, r) in &l.week_more {
                    self.text(&format!("+{n}"), &self.fonts.small_center, *r, 0.7);
                }
                self.paint_rows(lang, l, events, snap.status, true);
            }
        }
    }

    fn paint_day_cell(
        &self,
        d: NaiveDate,
        r: Rect,
        today: NaiveDate,
        selected: NaiveDate,
        anchor: NaiveDate,
        events: &[CalEvent],
    ) {
        let number = d.day().to_string();
        let digits = Rect { h: r.h - 3.0, ..r };
        let is_today = d == today;
        let p = self.palette;
        if is_today {
            self.set_color(p.accent, 1.0);
            self.fill_round(r, 6.0);
            let bold = &self.fonts.small_center_bold;
            self.text_colored(&number, bold, digits, p.on_accent, 1.0);
        } else {
            if d == selected {
                self.stroke_round(inset(r, 0.5, 0.5), 6.0, p.accent);
            }
            let in_month = d.month() == anchor.month() && d.year() == anchor.year();
            let alpha = if in_month { 1.0 } else { 0.35 };
            self.text(&number, &self.fonts.small_center, digits, alpha);
        }
        if let Some(first) = events_on(events, d).first() {
            let dot = if is_today {
                p.on_accent
            } else {
                first.color.unwrap_or(p.accent)
            };
            self.set_color(dot, 1.0);
            self.fill_round(
                Rect {
                    x: r.x + r.w / 2.0 - 2.0,
                    y: r.bottom() - 6.0,
                    w: 4.0,
                    h: 4.0,
                },
                2.0,
            );
        }
    }

    /// Event rows, "+N lagi", "+ Tambah acara" and the connect button.
    fn paint_rows(
        &self,
        lang: Lang,
        l: &CalendarLayout,
        events: &[CalEvent],
        status: CalStatus,
        narrow: bool,
    ) {
        for (id, r) in &l.rows {
            let Some(ev) = events.iter().find(|e| &e.id == id) else {
                continue;
            };
            self.set_color(ev.color.unwrap_or(self.palette.accent), 1.0);
            self.fill_round(
                Rect {
                    x: r.x + 2.0,
                    y: r.y + 5.0,
                    w: 3.0,
                    h: r.h - 10.0,
                },
                1.5,
            );
            let text_x = r.x + 11.0;
            let w = r.right() - 2.0 - text_x;
            let (title_font, time_font) = if narrow {
                (&self.fonts.small, &self.fonts.tiny)
            } else {
                (&self.fonts.secondary, &self.fonts.small)
            };
            let title = Rect {
                x: text_x,
                y: r.y + 2.0,
                w,
                h: 17.0,
            };
            let time = Rect {
                x: text_x,
                y: r.y + 18.0,
                w,
                h: 14.0,
            };
            self.text(ev.shown_title(), title_font, title, 1.0);
            self.text(&event_time_label(lang, ev), time_font, time, 0.6);
        }
        if let Some((n, r)) = l.more {
            self.text(
                &more_label(lang, n),
                &self.fonts.small,
                inset(r, 2.0, 0.0),
                0.7,
            );
        }
        self.text(
            text(lang, Key::AddEventRow),
            &self.fonts.secondary,
            l.add,
            0.8,
        );
        if let Some(r) = l.connect {
            self.set_color(self.palette.accent, 1.0);
            self.fill_round(r, 7.0);
            let label = text(
                lang,
                if status == CalStatus::Reconnect {
                    Key::ReconnectGoogle
                } else {
                    Key::ConnectGoogle
                },
            );
            let ink = self.palette.on_accent;
            self.text_colored(label, &self.fonts.body_center, r, ink, 1.0);
        }
    }

    fn paint_pomodoro(&mut self, l: &PomodoroLayout, frame: &Frame) -> Result<()> {
        let lang = i18n::lang();
        let pomo = frame.pomodoro;
        let settings = pomo.settings();
        let status = pomo.status();
        let accent = phase_color(pomo.phase());
        self.text(
            text(lang, Key::Pomodoro),
            &self.fonts.heading,
            l.header,
            1.0,
        );

        let (cx, cy) = l.ring_center;
        self.ring(
            (cx, cy),
            l.ring_radius,
            8.0,
            pomo.progress(frame.now),
            accent,
        )?;
        let r = l.ring_radius;
        self.text(
            &mmss(pomo.remaining(frame.now)),
            &self.fonts.big_time,
            Rect {
                x: cx - r,
                y: cy - 21.0,
                w: 2.0 * r,
                h: 42.0,
            },
            1.0,
        );

        self.text(&phase_label(lang, pomo), &self.fonts.phase, l.phase, 1.0);
        let session = i18n::session_of(
            lang,
            pomo.session().min(settings.sessions),
            settings.sessions,
        );
        self.text(&session, &self.fonts.body_center, l.session, 0.6);
        self.set_color(accent, 1.0);
        self.fill_round(l.start_pause, 7.0);
        self.text_colored(
            start_label(lang, status),
            &self.fonts.body_center,
            l.start_pause,
            0xFFFFFF,
            1.0,
        );
        for (r, key) in [(l.reset, Key::Reset), (l.skip, Key::Skip)] {
            self.set_color(self.palette.track, 1.0);
            self.fill_round(r, 7.0);
            self.text(text(lang, key), &self.fonts.body_center, r, 1.0);
        }
        Ok(())
    }

    fn paint_notes(&self, l: &NotesLayout, notes: &[Note], today: NaiveDate) {
        let lang = i18n::lang();
        self.text(text(lang, Key::Notes), &self.fonts.heading, l.header, 1.0);
        self.text(
            text(lang, Key::NewNote),
            &self.fonts.body_right,
            l.new_button,
            0.8,
        );
        if notes.is_empty() {
            let hint = Rect {
                y: l.header.bottom() + 8.0,
                h: 20.0,
                ..l.header
            };
            self.text(
                text(lang, Key::NotesEmpty),
                &self.fonts.secondary,
                hint,
                0.5,
            );
        }
        for (id, r) in &l.rows {
            let Some(note) = notes.iter().find(|n| &n.id == id) else {
                continue;
            };
            self.set_color(self.palette.raised, 1.0);
            self.fill_round(*r, 8.0);
            let inner = inset(*r, 10.0, 0.0);
            self.text(
                first_line(&note.text),
                &self.fonts.secondary,
                Rect {
                    y: r.y + 4.0,
                    h: 18.0,
                    ..inner
                },
                1.0,
            );
            let when = DateTime::from_timestamp(note.updated, 0)
                .map(|t| note_time_label(lang, t.with_timezone(&Local).naive_local(), today))
                .unwrap_or_default();
            self.text(
                &when,
                &self.fonts.tiny,
                Rect {
                    y: r.y + 22.0,
                    h: 14.0,
                    ..inner
                },
                0.55,
            );
        }
        if let Some((n, r)) = l.more {
            self.text(
                &more_label(lang, n),
                &self.fonts.small,
                inset(r, 2.0, 0.0),
                0.7,
            );
        }
    }

    fn paint_media(&mut self, l: &ExpandedLayout, media: Option<&MediaInfo>) -> Result<()> {
        self.paint_art(l.art, 12.0, media.and_then(|m| m.art.as_deref()))?;
        let Some(media) = media else {
            self.text(
                i18n::tr(Key::NothingPlaying),
                &self.fonts.media_title,
                l.title,
                0.8,
            );
            self.text(
                i18n::tr(Key::NothingPlayingHint),
                &self.fonts.media_artist,
                l.artist,
                0.5,
            );
            return Ok(());
        };
        self.text(&media.title, &self.fonts.media_title, l.title, 1.0);
        self.text(
            &media.artist,
            &self.fonts.media_artist,
            l.artist,
            SECONDARY_ALPHA,
        );
        let play = if media.playing {
            GLYPH_PAUSE
        } else {
            GLYPH_PLAY
        };
        let [prev, middle, next] = l.buttons;
        self.text(GLYPH_PREV, &self.fonts.icon_panel, prev, 1.0);
        self.text(play, &self.fonts.icon_panel_large, middle, 1.0);
        self.text(GLYPH_NEXT, &self.fonts.icon_panel, next, 1.0);
        Ok(())
    }

    /// Orange dot and "Claude butuh respon · {project}".
    fn paint_strip(&self, s: ShapeRect, notice: &Notice) {
        let r = waiting_strip(s);
        self.set_color(self.palette.raised, 1.0);
        self.fill_round(r, 8.0);
        self.set_color(CLAUDE, 1.0);
        self.fill_round(
            Rect {
                x: r.x + 10.0,
                y: r.y + r.h / 2.0 - 4.0,
                w: 8.0,
                h: 8.0,
            },
            4.0,
        );
        let text = if notice.body.is_empty() {
            notice.title.clone()
        } else {
            format!("{} · {}", notice.title, notice.body)
        };
        let area = Rect {
            x: r.x + 26.0,
            w: r.w - 36.0,
            ..r
        };
        self.text(&text, &self.fonts.secondary, area, 1.0);
    }

    /// Small ring, `mm:ss`, then "Fokus 2/4", "Istirahat" or "Siap · klik".
    pub(super) fn paint_pomodoro_compact(&mut self, shape: ShapeRect, frame: &Frame) -> Result<()> {
        let pomo = frame.pomodoro;
        let (center, radius, text) = pomodoro_compact_layout(shape);
        self.ring(
            center,
            radius,
            3.0,
            pomo.progress(frame.now),
            phase_color(pomo.phase()),
        )?;
        let paused = matches!(pomo.status(), PomoStatus::Paused { .. });
        let time = Rect { w: 46.0, ..text };
        let label = Rect {
            x: time.right() + 4.0,
            w: (text.right() - time.right() - 4.0).max(0.0),
            ..text
        };
        self.text(
            &mmss(pomo.remaining(frame.now)),
            &self.fonts.title,
            time,
            if paused { 0.6 } else { 1.0 },
        );
        self.text(
            &compact_label(i18n::lang(), pomo),
            &self.fonts.secondary,
            label,
            SECONDARY_ALPHA,
        );
        Ok(())
    }

    /// Grey track plus a clockwise arc from 12 o'clock covering `progress`.
    fn ring(
        &self,
        (cx, cy): (f32, f32),
        r: f32,
        stroke: f32,
        progress: f32,
        rgb: u32,
    ) -> Result<()> {
        let target = self.target.as_ref().expect("target ensured by paint");
        let ellipse = D2D1_ELLIPSE {
            point: Vector2::new(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            target.brush.SetColor(&color(self.palette.track, 1.0));
            target.rt.DrawEllipse(&ellipse, &target.brush, stroke, None);
        }
        let p = progress.clamp(0.0, 1.0);
        if p <= 0.0 {
            return Ok(());
        }
        unsafe {
            target.brush.SetColor(&color(rgb, 1.0));
            if p >= 0.999 {
                target.rt.DrawEllipse(&ellipse, &target.brush, stroke, None);
                return Ok(());
            }
            let angle = -FRAC_PI_2 + TAU * p;
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2::new(cx + r * angle.cos(), cy + r * angle.sin()),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if p > 0.5 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            target
                .rt
                .DrawGeometry(&path, &target.brush, stroke, &self.round_cap);
        }
        Ok(())
    }

    /// Everything drawn until `PopLayer` is clipped to `mask` at `opacity`.
    fn push_mask(&self, mask: &ID2D1Geometry, opacity: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F {
                left: -f32::MAX,
                top: -f32::MAX,
                right: f32::MAX,
                bottom: f32::MAX,
            },
            geometricMask: ManuallyDrop::new(Some(mask.clone())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe { target.rt.PushLayer(&params, &target.layer) };
        // The layer keeps its own reference while pushed.
        drop(ManuallyDrop::into_inner(params.geometricMask));
    }

    /// Shifts everything drawn next `dx` logical px to the right.
    fn set_offset(&self, dx: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.SetTransform(&Matrix3x2::translation(dx, 0.0)) };
    }

    fn set_color(&self, rgb: u32, alpha: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.brush.SetColor(&color(rgb, alpha)) };
    }

    /// A 1 px outline.
    fn stroke_round(&self, r: Rect, radius: f32, rgb: u32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.brush.SetColor(&color(rgb, 1.0));
            target.rt.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: d2d_rect(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                &target.brush,
                1.0,
                None,
            );
        }
    }
}

fn page_layout(s: ShapeRect, frame: &Frame, page: Page, strip: bool) -> PageLayout {
    match page {
        Page::Calendar => PageLayout::Calendar(calendar_layout(
            s,
            frame.cal_view,
            frame.cal_anchor,
            frame.cal_selected,
            &frame.calendar.events,
            &frame.calendar.status,
            frame.day_page,
            strip,
        )),
        Page::Pomodoro => PageLayout::Pomodoro(pomodoro_layout(s, strip)),
        Page::Notes => PageLayout::Notes(notes_layout(s, frame.notes, frame.notes_page, strip)),
        Page::Media => PageLayout::Media(media_panel_layout(s, strip)),
    }
}

fn inset(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect {
        x: r.x + dx,
        y: r.y + dy,
        w: (r.w - 2.0 * dx).max(0.0),
        h: (r.h - 2.0 * dy).max(0.0),
    }
}

fn phase_color(phase: Phase) -> u32 {
    match phase {
        Phase::Focus => FOCUS,
        Phase::ShortBreak | Phase::LongBreak => BREAK,
    }
}

fn phase_name(lang: Lang, phase: Phase) -> &'static str {
    text(
        lang,
        match phase {
            Phase::Focus => Key::Focus,
            Phase::ShortBreak => Key::Break,
            Phase::LongBreak => Key::LongBreak,
        },
    )
}

/// "+N lagi", or "Kembali ke atas" on the last chunk (`n` = 0).
fn more_label(lang: Lang, n: usize) -> String {
    if n == 0 {
        text(lang, Key::BackToTop).into()
    } else {
        i18n::more_count(lang, n)
    }
}

/// "10.00 – 11.30", "Seharian", "Seharian · 24 Okt – 26 Okt", or both dates
/// for a timed event spanning days.
fn event_time_label(lang: Lang, ev: &CalEvent) -> String {
    let date = |d| i18n::short_date(lang, d);
    let hm = |t| i18n::time_hm(lang, t);
    match (&ev.start, &ev.end) {
        (EventTime::AllDay(s), EventTime::AllDay(e)) => {
            let all_day = text(lang, Key::AllDay);
            let last = e.pred_opt().unwrap_or(*s);
            if last <= *s {
                all_day.into()
            } else {
                format!("{all_day} · {} – {}", date(*s), date(last))
            }
        }
        (EventTime::At(s), EventTime::At(e)) => {
            let (s, e) = (s.naive_local(), e.naive_local());
            if s.date() == e.date() {
                format!("{} – {}", hm(s), hm(e))
            } else {
                format!(
                    "{} {} – {} {}",
                    date(s.date()),
                    hm(s),
                    date(e.date()),
                    hm(e)
                )
            }
        }
        _ => String::new(),
    }
}

/// "hari ini 09.12", "kemarin", "23 Okt", or "23 Okt 2025" in another year.
fn note_time_label(lang: Lang, updated: NaiveDateTime, today: NaiveDate) -> String {
    let d = updated.date();
    if d == today {
        format!(
            "{} {}",
            text(lang, Key::Today),
            i18n::time_hm(lang, updated)
        )
    } else if Some(d) == today.pred_opt() {
        text(lang, Key::Yesterday).into()
    } else if d.year() == today.year() {
        i18n::short_date(lang, d)
    } else {
        format!("{} {}", i18n::short_date(lang, d), d.year())
    }
}

/// The first non-blank line, trimmed.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// Whole seconds rounded up, so a fresh 25-minute phase shows "25:00".
fn mmss(d: Duration) -> String {
    let secs = d.as_millis().div_ceil(1000);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn compact_label(lang: Lang, pomo: &Pomodoro) -> String {
    if pomo.status() == PomoStatus::Ready {
        return text(lang, Key::ReadyClick).into();
    }
    match pomo.phase() {
        Phase::Focus => {
            let n = pomo.settings().sessions;
            let focus = text(lang, Key::Focus);
            format!("{focus} {}/{n}", pomo.session().min(n))
        }
        Phase::ShortBreak | Phase::LongBreak => text(lang, Key::Break).into(),
    }
}

/// Beside the big ring: the phase, marked when paused or waiting.
fn phase_label(lang: Lang, pomo: &Pomodoro) -> String {
    let name = phase_name(lang, pomo.phase());
    match pomo.status() {
        PomoStatus::Paused { .. } => format!("{name} · {}", text(lang, Key::Paused)),
        PomoStatus::Ready => format!("{name} {}", text(lang, Key::Ready)),
        PomoStatus::Idle | PomoStatus::Running { .. } => name.into(),
    }
}

fn start_label(lang: Lang, status: PomoStatus) -> &'static str {
    text(
        lang,
        match status {
            PomoStatus::Idle | PomoStatus::Ready => Key::Start,
            PomoStatus::Running { .. } => Key::Pause,
            PomoStatus::Paused { .. } => Key::Resume,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pomodoro::PomoSettings;
    use chrono::{NaiveTime, TimeZone};
    use std::time::Instant;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(d: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
        d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    fn event(start: EventTime, end: EventTime) -> CalEvent {
        CalEvent {
            id: "e".into(),
            calendar_id: "primary".into(),
            title: "e".into(),
            start,
            end,
            description: String::new(),
            color: None,
            editable: true,
        }
    }

    fn local(t: NaiveDateTime) -> EventTime {
        EventTime::At(Local.from_local_datetime(&t).earliest().unwrap())
    }

    #[test]
    fn event_times() {
        let l = Lang::Id;
        let d = date(2026, 10, 25);
        let one = event(EventTime::AllDay(d), EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(event_time_label(l, &one), "Seharian");
        let multi = event(
            EventTime::AllDay(date(2026, 10, 24)),
            EventTime::AllDay(date(2026, 10, 27)),
        );
        assert_eq!(event_time_label(l, &multi), "Seharian · 24 Okt – 26 Okt");
        let timed = event(local(at(d, 10, 0)), local(at(d, 11, 30)));
        assert_eq!(event_time_label(l, &timed), "10.00 – 11.30");
        let night = event(local(at(d, 22, 0)), local(at(date(2026, 10, 26), 2, 0)));
        assert_eq!(event_time_label(l, &night), "25 Okt 22.00 – 26 Okt 02.00");
    }

    #[test]
    fn event_times_in_english() {
        let l = Lang::En;
        let d = date(2026, 10, 25);
        let one = event(EventTime::AllDay(d), EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(event_time_label(l, &one), "All day");
        let multi = event(
            EventTime::AllDay(date(2026, 10, 24)),
            EventTime::AllDay(date(2026, 10, 27)),
        );
        assert_eq!(event_time_label(l, &multi), "All day · 24 Oct – 26 Oct");
        let night = event(local(at(d, 22, 0)), local(at(date(2026, 10, 26), 2, 0)));
        assert_eq!(event_time_label(l, &night), "25 Oct 22:00 – 26 Oct 02:00");
    }

    #[test]
    fn note_times_relative_to_today() {
        let l = Lang::Id;
        let today = date(2026, 10, 25);
        assert_eq!(
            note_time_label(l, at(today, 9, 12), today),
            "hari ini 09.12"
        );
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 24), 23, 0), today),
            "kemarin"
        );
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 23), 8, 0), today),
            "23 Okt"
        );
        assert_eq!(
            note_time_label(l, at(date(2025, 10, 23), 8, 0), today),
            "23 Okt 2025"
        );
    }

    #[test]
    fn note_times_in_english() {
        let l = Lang::En;
        let today = date(2026, 10, 25);
        assert_eq!(note_time_label(l, at(today, 9, 12), today), "Today 09:12");
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 24), 23, 0), today),
            "Yesterday"
        );
        assert_eq!(
            note_time_label(l, at(date(2025, 10, 23), 8, 0), today),
            "23 Oct 2025"
        );
    }

    #[test]
    fn more_lines() {
        assert_eq!(more_label(Lang::Id, 0), "Kembali ke atas");
        assert_eq!(more_label(Lang::Id, 4), "+4 lagi");
        assert_eq!(more_label(Lang::En, 0), "Back to top");
        assert_eq!(more_label(Lang::En, 4), "+4 more");
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line("\n  \n  Beli tinta  \nlain"), "Beli tinta");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn mmss_rounds_up_and_allows_long_phases() {
        assert_eq!(mmss(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(mmss(Duration::from_millis(1_121_400)), "18:42");
        assert_eq!(mmss(Duration::from_secs(120 * 60)), "120:00");
        assert_eq!(mmss(Duration::ZERO), "00:00");
    }

    #[test]
    fn pomodoro_labels_by_status() {
        let l = Lang::Id;
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        assert_eq!(start_label(l, p.status()), "Mulai");
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Fokus 1/4");
        assert_eq!(start_label(l, p.status()), "Jeda");
        assert_eq!(phase_label(l, &p), "Fokus");
        p.start_pause(t0);
        assert_eq!(start_label(l, p.status()), "Lanjut");
        assert_eq!(phase_label(l, &p), "Fokus · dijeda");
        p.skip(t0);
        assert_eq!(compact_label(l, &p), "Siap · klik");
        assert_eq!(phase_label(l, &p), "Istirahat siap");
        assert_eq!(phase_color(p.phase()), BREAK);
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Istirahat");
    }

    #[test]
    fn pomodoro_labels_in_english() {
        let l = Lang::En;
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        assert_eq!(start_label(l, p.status()), "Start");
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Focus 1/4");
        assert_eq!(start_label(l, p.status()), "Pause");
        p.start_pause(t0);
        assert_eq!(start_label(l, p.status()), "Resume");
        assert_eq!(phase_label(l, &p), "Focus · paused");
        p.skip(t0);
        assert_eq!(compact_label(l, &p), "Ready · click");
        assert_eq!(phase_label(l, &p), "Break ready");
    }
}
