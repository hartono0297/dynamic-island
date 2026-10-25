// Drawn and hit-tested by the panel renderer (Task 8).
#![allow(dead_code)]

//! Layout of the open panel's pages, the waiting strip, the page dots and the
//! compact pomodoro, in window logical pixels. Pure and deterministic: text is
//! trimmed by the renderer inside the rects given here.

use chrono::{Days, NaiveDate};

use super::geometry::{ExpandedLayout, Rect, ShapeRect};
use crate::calendar::{CalEvent, CalStatus, events_on, range_for};
use crate::model::{CalView, Hit, Page, PomoField};
use crate::notes::Note;

/// Month grid column labels, Monday first: Senin, Selasa, Rabu, Kamis, Jumat,
/// Sabtu, Minggu.
pub const WEEKDAY_LABELS: [&str; 7] = ["S", "S", "R", "K", "J", "S", "M"];

/// A clickable rect and what clicking it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub hit: Hit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalendarLayout {
    pub view: CalView,
    pub selected: NaiveDate,
    /// Month name, week range or day title.
    pub header: Rect,
    /// Hari, Minggu, Bulan.
    pub view_buttons: [(CalView, Rect); 3],
    pub prev: Rect,
    pub next: Rect,
    /// Month view: one rect above each grid column for `WEEKDAY_LABELS`.
    pub weekday_labels: Vec<Rect>,
    /// Month view: 42 days, Monday first.
    pub month_cells: Vec<(NaiveDate, Rect)>,
    /// Week view: 7 columns, Monday first; the top `WEEK_HEAD_H` holds the
    /// clickable day label.
    pub week_columns: Vec<(NaiveDate, Rect)>,
    /// Week view: "+N" under a column's rows, with the hidden count.
    pub week_more: Vec<(NaiveDate, usize, Rect)>,
    /// The area the event list (month: right column, day: full width) or the
    /// week columns occupy.
    pub list: Rect,
    /// Month view: the selected day's caption at the top of the list.
    pub caption: Option<Rect>,
    /// Event id and row.
    pub rows: Vec<(String, Rect)>,
    /// Month and day list: hidden event count and the "+N lagi" rect.
    pub more: Option<(usize, Rect)>,
    /// "+ Tambah acara".
    pub add: Rect,
    /// "Hubungkan Google" / "Hubungkan ulang Google".
    pub connect: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PomodoroLayout {
    /// "Pomodoro" on the left, the session label on the right.
    pub header: Rect,
    pub ring_center: (f32, f32),
    pub ring_radius: f32,
    /// Field, minus button, plus button, label. The value is drawn between
    /// the buttons (`stepper_value`).
    pub steppers: [(PomoField, Rect, Rect, Rect); 4],
    pub start_pause: Rect,
    pub reset: Rect,
    pub skip: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotesLayout {
    /// "Notes" title.
    pub header: Rect,
    /// "+ Baru".
    pub new_button: Rect,
    /// Note id and row.
    pub rows: Vec<(String, Rect)>,
    /// Notes after this chunk and the clickable rect: "+N lagi", or
    /// "Kembali ke atas" when the count is 0 (last chunk).
    pub more: Option<(usize, Rect)>,
}

/// One page's layout, for `regions`.
#[derive(Clone, Debug, PartialEq)]
pub enum PageLayout {
    Calendar(CalendarLayout),
    Pomodoro(PomodoroLayout),
    Notes(NotesLayout),
    Media(ExpandedLayout),
}

/// Side padding of the panel content.
const PAD_X: f32 = 20.0;
/// Content starts this far below the panel's top edge.
const PAD_TOP: f32 = 14.0;
const STRIP_TOP: f32 = 8.0;
const STRIP_H: f32 = 24.0;
const DOT: f32 = 6.0;
const DOT_GAP: f32 = 6.0;
const DOT_BOTTOM: f32 = 12.0;
/// Clear space between the content and the page dots.
const ABOVE_DOTS: f32 = 6.0;
const HEADER_H: f32 = 22.0;
const HEADER_GAP: f32 = 6.0;

const NAV_W: f32 = 22.0;
const VIEW_BUTTON_W: f32 = 50.0;
const VIEW_BUTTON_GAP: f32 = 2.0;
const CELL_W: f32 = 28.0;
const CELL_H: f32 = 24.0;
const CELL_GAP_X: f32 = 4.0;
const CELL_GAP_Y: f32 = 2.0;
const LABEL_H: f32 = 14.0;
const LIST_GAP: f32 = 16.0;
const CAPTION_H: f32 = 18.0;
const ROW_H: f32 = 34.0;
const MORE_H: f32 = 20.0;
const MORE_W: f32 = 90.0;
const ADD_W: f32 = 130.0;
const ADD_H: f32 = 22.0;
const ADD_GAP: f32 = 4.0;
const CONNECT_W: f32 = 180.0;
const CONNECT_H: f32 = 26.0;
const CONNECT_GAP: f32 = 6.0;
const WEEK_GAP: f32 = 6.0;
/// Height of the clickable day label at the top of a week column.
pub const WEEK_HEAD_H: f32 = 20.0;
const WEEK_ROWS: usize = 3;

const RING_RADIUS: f32 = 70.0;
const RING_X: f32 = 122.0;
const CONTROLS_X: f32 = 232.0;
const STEP_H: f32 = 26.0;
const STEP_GAP: f32 = 8.0;
const STEP_BUTTON: f32 = 22.0;
const STEP_LABEL_W: f32 = 128.0;
const STEP_VALUE_W: f32 = 60.0;
const ACTION_GAP: f32 = 14.0;
const ACTION_H: f32 = 28.0;

const NEW_W: f32 = 64.0;
const NOTE_H: f32 = 40.0;
const NOTE_GAP: f32 = 4.0;
const NOTES_MORE_W: f32 = 120.0;

const MEDIA_ART: f32 = 128.0;
const MEDIA_BUTTON_W: f32 = 56.0;
const MEDIA_BUTTON_H: f32 = 44.0;
const MEDIA_BUTTON_STEP: f32 = 72.0;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect {
        x,
        y,
        w: w.max(0.0),
        h: h.max(0.0),
    }
}

fn left(s: ShapeRect) -> f32 {
    s.x + PAD_X
}

fn right(s: ShapeRect) -> f32 {
    s.x + s.w - PAD_X
}

/// Top of the page content: below the waiting strip when it is shown.
fn content_top(s: ShapeRect, strip: bool) -> f32 {
    s.y + PAD_TOP + if strip { STRIP_H } else { 0.0 }
}

/// Lowest y the page content may reach: just above the page dots.
fn content_bottom(s: ShapeRect) -> f32 {
    s.y + s.h - DOT_BOTTOM - DOT - ABOVE_DOTS
}

/// How many of `n` rows of `pitch` fit in `avail`; when not all do, room is
/// kept for a `more_h` "+N" line under the shown rows.
fn rows_that_fit(n: usize, avail: f32, pitch: f32, gap: f32, more_h: f32) -> usize {
    if n as f32 * pitch - gap <= avail {
        n
    } else {
        ((avail - more_h) / pitch).floor().max(0.0) as usize
    }
}

/// The page dots and the index of the highlighted one.
pub fn page_dots(s: ShapeRect, page: Page) -> ([Rect; 4], usize) {
    let active = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
    let count = Page::ALL.len() as f32;
    let total = count * DOT + (count - 1.0) * DOT_GAP;
    let x0 = s.x + (s.w - total) / 2.0;
    let y = s.y + s.h - DOT_BOTTOM - DOT;
    let dots = std::array::from_fn(|i| rect(x0 + i as f32 * (DOT + DOT_GAP), y, DOT, DOT));
    (dots, active)
}

/// The Claude waiting strip across the content width, under the top edge.
pub fn waiting_strip(s: ShapeRect) -> Rect {
    rect(left(s), s.y + STRIP_TOP, s.w - 2.0 * PAD_X, STRIP_H)
}

/// Event rows stacked from `y` down to `bottom`: optional connect button,
/// rows, "+N lagi" if they do not all fit, then "+ Tambah acara".
struct Stack {
    connect: Option<Rect>,
    rows: Vec<(String, Rect)>,
    more: Option<(usize, Rect)>,
    add: Rect,
}

fn stack(x: f32, w: f32, mut y: f32, bottom: f32, ids: Vec<String>, connect: bool) -> Stack {
    let connect = if connect {
        let r = rect(x, y, w.min(CONNECT_W), CONNECT_H);
        y = r.bottom() + CONNECT_GAP;
        Some(r)
    } else {
        None
    };
    let avail = bottom - y - ADD_GAP - ADD_H;
    let total = ids.len();
    let shown = rows_that_fit(total, avail, ROW_H, 0.0, MORE_H);
    let rows: Vec<(String, Rect)> = ids
        .into_iter()
        .take(shown)
        .enumerate()
        .map(|(i, id)| (id, rect(x, y + i as f32 * ROW_H, w, ROW_H)))
        .collect();
    y += shown as f32 * ROW_H;
    let more = if shown < total {
        let r = rect(x, y, w.min(MORE_W), MORE_H);
        y = r.bottom();
        Some((total - shown, r))
    } else {
        None
    };
    Stack {
        connect,
        rows,
        more,
        add: rect(x, y + ADD_GAP, w.min(ADD_W), ADD_H),
    }
}

/// Month: grid on the left, the selected day's events on the right. Week:
/// seven columns of up to three events. Day: the selected day's events.
/// `events` is the whole snapshot; each day's share comes from `events_on`.
pub fn calendar_layout(
    s: ShapeRect,
    view: CalView,
    anchor: NaiveDate,
    selected: NaiveDate,
    events: &[CalEvent],
    status: &CalStatus,
    strip: bool,
) -> CalendarLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));

    // Header row, right to left: view selector, next, prev; the title takes the rest.
    let seg_w = 3.0 * VIEW_BUTTON_W + 2.0 * VIEW_BUTTON_GAP;
    let seg_x = r - seg_w;
    let views = [CalView::Day, CalView::Week, CalView::Month];
    let view_buttons = std::array::from_fn(|i| {
        let x = seg_x + i as f32 * (VIEW_BUTTON_W + VIEW_BUTTON_GAP);
        (views[i], rect(x, top, VIEW_BUTTON_W, HEADER_H))
    });
    let next = rect(seg_x - 12.0 - NAV_W, top, NAV_W, HEADER_H);
    let prev = rect(next.x - 4.0 - NAV_W, top, NAV_W, HEADER_H);
    let header = rect(l, top, prev.x - 8.0 - l, HEADER_H);
    let body = header.bottom() + HEADER_GAP;

    let ids_on = |d: NaiveDate| -> Vec<String> {
        events_on(events, d)
            .into_iter()
            .map(|e| e.id.clone())
            .collect()
    };
    let wants_connect = matches!(status, CalStatus::NeedsLogin | CalStatus::Reconnect);
    let mut layout = CalendarLayout {
        view,
        selected,
        header,
        view_buttons,
        prev,
        next,
        weekday_labels: Vec::new(),
        month_cells: Vec::new(),
        week_columns: Vec::new(),
        week_more: Vec::new(),
        list: Rect::default(),
        caption: None,
        rows: Vec::new(),
        more: None,
        add: Rect::default(),
        connect: None,
    };

    match view {
        CalView::Month => {
            let col_x = |c: usize| l + c as f32 * (CELL_W + CELL_GAP_X);
            layout.weekday_labels = (0..7)
                .map(|c| rect(col_x(c), body, CELL_W, LABEL_H))
                .collect();
            let cells_y = body + LABEL_H + CELL_GAP_Y;
            let (first, _) = range_for(CalView::Month, anchor);
            layout.month_cells = (0..42usize)
                .filter_map(|i| {
                    let d = first.checked_add_days(Days::new(i as u64))?;
                    let y = cells_y + (i / 7) as f32 * (CELL_H + CELL_GAP_Y);
                    Some((d, rect(col_x(i % 7), y, CELL_W, CELL_H)))
                })
                .collect();
            let list_x = col_x(6) + CELL_W + LIST_GAP;
            let list = rect(list_x, body, r - list_x, bottom - body);
            let caption = rect(list.x, list.y, list.w, CAPTION_H);
            let st = stack(
                list.x,
                list.w,
                caption.bottom() + 4.0,
                bottom,
                ids_on(selected),
                wants_connect,
            );
            layout.list = list;
            layout.caption = Some(caption);
            layout.rows = st.rows;
            layout.more = st.more;
            layout.add = st.add;
            layout.connect = st.connect;
        }
        CalView::Day => {
            let list = rect(l, body, r - l, bottom - body);
            let st = stack(
                list.x,
                list.w,
                list.y,
                bottom,
                ids_on(selected),
                wants_connect,
            );
            layout.list = list;
            layout.rows = st.rows;
            layout.more = st.more;
            layout.add = st.add;
            layout.connect = st.connect;
        }
        CalView::Week => {
            let (monday, _) = range_for(CalView::Week, anchor);
            let col_w = (r - l - 6.0 * WEEK_GAP) / 7.0;
            let col_h = WEEK_HEAD_H + WEEK_ROWS as f32 * ROW_H + MORE_H;
            for i in 0..7usize {
                let Some(d) = monday.checked_add_days(Days::new(i as u64)) else {
                    break;
                };
                let x = l + i as f32 * (col_w + WEEK_GAP);
                layout.week_columns.push((d, rect(x, body, col_w, col_h)));
                let ids = ids_on(d);
                let rows_y = body + WEEK_HEAD_H;
                if ids.len() > WEEK_ROWS {
                    let more = rect(x, rows_y + WEEK_ROWS as f32 * ROW_H, col_w, MORE_H);
                    layout.week_more.push((d, ids.len() - WEEK_ROWS, more));
                }
                layout.rows.extend(
                    ids.into_iter()
                        .take(WEEK_ROWS)
                        .enumerate()
                        .map(|(j, id)| (id, rect(x, rows_y + j as f32 * ROW_H, col_w, ROW_H))),
                );
            }
            layout.list = rect(l, body, r - l, col_h);
            layout.add = rect(l, layout.list.bottom() + 6.0, ADD_W, ADD_H);
            layout.connect = wants_connect
                .then(|| rect(layout.add.right() + 12.0, layout.add.y, CONNECT_W, ADD_H));
        }
    }
    layout
}

/// Big ring on the left; four steppers and the action buttons on the right.
pub fn pomodoro_layout(s: ShapeRect, strip: bool) -> PomodoroLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let l = left(s);
    let header = rect(l, top, right(s) - l, HEADER_H);
    let body = header.bottom() + 8.0;
    let body_h = bottom - body;
    let ring_center = (l + RING_X, body + body_h / 2.0);

    let steps_h = 4.0 * STEP_H + 3.0 * STEP_GAP;
    let block_h = steps_h + ACTION_GAP + ACTION_H;
    let block_y = body + (body_h - block_h) / 2.0;
    let x = l + CONTROLS_X;
    let fields = [
        PomoField::Focus,
        PomoField::ShortBreak,
        PomoField::LongBreak,
        PomoField::Sessions,
    ];
    let steppers = std::array::from_fn(|i| {
        let y = block_y + i as f32 * (STEP_H + STEP_GAP);
        let button_y = y + (STEP_H - STEP_BUTTON) / 2.0;
        let label = rect(x, y, STEP_LABEL_W, STEP_H);
        let minus = rect(label.right() + 8.0, button_y, STEP_BUTTON, STEP_BUTTON);
        let plus = rect(
            minus.right() + STEP_VALUE_W,
            button_y,
            STEP_BUTTON,
            STEP_BUTTON,
        );
        (fields[i], minus, plus, label)
    });
    let action_y = block_y + steps_h + ACTION_GAP;
    let start_pause = rect(x, action_y, 72.0, ACTION_H);
    let reset = rect(start_pause.right() + 8.0, action_y, 64.0, ACTION_H);
    let skip = rect(reset.right() + 8.0, action_y, 64.0, ACTION_H);
    PomodoroLayout {
        header,
        ring_center,
        ring_radius: RING_RADIUS,
        steppers,
        start_pause,
        reset,
        skip,
    }
}

/// Where a stepper's value ("25 mnt") goes: between its minus and plus.
pub fn stepper_value(minus: Rect, plus: Rect) -> Rect {
    rect(minus.right(), minus.y, plus.x - minus.right(), minus.h)
}

/// Rows of 40 px, as many as fit. When they do not all fit, the list shows
/// chunk `notes_page` (wrapping) and a "+N lagi" / "Kembali ke atas" line.
pub fn notes_layout(s: ShapeRect, notes: &[Note], notes_page: usize, strip: bool) -> NotesLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));
    let header = rect(l, top, r - l - NEW_W - 8.0, HEADER_H);
    let new_button = rect(r - NEW_W, top, NEW_W, HEADER_H);
    let list_y = header.bottom() + 8.0;
    let pitch = NOTE_H + NOTE_GAP;
    let n = notes.len();
    let k = rows_that_fit(n, bottom - list_y, pitch, NOTE_GAP, MORE_H).max(1);
    let (start, shown) = if n <= k {
        (0, n)
    } else {
        let start = (notes_page % n.div_ceil(k)) * k;
        (start, k.min(n - start))
    };
    let rows = notes[start..start + shown]
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let y = list_y + i as f32 * pitch;
            (note.id.clone(), rect(l, y, r - l, NOTE_H))
        })
        .collect();
    let more = (n > k).then(|| {
        let y = list_y + shown as f32 * pitch;
        (n - start - shown, rect(l, y, NOTES_MORE_W, MORE_H))
    });
    NotesLayout {
        header,
        new_button,
        rows,
        more,
    }
}

/// The v1 expanded arrangement at panel scale: large art on the left, title
/// over artist beside it, prev / play-pause / next under the text.
pub fn media_panel_layout(s: ShapeRect, strip: bool) -> ExpandedLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let art = rect(
        left(s) + 20.0,
        top + (bottom - top - MEDIA_ART) / 2.0,
        MEDIA_ART,
        MEDIA_ART,
    );
    let text_x = art.right() + 24.0;
    let text_w = right(s) - 20.0 - text_x;
    let button_y = art.bottom() - MEDIA_BUTTON_H;
    let button = |i: f32| {
        rect(
            text_x + i * MEDIA_BUTTON_STEP,
            button_y,
            MEDIA_BUTTON_W,
            MEDIA_BUTTON_H,
        )
    };
    ExpandedLayout {
        art,
        title: rect(text_x, art.y + 14.0, text_w, 26.0),
        artist: rect(text_x, art.y + 42.0, text_w, 22.0),
        buttons: [button(0.0), button(1.0), button(2.0)],
    }
}

/// Small ring on the left (center, radius 11), then the time and label.
pub fn pomodoro_compact_layout(s: ShapeRect) -> ((f32, f32), f32, Rect) {
    const RADIUS: f32 = 11.0;
    let center = (s.x + 22.0, s.y + s.h / 2.0);
    let text_x = center.0 + RADIUS + 9.0;
    let text = rect(text_x, s.y, s.x + s.w - 18.0 - text_x, s.h);
    (center, RADIUS, text)
}

/// Clickable regions of a page, plus the waiting strip when it is shown.
/// Clicks inside the panel that miss every region count as `Hit::Body`.
pub fn regions(s: ShapeRect, layout: &PageLayout, strip: bool) -> Vec<Region> {
    let mut out = Vec::new();
    let mut add = |rect: Rect, hit: Hit| out.push(Region { rect, hit });
    if strip {
        add(waiting_strip(s), Hit::Strip);
    }
    match layout {
        PageLayout::Calendar(l) => {
            for (view, r) in l.view_buttons {
                add(r, Hit::CalViewSel(view));
            }
            add(l.prev, Hit::CalPrev);
            add(l.next, Hit::CalNext);
            for (d, r) in &l.month_cells {
                add(*r, Hit::CalDate(*d));
            }
            for (d, r) in &l.week_columns {
                let head = Rect {
                    h: WEEK_HEAD_H,
                    ..*r
                };
                add(head, Hit::CalDate(*d));
            }
            for (id, r) in &l.rows {
                add(*r, Hit::CalEvent(id.clone()));
            }
            // In the day view "+N lagi" is only a label: that day is already open.
            if l.view == CalView::Month
                && let Some((_, r)) = l.more
            {
                add(r, Hit::CalMore(l.selected));
            }
            for (d, _, r) in &l.week_more {
                add(*r, Hit::CalMore(*d));
            }
            add(l.add, Hit::CalAdd);
            if let Some(r) = l.connect {
                add(r, Hit::CalConnect);
            }
        }
        PageLayout::Pomodoro(l) => {
            for (field, minus, plus, _) in l.steppers {
                add(minus, Hit::PomoAdjust(field, -1));
                add(plus, Hit::PomoAdjust(field, 1));
            }
            add(l.start_pause, Hit::PomoStartPause);
            add(l.reset, Hit::PomoReset);
            add(l.skip, Hit::PomoSkip);
        }
        PageLayout::Notes(l) => {
            add(l.new_button, Hit::NoteNew);
            for (id, r) in &l.rows {
                add(*r, Hit::NoteOpen(id.clone()));
            }
            if let Some((_, r)) = l.more {
                add(r, Hit::NotesMore);
            }
        }
        PageLayout::Media(l) => {
            let [prev, play, next] = l.buttons;
            add(prev, Hit::Prev);
            add(play, Hit::PlayPause);
            add(next, Hit::Next);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::EventTime;
    use crate::config::Style;
    use crate::ui::geometry::{ShapeKind, shape_contains, shape_for};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn all_day(id: &str, d: NaiveDate) -> CalEvent {
        CalEvent {
            id: id.into(),
            calendar_id: "primary".into(),
            title: id.into(),
            start: EventTime::AllDay(d),
            end: EventTime::AllDay(d.succ_opt().unwrap()),
            description: String::new(),
            color: None,
            editable: true,
        }
    }

    fn many_events(n: usize, d: NaiveDate) -> Vec<CalEvent> {
        (0..n).map(|i| all_day(&format!("ev{i}"), d)).collect()
    }

    fn notes(n: usize) -> Vec<Note> {
        (0..n)
            .map(|i| Note {
                id: format!("n{i}"),
                text: format!("note {i}"),
                updated: 1000 - i as i64,
            })
            .collect()
    }

    fn panel(style: Style) -> ShapeRect {
        shape_for(style, ShapeKind::Panel)
    }

    fn oct25() -> NaiveDate {
        date(2026, 10, 25)
    }

    /// Every page layout worth checking: all calendar views (with overflow,
    /// connect button), pomodoro, notes (overflow) and media.
    fn all_layouts(s: ShapeRect, strip: bool) -> Vec<(String, PageLayout)> {
        let mut events = many_events(20, oct25());
        // Spread some over the week so several week columns overflow.
        for d in 19..=24 {
            events.extend(many_events(5, date(2026, 10, d)).into_iter().map(|mut e| {
                e.id = format!("{}-{d}", e.id);
                e
            }));
        }
        let mut out = Vec::new();
        for view in [CalView::Day, CalView::Week, CalView::Month] {
            for status in [CalStatus::Ok, CalStatus::Reconnect] {
                let l = calendar_layout(s, view, oct25(), oct25(), &events, &status, strip);
                out.push((
                    format!("calendar {view:?} {status:?}"),
                    PageLayout::Calendar(l),
                ));
            }
        }
        out.push((
            "pomodoro".into(),
            PageLayout::Pomodoro(pomodoro_layout(s, strip)),
        ));
        for (n, page) in [(0, 0), (3, 0), (30, 0), (30, 7)] {
            let l = notes_layout(s, &notes(n), page, strip);
            out.push((format!("notes {n} p{page}"), PageLayout::Notes(l)));
        }
        out.push((
            "media".into(),
            PageLayout::Media(media_panel_layout(s, strip)),
        ));
        out
    }

    fn disjoint(a: Rect, b: Rect) -> bool {
        a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y
    }

    fn rect_inside(style: Style, s: ShapeRect, r: Rect) -> bool {
        const E: f32 = 0.01;
        r.w > 0.0
            && r.h > 0.0
            && [
                (r.x, r.y),
                (r.right() - E, r.y),
                (r.x, r.bottom() - E),
                (r.right() - E, r.bottom() - E),
            ]
            .into_iter()
            .all(|(x, y)| shape_contains(style, s, x, y))
    }

    #[test]
    fn page_dots_centered_and_highlight_index() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for (i, page) in Page::ALL.into_iter().enumerate() {
                let (dots, active) = page_dots(s, page);
                assert_eq!(active, i, "{page:?}");
                for d in dots {
                    assert_eq!((d.w, d.h), (6.0, 6.0));
                    assert_eq!(d.bottom(), s.y + s.h - 12.0);
                }
                for pair in dots.windows(2) {
                    assert_eq!(pair[1].x - pair[0].right(), 6.0);
                }
                let left = dots[0].x - s.x;
                let right = s.x + s.w - dots[3].right();
                assert_eq!(left, right, "centered");
            }
        }
    }

    #[test]
    fn month_cells_42_starting_monday() {
        let s = panel(Style::Pill);
        let l = calendar_layout(
            s,
            CalView::Month,
            date(2026, 10, 1),
            date(2026, 10, 25),
            &[],
            &CalStatus::Ok,
            false,
        );
        assert_eq!(l.month_cells.len(), 42);
        assert_eq!(l.month_cells[0].0, date(2026, 9, 28));
        assert_eq!(l.month_cells[41].0, date(2026, 11, 8));
        for pair in l.month_cells.windows(2) {
            assert_eq!(pair[0].0.succ_opt(), Some(pair[1].0));
        }
        let first = l.month_cells[0].1;
        assert_eq!((first.w, first.h), (28.0, 24.0));
        assert_eq!(first.x, s.x + 20.0);
        assert_eq!(l.month_cells[1].1.x, first.right() + 4.0);
        assert_eq!(l.month_cells[7].1.y, first.bottom() + 2.0);
        assert_eq!(l.month_cells[7].1.x, first.x);
        assert!(first.y >= l.header.bottom() + 6.0);
        // Weekday labels sit over the grid columns.
        assert_eq!(l.weekday_labels.len(), 7);
        for (label, (_, cell)) in l.weekday_labels.iter().zip(&l.month_cells) {
            assert_eq!((label.x, label.w), (cell.x, cell.w));
            assert!(label.bottom() <= cell.y);
        }
        // The list sits right of the grid.
        assert_eq!(l.list.x, l.month_cells[6].1.right() + 16.0);
        assert_eq!(l.list.right(), s.x + s.w - 20.0);
        assert!(l.week_columns.is_empty());
        assert_eq!(WEEKDAY_LABELS, ["S", "S", "R", "K", "J", "S", "M"]);
    }

    #[test]
    fn week_columns_monday_to_sunday() {
        let s = panel(Style::Pill);
        let events = many_events(5, date(2026, 10, 21));
        let l = calendar_layout(
            s,
            CalView::Week,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            false,
        );
        let days: Vec<NaiveDate> = l.week_columns.iter().map(|(d, _)| *d).collect();
        assert_eq!(days.first(), Some(&date(2026, 10, 19)));
        assert_eq!(days.last(), Some(&date(2026, 10, 25)));
        assert_eq!(days.len(), 7);
        assert_eq!(l.rows.len(), 3, "at most 3 rows per column");
        assert_eq!(l.week_more.len(), 1);
        assert_eq!(l.week_more[0].0, date(2026, 10, 21));
        assert_eq!(l.week_more[0].1, 2);
        assert!(l.month_cells.is_empty());
    }

    #[test]
    fn day_list_overflow_shows_more_count() {
        let s = panel(Style::Pill);
        let events = many_events(20, oct25());
        let l = calendar_layout(
            s,
            CalView::Day,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            false,
        );
        assert!(!l.rows.is_empty());
        let (hidden, more) = l.more.expect("overflow");
        assert_eq!(hidden, 20 - l.rows.len());
        let (dots, _) = page_dots(s, Page::Calendar);
        assert!(l.add.bottom() <= dots[0].y, "everything above the dots");
        assert!(more.y >= l.rows.last().unwrap().1.bottom());
        for (_, r) in &l.rows {
            assert_eq!(r.h, 34.0);
        }
        // Few events: all shown, no "+N".
        let l = calendar_layout(
            s,
            CalView::Day,
            oct25(),
            oct25(),
            &events[..2],
            &CalStatus::Ok,
            false,
        );
        assert_eq!(l.rows.len(), 2);
        assert_eq!(l.more, None);
    }

    #[test]
    fn connect_only_when_login_needed() {
        let s = panel(Style::Notch);
        let layout =
            |status| calendar_layout(s, CalView::Month, oct25(), oct25(), &[], &status, false);
        assert!(layout(CalStatus::NeedsLogin).connect.is_some());
        assert!(layout(CalStatus::Reconnect).connect.is_some());
        assert!(layout(CalStatus::Ok).connect.is_none());
        assert!(layout(CalStatus::Offline).connect.is_none());
    }

    #[test]
    fn notes_list_caps_rows() {
        let s = panel(Style::Pill);
        let all = notes(30);
        let l = notes_layout(s, &all, 0, false);
        let k = l.rows.len();
        assert!(k >= 3, "at least three rows fit, got {k}");
        assert!(k < 30);
        assert_eq!(l.rows[0].0, "n0");
        assert_eq!(l.more.as_ref().map(|m| m.0), Some(30 - k));
        for (_, r) in &l.rows {
            assert_eq!(r.h, 40.0);
        }
        let (dots, _) = page_dots(s, Page::Notes);
        assert!(l.more.unwrap().1.bottom() <= dots[0].y);

        // The next chunk starts after the first.
        let l1 = notes_layout(s, &all, 1, false);
        assert_eq!(l1.rows[0].0, format!("n{k}"));
        // The last chunk offers "Kembali ke atas" (count 0); one more wraps.
        let chunks = 30usize.div_ceil(k);
        let last = notes_layout(s, &all, chunks - 1, false);
        assert_eq!(last.more.as_ref().map(|m| m.0), Some(0));
        assert_eq!(last.rows.last().unwrap().0, "n29");
        let wrapped = notes_layout(s, &all, chunks, false);
        assert_eq!(wrapped.rows[0].0, "n0");

        // Few notes: no "+N"; none: no rows.
        let l = notes_layout(s, &notes(2), 0, false);
        assert_eq!((l.rows.len(), l.more), (2, None));
        let l = notes_layout(s, &[], 3, false);
        assert_eq!((l.rows.len(), l.more), (0, None));
    }

    #[test]
    fn strip_shifts_content_down_24() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            let strip = waiting_strip(s);
            assert_eq!(strip.h, 24.0);
            assert!(strip.y >= s.y && strip.y < s.y + 14.0, "under the top edge");

            let cal = |strip| {
                calendar_layout(
                    s,
                    CalView::Month,
                    oct25(),
                    oct25(),
                    &[],
                    &CalStatus::Ok,
                    strip,
                )
            };
            let (a, b) = (cal(false), cal(true));
            assert_eq!(b.header.y - a.header.y, 24.0);
            assert_eq!(b.month_cells[0].1.y - a.month_cells[0].1.y, 24.0);
            assert!(b.header.y >= strip.bottom());

            let (a, b) = (pomodoro_layout(s, false), pomodoro_layout(s, true));
            assert_eq!(b.header.y - a.header.y, 24.0);
            assert!(b.ring_center.1 > a.ring_center.1);

            let n = notes(3);
            let (a, b) = (notes_layout(s, &n, 0, false), notes_layout(s, &n, 0, true));
            assert_eq!(b.new_button.y - a.new_button.y, 24.0);
            assert_eq!(b.rows[0].1.y - a.rows[0].1.y, 24.0);

            let (a, b) = (media_panel_layout(s, false), media_panel_layout(s, true));
            assert!(b.art.y > a.art.y && b.art.y >= strip.bottom());
        }
    }

    #[test]
    fn strip_region_maps_to_strip_hit() {
        let s = panel(Style::Pill);
        let layout = PageLayout::Pomodoro(pomodoro_layout(s, true));
        let with = regions(s, &layout, true);
        assert!(with.contains(&Region {
            rect: waiting_strip(s),
            hit: Hit::Strip
        }));
        let layout = PageLayout::Pomodoro(pomodoro_layout(s, false));
        assert!(
            !regions(s, &layout, false)
                .iter()
                .any(|r| r.hit == Hit::Strip)
        );
    }

    #[test]
    fn regions_map_to_page_hits() {
        let s = panel(Style::Pill);
        let has = |regions: &[Region], hit: Hit| regions.iter().any(|r| r.hit == hit);

        let events = many_events(20, oct25());
        let l = calendar_layout(
            s,
            CalView::Month,
            oct25(),
            oct25(),
            &events,
            &CalStatus::NeedsLogin,
            false,
        );
        let r = regions(s, &PageLayout::Calendar(l), false);
        for hit in [
            Hit::CalPrev,
            Hit::CalNext,
            Hit::CalAdd,
            Hit::CalConnect,
            Hit::CalViewSel(CalView::Day),
            Hit::CalViewSel(CalView::Week),
            Hit::CalViewSel(CalView::Month),
            Hit::CalDate(date(2026, 9, 28)),
            Hit::CalEvent("ev0".into()),
            Hit::CalMore(oct25()),
        ] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }

        let l = calendar_layout(
            s,
            CalView::Week,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            false,
        );
        let r = regions(s, &PageLayout::Calendar(l), false);
        assert!(has(&r, Hit::CalDate(date(2026, 10, 19))));
        assert!(has(&r, Hit::CalMore(oct25())));

        // Day view's "+N lagi" is a label: the day is already open.
        let l = calendar_layout(
            s,
            CalView::Day,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            false,
        );
        assert!(l.more.is_some());
        let r = regions(s, &PageLayout::Calendar(l), false);
        assert!(!r.iter().any(|r| matches!(r.hit, Hit::CalMore(_))));

        let r = regions(s, &PageLayout::Pomodoro(pomodoro_layout(s, false)), false);
        for hit in [
            Hit::PomoStartPause,
            Hit::PomoReset,
            Hit::PomoSkip,
            Hit::PomoAdjust(PomoField::Focus, -1),
            Hit::PomoAdjust(PomoField::Sessions, 1),
        ] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }

        let r = regions(
            s,
            &PageLayout::Notes(notes_layout(s, &notes(30), 0, false)),
            false,
        );
        for hit in [Hit::NoteNew, Hit::NoteOpen("n0".into()), Hit::NotesMore] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }

        let r = regions(s, &PageLayout::Media(media_panel_layout(s, false)), false);
        for hit in [Hit::Prev, Hit::PlayPause, Hit::Next] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }
    }

    #[test]
    fn regions_have_no_overlaps_within_page() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for strip in [false, true] {
                for (name, layout) in all_layouts(s, strip) {
                    let r = regions(s, &layout, strip);
                    for (i, a) in r.iter().enumerate() {
                        for b in &r[i + 1..] {
                            assert!(
                                disjoint(a.rect, b.rect),
                                "{style:?} strip={strip} {name}: {a:?} overlaps {b:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn all_regions_inside_panel() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            let (dots, _) = page_dots(s, Page::Calendar);
            for d in dots {
                assert!(rect_inside(style, s, d), "{style:?} dot {d:?}");
            }
            for strip in [false, true] {
                for (name, layout) in all_layouts(s, strip) {
                    for r in regions(s, &layout, strip) {
                        assert!(
                            rect_inside(style, s, r.rect),
                            "{style:?} strip={strip} {name}: {r:?} outside"
                        );
                        if r.hit != Hit::Strip {
                            assert!(r.rect.bottom() <= dots[0].y, "{name}: {r:?} over the dots");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn drawn_rects_inside_panel() {
        // Rects that are drawn but not clickable must stay inside too.
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for strip in [false, true] {
                let mut drawn = Vec::new();
                for (_, layout) in all_layouts(s, strip) {
                    match layout {
                        PageLayout::Calendar(l) => {
                            drawn.extend([l.header, l.list]);
                            drawn.extend(l.weekday_labels.iter().copied());
                            drawn.extend(l.week_columns.iter().map(|(_, r)| *r));
                            drawn.extend(l.caption);
                            drawn.extend(l.more.map(|(_, r)| r));
                        }
                        PageLayout::Pomodoro(l) => {
                            drawn.push(l.header);
                            let (cx, cy) = l.ring_center;
                            let r = l.ring_radius + 4.0;
                            drawn.push(Rect {
                                x: cx - r,
                                y: cy - r,
                                w: 2.0 * r,
                                h: 2.0 * r,
                            });
                            for (_, minus, plus, label) in l.steppers {
                                drawn.extend([label, stepper_value(minus, plus)]);
                                assert!(label.right() <= minus.x);
                            }
                        }
                        PageLayout::Notes(l) => drawn.push(l.header),
                        PageLayout::Media(l) => drawn.extend([l.art, l.title, l.artist]),
                    }
                }
                for r in drawn {
                    assert!(rect_inside(style, s, r), "{style:?} strip={strip} {r:?}");
                }
            }
        }
    }

    #[test]
    fn pomodoro_ring_left_controls_right() {
        let s = panel(Style::Pill);
        let l = pomodoro_layout(s, false);
        assert_eq!(l.ring_radius, 70.0);
        assert!(l.ring_center.0 < s.x + s.w / 3.0);
        let fields: Vec<PomoField> = l.steppers.iter().map(|t| t.0).collect();
        assert_eq!(
            fields,
            [
                PomoField::Focus,
                PomoField::ShortBreak,
                PomoField::LongBreak,
                PomoField::Sessions
            ]
        );
        for (_, minus, plus, label) in l.steppers {
            assert!(label.x > l.ring_center.0 + l.ring_radius);
            assert!(minus.right() < plus.x);
        }
        assert!(l.start_pause.right() <= l.reset.x && l.reset.right() <= l.skip.x);
    }

    #[test]
    fn pomodoro_compact_fits_its_shape() {
        for style in [Style::Pill, Style::Notch] {
            let s = shape_for(style, ShapeKind::PomodoroCompact);
            let ((cx, cy), r, text) = pomodoro_compact_layout(s);
            assert_eq!(r, 11.0);
            assert_eq!(cy, s.y + s.h / 2.0);
            assert!(cx - r - 1.5 >= s.x + 4.0);
            assert!(text.x >= cx + r);
            assert!(text.right() <= s.x + s.w);
            assert!(rect_inside(style, s, text), "{style:?} {text:?}");
        }
    }
}
