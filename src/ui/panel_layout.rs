//! Layout of the open panel's pages, the waiting strip, the page dots and the
//! compact pomodoro, in window logical pixels. Pure and deterministic: text is
//! trimmed by the renderer inside the rects given here, and the one width that
//! depends on text (the calendar title, which the arrows follow) is passed in.

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use super::geometry::{ExpandedLayout, Rect, ShapeRect};
use crate::calendar::{CalEvent, CalStatus, EventTime, events_on, range_for};
use crate::cli::CARD_LINES;
use crate::model::{CalView, Hit, Page, Shortcut};
use crate::notes::Note;
use crate::reminders::{Reminder, local_time};

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
    /// Month name, week range or day title, as wide as the title drawn.
    pub header: Rect,
    /// Hari, Minggu, Bulan.
    pub view_buttons: [(CalView, Rect); 3],
    /// Right after the title.
    pub prev: Rect,
    pub next: Rect,
    /// Month view: one rect above each grid column for its weekday name.
    pub weekday_labels: Vec<Rect>,
    /// Month view: 42 days, Monday first.
    pub month_cells: Vec<(NaiveDate, Rect)>,
    /// Week view: 7 columns, Monday first; the top `WEEK_HEAD_H` holds the
    /// clickable day name and number.
    pub week_columns: Vec<(NaiveDate, Rect)>,
    /// Week view: "+N" at a column's foot, with the hidden count.
    pub week_more: Vec<(NaiveDate, usize, Rect)>,
    /// Month: the event list right of the grid. Week: the columns. Day: the
    /// timeline (its first hour line at the top).
    pub list: Rect,
    /// Month view: the selected day's caption at the top of the list.
    pub caption: Option<Rect>,
    /// Event id and row (month list), chip (week) or all-day band and
    /// time block (day).
    pub rows: Vec<(String, Rect)>,
    /// Month and day: events after the shown ones and the "+N lagi" rect;
    /// in the day view a count of 0 means the last page ("Kembali ke atas").
    pub more: Option<(usize, Rect)>,
    /// Day view: the hour lines shown, each hour with its line's y.
    pub hours: Vec<(u32, f32)>,
    /// Day view of today: the "now" line's y, when inside the hours shown.
    pub now_y: Option<f32>,
    /// "+ Tambah acara".
    pub add: Rect,
    /// "Hubungkan Google" / "Hubungkan ulang Google".
    pub connect: Option<Rect>,
    /// The "Offline" tag after the arrows, while the calendar is offline; the
    /// title is cut shorter so it always fits.
    pub offline: Option<Rect>,
}

/// The timer and its controls only; durations are set in Settings → Pomodoro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PomodoroLayout {
    /// "Pomodoro".
    pub header: Rect,
    /// "25 · 5 · 15 menit", right of the title.
    pub durations: Rect,
    pub ring_center: (f32, f32),
    pub ring_radius: f32,
    /// The countdown, a little above the ring's centre: as wide as the
    /// stroke's inner edge allows at the digits' top and bottom, so text
    /// centred in it never meets it.
    pub time: Rect,
    /// The phase in capitals ("FOKUS"), under the countdown inside the ring.
    pub phase: Rect,
    /// "Sesi 2 dari 4" and its dots, right of the ring.
    pub session: Rect,
    /// The big round primary button: start, pause or resume.
    pub start_pause: Rect,
    /// The small round buttons either side of it.
    pub reset: Rect,
    pub skip: Rect,
    /// "Berikutnya: Istirahat 5 menit".
    pub next_up: Rect,
}

/// The notes and reminders pages: a title, "+ Baru" and the items.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayout {
    /// "Notes" / "Pengingat" title.
    pub header: Rect,
    /// "+ Baru".
    pub new_button: Rect,
    /// Reminders: each day group's heading in this chunk, by the day it
    /// stands for ("Hari ini", "Besok", a date). Empty for notes.
    pub groups: Vec<(NaiveDate, Rect)>,
    /// Note id and card, or reminder id and row.
    pub rows: Vec<(String, Rect)>,
    /// Items after this chunk and the clickable rect: "+N lagi", or
    /// "Kembali ke atas" when the count is 0 (last chunk).
    pub more: Option<(usize, Rect)>,
}

/// The CLI page: a terminal-like card with the last command and its last
/// lines, and a button that opens the CLI window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CliLayout {
    /// "CLI".
    pub header: Rect,
    /// Right of the header: "● berjalan" or the shell's name.
    pub status: Rect,
    pub card: Rect,
    /// `> command` first, then up to `cli::CARD_LINES` output lines.
    pub lines: [Rect; 1 + CARD_LINES],
    /// "Ketik perintah".
    pub button: Rect,
}

/// The Pintasan page: two rows of four tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShortcutsLayout {
    /// "Pintasan".
    pub header: Rect,
    /// In `Shortcut::ALL` order, row by row.
    pub tiles: [(Shortcut, Rect); 8],
}

/// One page's layout, for `regions`.
// Built on the stack for one frame and dropped; boxing the calendar would
// only add an allocation per frame.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum PageLayout {
    Calendar(CalendarLayout),
    Pomodoro(PomodoroLayout),
    Reminders(ListLayout),
    Notes(ListLayout),
    Media(ExpandedLayout),
    Cli(CliLayout),
    Shortcuts(ShortcutsLayout),
}

/// Side padding of the panel content.
const PAD_X: f32 = 20.0;
/// Content starts this far below the panel's top edge.
const PAD_TOP: f32 = 14.0;
const STRIP_TOP: f32 = 8.0;
const STRIP_H: f32 = 24.0;
const DOT: f32 = 5.0;
const DOT_GAP: f32 = 6.0;
const DOT_BOTTOM: f32 = 11.0;
/// Clear space between the content and the page dots.
const ABOVE_DOTS: f32 = 6.0;
const HEADER_H: f32 = 22.0;
const HEADER_GAP: f32 = 6.0;

const NAV_W: f32 = 22.0;
const NAV_GAP: f32 = 2.0;
/// Between the title and its arrows.
const TITLE_NAV_GAP: f32 = 8.0;
/// Between the arrows and the view selector, at the least.
const NAV_SEG_GAP: f32 = 12.0;
const VIEW_BUTTON_W: f32 = 50.0;
/// The view selector's track reaches this far around its buttons.
pub const SEG_PAD: f32 = 2.0;
/// The "Offline" tag and its gap after the arrows.
const OFFLINE_W: f32 = 52.0;
const OFFLINE_GAP: f32 = 8.0;

const CELL_W: f32 = 36.0;
/// Month rows are at most this far apart; less when the strip takes room.
const CELL_PITCH_MAX: f32 = 32.0;
const CELL_GAP_Y: f32 = 2.0;
const LABEL_H: f32 = 16.0;
/// Each side of the line between the month grid and its list.
const LIST_GAP: f32 = 16.0;
const CAPTION_H: f32 = 18.0;
const CAPTION_GAP: f32 = 6.0;
const ROW_H: f32 = 38.0;
const MORE_H: f32 = 20.0;
const MORE_W: f32 = 90.0;
const ADD_W: f32 = 130.0;
const ADD_H: f32 = 22.0;
const ADD_GAP: f32 = 4.0;
const CONNECT_W: f32 = 180.0;
const CONNECT_H: f32 = 26.0;
const CONNECT_GAP: f32 = 6.0;

const WEEK_GAP: f32 = 6.0;
/// Height of the clickable day name and number at the top of a week column.
pub const WEEK_HEAD_H: f32 = 36.0;
/// Event chips sit this far inside their column.
const CHIP_PAD: f32 = 5.0;
/// A timed event's chip (title over time) and an all-day band.
const CHIP_H: f32 = 30.0;
const BAND_H: f32 = 20.0;
const CHIP_GAP: f32 = 4.0;
const WEEK_MORE_H: f32 = 16.0;
const WEEK_ADD_GAP: f32 = 6.0;

/// The day view's all-day bands: per page, and how wide one gets.
const ALL_DAY_H: f32 = 22.0;
const ALL_DAY_PER_PAGE: usize = 2;
const ALL_DAY_MAX_W: f32 = 300.0;
const ALL_DAY_GAP: f32 = 6.0;
/// One hour of the day view's timeline.
const HOUR_H: f32 = 22.0;
/// Room left of the timeline for "09.00".
pub const HOUR_GUTTER: f32 = 40.0;
/// Half an hour label's height: kept clear above the first line.
const HOUR_LABEL_HALF: f32 = 6.0;
const BLOCK_INSET: f32 = 8.0;
/// Blocks end this short of the right edge, as in the design.
const BLOCK_RIGHT: f32 = 100.0;
const LANE_GAP: f32 = 4.0;
/// Overlapping blocks never get narrower than this; the ones that would
/// wait for a later page.
const MIN_LANE_W: f32 = 44.0;
const MIN_BLOCK_H: f32 = 18.0;
/// The first hour shown on a day without timed events.
const DAY_START_H: u32 = 8;
const MIN_HOURS: u32 = 2;

/// Big enough that the widest countdown (Fraunces It 48) clears the stroke;
/// the design's r = 100 does not fit under the waiting strip.
const RING_RADIUS: f32 = 92.0;
/// The ring's stroke, centred on `RING_RADIUS`.
pub const RING_STROKE: f32 = 8.0;
/// The countdown's box: one Fraunces It 48 line (59.2 px).
const TIME_H: f32 = 60.0;
/// The digits' ink reaches this far above and below the box's centre
/// (Fraunces It 48 measures cy - 17 to cy + 18).
const TIME_INK_HALF: f32 = 18.0;
/// The countdown sits this far above the ring's centre, the phase under it.
const TIME_LIFT: f32 = 9.0;
const PHASE_GAP: f32 = 8.0;
const PHASE_H: f32 = 16.0;
/// Between the ring and the column of the session, buttons and next phase.
const RING_GAP: f32 = 36.0;
const COLUMN_W: f32 = 230.0;
const SESSION_H: f32 = 20.0;
const COLUMN_GAP: f32 = 18.0;
const PRIMARY_D: f32 = 62.0;
const SECONDARY_D: f32 = 44.0;
const CONTROL_GAP: f32 = 14.0;
const NEXT_H: f32 = 18.0;
const DURATIONS_W: f32 = 160.0;

const NEW_W: f32 = 64.0;
const LIST_TOP_GAP: f32 = 6.0;
const REMINDER_H: f32 = 29.0;
const GROUP_H: f32 = 18.0;
/// Above every day group but the chunk's first.
const GROUP_GAP: f32 = 6.0;
const NOTE_COLUMNS: usize = 2;
const NOTE_ROWS: usize = 2;
const NOTE_GAP: f32 = 9.0;
const NOTE_MAX_H: f32 = 92.0;
const LIST_MORE_GAP: f32 = 4.0;
const LIST_MORE_W: f32 = 120.0;

const CLI_STATUS_W: f32 = 160.0;
const CLI_LINE_H: f32 = 20.0;
const CLI_PAD: f32 = 14.0;
const CLI_BUTTON_W: f32 = 150.0;
const CLI_BUTTON_H: f32 = 30.0;
const CLI_GAP: f32 = 12.0;

const TILE_GAP: f32 = 10.0;
/// Tiles stay this short when the panel has room to spare.
const TILE_MAX_H: f32 = 98.0;
const TILE_GLYPH_H: f32 = 28.0;
const TILE_LABEL_H: f32 = 16.0;
const TILE_INNER_GAP: f32 = 8.0;

const COVER: f32 = 196.0;
const COVER_GAP: f32 = 24.0;
const MEDIA_LABEL_H: f32 = 14.0;
const MEDIA_TITLE_H: f32 = 38.0;
const MEDIA_ARTIST_H: f32 = 18.0;
const MEDIA_CONTROLS_GAP: f32 = 26.0;
const MEDIA_PLAY: f32 = 48.0;
const MEDIA_SIDE_W: f32 = 32.0;
const MEDIA_BUTTON_GAP: f32 = 12.0;
const LYRICS_W: f32 = 66.0;
const LYRICS_H: f32 = 26.0;

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
pub fn page_dots(s: ShapeRect, page: Page) -> ([Rect; Page::ALL.len()], usize) {
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

/// Which `shown` items of `n` to list when `k` fit: chunk `page` (wrapping)
/// once they do not all fit. Returns `(start, shown)`.
fn chunk(n: usize, k: usize, page: usize) -> (usize, usize) {
    if n <= k {
        (0, n)
    } else if k == 0 {
        (0, 0)
    } else {
        let start = (page % n.div_ceil(k)) * k;
        (start, k.min(n - start))
    }
}

/// Event rows stacked from `y` down to `bottom`: optional connect button,
/// rows, "+N lagi" if they do not all fit, then "+ Tambah acara".
struct Stack {
    connect: Option<Rect>,
    rows: Vec<(String, Rect)>,
    more: Option<(usize, Rect)>,
    add: Rect,
}

/// `page` picks the chunk of rows shown when they do not all fit.
fn stack(
    x: f32,
    w: f32,
    mut y: f32,
    bottom: f32,
    ids: Vec<String>,
    page: usize,
    connect: bool,
) -> Stack {
    let connect = if connect {
        let r = rect(x, y, w.min(CONNECT_W), CONNECT_H);
        y = r.bottom() + CONNECT_GAP;
        Some(r)
    } else {
        None
    };
    let avail = bottom - y - ADD_GAP - ADD_H;
    let total = ids.len();
    let k = rows_that_fit(total, avail, ROW_H, 0.0, MORE_H);
    let (start, shown) = chunk(total, k, page);
    let rows: Vec<(String, Rect)> = ids
        .into_iter()
        .skip(start)
        .take(shown)
        .enumerate()
        .map(|(i, id)| (id, rect(x, y + i as f32 * ROW_H, w, ROW_H)))
        .collect();
    y += shown as f32 * ROW_H;
    let more = if shown < total {
        let r = rect(x, y, w.min(MORE_W), MORE_H);
        y = r.bottom();
        Some((total - start - shown, r))
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

/// An event's minutes on `day`, start and end clipped to the day; one with
/// no length counts as half an hour.
fn minutes_on(ev: &CalEvent, day: NaiveDate) -> (i64, i64) {
    let day_start = day.and_time(NaiveTime::MIN);
    let minute = |t: &EventTime| {
        let at = match t {
            EventTime::AllDay(d) => d.and_time(NaiveTime::MIN),
            EventTime::At(t) => t.naive_local(),
        };
        (at - day_start).num_minutes().clamp(0, 24 * 60)
    };
    let start = minute(&ev.start).min(24 * 60 - 1);
    let end = minute(&ev.end);
    (
        start,
        if end > start {
            end
        } else {
            (start + 30).min(24 * 60)
        },
    )
}

/// Month: grid on the left, the selected day's events on the right. Week:
/// seven columns of event chips. Day: all-day bands over an hour timeline;
/// `day_page` (wrapping) pages through what does not fit. `events` is the
/// whole snapshot; each day's share comes from `events_on`. `title_w` is the
/// drawn title's width (the arrows follow it); `now` the local time, which
/// the day view of today keeps in sight and marks.
#[allow(clippy::too_many_arguments)]
pub fn calendar_layout(
    s: ShapeRect,
    view: CalView,
    anchor: NaiveDate,
    selected: NaiveDate,
    events: &[CalEvent],
    status: &CalStatus,
    day_page: usize,
    strip: bool,
    title_w: f32,
    now: Option<NaiveDateTime>,
) -> CalendarLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));

    // Header row: the title and its arrows on the left, the view selector
    // on the right.
    let seg_w = 3.0 * VIEW_BUTTON_W;
    let seg_x = r - SEG_PAD - seg_w;
    let views = [CalView::Day, CalView::Week, CalView::Month];
    let view_buttons = std::array::from_fn(|i| {
        let x = seg_x + i as f32 * VIEW_BUTTON_W;
        (views[i], rect(x, top, VIEW_BUTTON_W, HEADER_H))
    });
    let nav_w = 2.0 * NAV_W + NAV_GAP;
    let offline_on = *status == CalStatus::Offline;
    // Offline, the title gives up room for the tag rather than the tag hiding.
    let tag_w = if offline_on {
        OFFLINE_GAP + OFFLINE_W
    } else {
        0.0
    };
    let title_max = seg_x - SEG_PAD - NAV_SEG_GAP - tag_w - nav_w - TITLE_NAV_GAP - l;
    let header = rect(l, top, title_w.clamp(0.0, title_max.max(0.0)), HEADER_H);
    let prev = rect(header.right() + TITLE_NAV_GAP, top, NAV_W, HEADER_H);
    let next = rect(prev.right() + NAV_GAP, top, NAV_W, HEADER_H);
    let offline = offline_on.then(|| {
        rect(
            next.right() + OFFLINE_GAP,
            top + 3.0,
            OFFLINE_W,
            HEADER_H - 6.0,
        )
    });
    let body = header.bottom() + HEADER_GAP;

    let ids_on = |d: NaiveDate| -> Vec<String> {
        events_on(events, d)
            .into_iter()
            .map(|e| e.id.clone())
            .collect()
    };
    // Not configured also offers the button: clicking it tells the owner to
    // fill in the client ID, instead of an empty calendar saying nothing.
    let wants_connect = matches!(
        status,
        CalStatus::NotConfigured | CalStatus::NeedsLogin | CalStatus::Reconnect
    );
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
        hours: Vec::new(),
        now_y: None,
        add: Rect::default(),
        connect: None,
        offline,
    };

    match view {
        CalView::Month => {
            let col_x = |c: usize| l + c as f32 * CELL_W;
            layout.weekday_labels = (0..7)
                .map(|c| rect(col_x(c), body, CELL_W, LABEL_H))
                .collect();
            let cells_y = body + LABEL_H + CELL_GAP_Y;
            let pitch = ((bottom - cells_y + CELL_GAP_Y) / 6.0)
                .floor()
                .min(CELL_PITCH_MAX);
            let (first, _) = range_for(CalView::Month, anchor);
            layout.month_cells = (0..42usize)
                .filter_map(|i| {
                    let d = first.checked_add_days(Days::new(i as u64))?;
                    let y = cells_y + (i / 7) as f32 * pitch;
                    Some((d, rect(col_x(i % 7), y, CELL_W, pitch - CELL_GAP_Y)))
                })
                .collect();
            let list_x = col_x(7) + 2.0 * LIST_GAP;
            let list = rect(list_x, body, r - list_x, bottom - body);
            let caption = rect(list.x, list.y, list.w, CAPTION_H);
            let st = stack(
                list.x,
                list.w,
                caption.bottom() + CAPTION_GAP,
                bottom,
                ids_on(selected),
                0,
                wants_connect,
            );
            layout.list = list;
            layout.caption = Some(caption);
            layout.rows = st.rows;
            layout.more = st.more;
            layout.add = st.add;
            layout.connect = st.connect;
        }
        CalView::Day => day_timeline(
            &mut layout,
            (l, r, body, bottom),
            events,
            day_page,
            wants_connect,
            now,
        ),
        CalView::Week => {
            let (monday, _) = range_for(CalView::Week, anchor);
            let col_w = (r - l - 6.0 * WEEK_GAP) / 7.0;
            let col_h = bottom - body - WEEK_ADD_GAP - ADD_H;
            for i in 0..7usize {
                let Some(d) = monday.checked_add_days(Days::new(i as u64)) else {
                    break;
                };
                let x = l + i as f32 * (col_w + WEEK_GAP);
                let column = rect(x, body, col_w, col_h);
                layout.week_columns.push((d, column));
                let on = events_on(events, d);
                let heights: Vec<f32> = on
                    .iter()
                    .map(|e| if e.all_day() { BAND_H } else { CHIP_H })
                    .collect();
                let chips_top = body + WEEK_HEAD_H;
                let avail = column.bottom() - CHIP_PAD - chips_top;
                let all: f32 = heights.iter().map(|h| h + CHIP_GAP).sum::<f32>() - CHIP_GAP;
                let room = if all <= avail {
                    avail
                } else {
                    avail - WEEK_MORE_H - CHIP_GAP
                };
                let mut y = chips_top;
                let mut shown = 0;
                for (ev, h) in on.iter().zip(&heights) {
                    if y + h > chips_top + room {
                        break;
                    }
                    let chip = rect(x + CHIP_PAD, y, col_w - 2.0 * CHIP_PAD, *h);
                    layout.rows.push((ev.id.clone(), chip));
                    y += h + CHIP_GAP;
                    shown += 1;
                }
                if shown < on.len() {
                    let foot = column.bottom() - CHIP_PAD - WEEK_MORE_H;
                    let more = rect(x + CHIP_PAD, foot, col_w - 2.0 * CHIP_PAD, WEEK_MORE_H);
                    layout.week_more.push((d, on.len() - shown, more));
                }
            }
            layout.list = rect(l, body, r - l, col_h);
            layout.add = rect(l, layout.list.bottom() + WEEK_ADD_GAP, ADD_W, ADD_H);
            // As tall as in the other views, centered on the "+ Tambah acara" row.
            let y = layout.add.y + (ADD_H - CONNECT_H) / 2.0;
            layout.connect =
                wants_connect.then(|| rect(layout.add.right() + 12.0, y, CONNECT_W, CONNECT_H));
        }
    }
    layout
}

/// The day view: an optional connect button, the all-day bands, then an
/// hour timeline with the timed events as blocks (side by side where they
/// overlap), and "+ Tambah acara" with "+N lagi" at the foot.
///
/// Pages: the first shows the hours from just before the first event (or
/// around `now` on today); each further one the hours of the earliest event
/// not seen yet, and the next two all-day bands when there are more than
/// two. Blocks that overlap share the width in lanes no narrower than
/// `MIN_LANE_W`; any that do not fit wait for the next page. "+N lagi"
/// counts what the following pages add.
fn day_timeline(
    layout: &mut CalendarLayout,
    (l, r, body, bottom): (f32, f32, f32, f32),
    events: &[CalEvent],
    day_page: usize,
    wants_connect: bool,
    now: Option<NaiveDateTime>,
) {
    let day = layout.selected;
    let mut y = body;
    if wants_connect {
        let c = rect(l, y, (r - l).min(CONNECT_W), CONNECT_H);
        y = c.bottom() + CONNECT_GAP;
        layout.connect = Some(c);
    }
    let on = events_on(events, day);
    let (all_day, timed): (Vec<&CalEvent>, Vec<&CalEvent>) =
        on.into_iter().partition(|e| e.all_day());
    let band_y = y;
    if !all_day.is_empty() {
        y += ALL_DAY_H + ALL_DAY_GAP;
    }
    let foot = bottom - ADD_H;
    layout.add = rect(l, foot, ADD_W, ADD_H);
    let tl_top = y + HOUR_LABEL_HALF;
    let tl_bottom = foot - ADD_GAP - HOUR_LABEL_HALF;
    let span = (((tl_bottom - tl_top) / HOUR_H).floor().max(0.0) as u32).clamp(MIN_HOURS, 24);

    let minutes: Vec<(i64, i64)> = timed.iter().map(|e| minutes_on(e, day)).collect();
    let in_window =
        |h: u32, (s, e): (i64, i64)| s < i64::from(h + span) * 60 && e > i64::from(h) * 60;
    let mut first = minutes
        .first()
        .map_or(DAY_START_H, |(s, _)| ((*s / 60) as u32).saturating_sub(1));
    let today_now = now.filter(|n| n.date() == day);
    if let Some(n) = today_now
        && !(first..first + span).contains(&n.hour())
    {
        first = n.hour().saturating_sub(span / 2);
    }
    let tl_end = tl_top + span as f32 * HOUR_H;
    let area_x = l + HOUR_GUTTER + BLOCK_INSET;
    let area_w = r - BLOCK_RIGHT - area_x;
    let max_lanes = (((area_w + LANE_GAP) / (MIN_LANE_W + LANE_GAP)).floor() as usize).max(1);
    // An event's block (top, height) in the hours from `window`.
    let block = |window: u32, (s, e): (i64, i64)| {
        let y_of = |minute: i64| tl_top + (minute - i64::from(window) * 60) as f32 / 60.0 * HOUR_H;
        let (y0, y1) = (y_of(s).max(tl_top), y_of(e).min(tl_end));
        let h = (y1 - y0 - 2.0).max(MIN_BLOCK_H);
        ((y0 + 1.0).min(tl_end - h), h)
    };
    // A placed block: index in `timed`, top, height, lane.
    type Block = (usize, f32, f32, usize);
    // The blocks of one window: each in the first lane free over its time,
    // events not shown yet first. Past `max_lanes` a block waits for a
    // later page.
    let place = |window: u32, seen: &[bool]| {
        let mut order: Vec<usize> = (0..minutes.len())
            .filter(|&i| in_window(window, minutes[i]))
            .collect();
        order.sort_by_key(|&i| (seen[i], minutes[i].0, i));
        let mut lanes: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut placed: Vec<Block> = Vec::new();
        for i in order {
            let (top, h) = block(window, minutes[i]);
            let (a, b) = (top, top + h + 1.0);
            let free = |lane: &Vec<(f32, f32)>| lane.iter().all(|(s, e)| b <= *s || *e <= a);
            let lane = match lanes.iter().position(free) {
                Some(k) => k,
                None if lanes.len() < max_lanes => {
                    lanes.push(Vec::new());
                    lanes.len() - 1
                }
                None => continue,
            };
            lanes[lane].push((a, b));
            placed.push((i, top, h, lane));
        }
        placed
    };
    // Pages of the timeline: each further one starts at the hour of the
    // earliest event not shown yet, which always gets a lane there.
    let mut seen = vec![false; minutes.len()];
    let mut windows: Vec<(u32, Vec<Block>)> = Vec::new();
    let mut window = first.min(24 - span);
    loop {
        let placed = place(window, &seen);
        for (i, ..) in &placed {
            seen[*i] = true;
        }
        windows.push((window, placed));
        let next = (0..minutes.len())
            .filter(|&i| !seen[i])
            .min_by_key(|&i| (minutes[i].0, i));
        let Some(i) = next else { break };
        window = ((minutes[i].0 / 60) as u32).min(24 - span);
    }
    let band_pages = if all_day.len() <= ALL_DAY_PER_PAGE {
        1
    } else {
        all_day.len().div_ceil(ALL_DAY_PER_PAGE)
    };
    let pages = windows.len().max(band_pages);
    let page = day_page % pages;
    let (window, placed) = &windows[page.min(windows.len() - 1)];
    let window = *window;

    // All-day bands of this page.
    let bands: &[&CalEvent] = if all_day.len() <= ALL_DAY_PER_PAGE {
        &all_day
    } else if page < band_pages {
        let start = page * ALL_DAY_PER_PAGE;
        &all_day[start..(start + ALL_DAY_PER_PAGE).min(all_day.len())]
    } else {
        &[]
    };
    if !bands.is_empty() {
        let k = bands.len() as f32;
        let w = ((r - l - (k - 1.0) * ALL_DAY_GAP) / k).min(ALL_DAY_MAX_W);
        for (i, ev) in bands.iter().enumerate() {
            let x = l + i as f32 * (w + ALL_DAY_GAP);
            layout
                .rows
                .push((ev.id.clone(), rect(x, band_y, w, ALL_DAY_H)));
        }
    }

    // The timeline and its blocks.
    let y_of = |minute: i64| tl_top + (minute - i64::from(window) * 60) as f32 / 60.0 * HOUR_H;
    layout.list = rect(l, tl_top, r - l, tl_end - tl_top);
    layout.hours = (0..=span)
        .map(|k| (window + k, tl_top + k as f32 * HOUR_H))
        .collect();
    layout.now_y = today_now.and_then(|n| {
        let minute = i64::from(n.hour() * 60 + n.minute());
        let y = y_of(minute);
        (tl_top..=tl_end).contains(&y).then_some(y)
    });
    // Lanes split the width only within a run of overlapping blocks.
    let mut by_top: Vec<&Block> = placed.iter().collect();
    by_top.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut start = 0;
    while start < by_top.len() {
        let mut end_y = by_top[start].1 + by_top[start].2 + 1.0;
        let mut stop = start + 1;
        while stop < by_top.len() && by_top[stop].1 < end_y {
            end_y = end_y.max(by_top[stop].1 + by_top[stop].2 + 1.0);
            stop += 1;
        }
        let cluster = &by_top[start..stop];
        // The run's lanes renumbered 0, 1, 2…, so a lane no block of the run
        // uses never takes width (blocks are placed unseen-first, not in time
        // order).
        let mut used: Vec<usize> = cluster.iter().map(|b| b.3).collect();
        used.sort_unstable();
        used.dedup();
        let n = used.len().max(1) as f32;
        let lane_w = ((area_w - (n - 1.0) * LANE_GAP) / n).max(0.0);
        for (i, top, h, lane) in cluster {
            let rank = used.binary_search(lane).unwrap_or(0);
            let x = area_x + rank as f32 * (lane_w + LANE_GAP);
            layout
                .rows
                .push((timed[*i].id.clone(), rect(x, *top, lane_w, *h)));
        }
        start = stop;
    }

    // "+N lagi": what the next pages add; "Kembali ke atas" on the last.
    if pages > 1 {
        let shown_bands = if all_day.len() <= ALL_DAY_PER_PAGE {
            all_day.len()
        } else {
            ((page + 1) * ALL_DAY_PER_PAGE).min(all_day.len())
        };
        let mut shown = vec![false; minutes.len()];
        for (_, placed) in &windows[..=page.min(windows.len() - 1)] {
            for (i, ..) in placed {
                shown[*i] = true;
            }
        }
        let shown_timed = shown.iter().filter(|s| **s).count();
        let later = all_day.len() + timed.len() - shown_bands - shown_timed;
        let count = if page + 1 == pages { 0 } else { later };
        let more = rect(layout.add.right() + 12.0, foot, MORE_W, ADD_H);
        layout.more = Some((count, more));
    }
}

/// Big ring on the left with the countdown and the phase inside; right of
/// it a column with the session, the three round buttons (the primary one
/// in the middle) and the phase that comes next. Ring and column together
/// are centred in the panel.
pub fn pomodoro_layout(s: ShapeRect, strip: bool) -> PomodoroLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));
    let durations = rect(r - DURATIONS_W, top, DURATIONS_W, HEADER_H);
    let header = rect(l, top, durations.x - 8.0 - l, HEADER_H);
    let body = header.bottom() + 8.0;
    let cy = body + (bottom - body) / 2.0;
    let outer = RING_RADIUS + RING_STROKE / 2.0;
    let group_w = 2.0 * outer + RING_GAP + COLUMN_W;
    let group_x = l + ((r - l) - group_w) / 2.0;
    let cx = group_x + outer;
    let inner = RING_RADIUS - RING_STROKE / 2.0;
    // Widest where the inner edge allows at the farther of the ink's edges.
    let half_at = |dy: f32| (inner * inner - dy * dy).max(0.0).sqrt();
    let time_cy = cy - TIME_LIFT;
    let time_half = half_at(TIME_LIFT + TIME_INK_HALF);
    let time = rect(
        cx - time_half,
        time_cy - TIME_H / 2.0,
        2.0 * time_half,
        TIME_H,
    );
    let phase_y = time_cy + TIME_INK_HALF + PHASE_GAP;
    let phase_half = half_at(phase_y + PHASE_H - cy);
    let phase = rect(cx - phase_half, phase_y, 2.0 * phase_half, PHASE_H);

    let x = group_x + 2.0 * outer + RING_GAP;
    let column_h = SESSION_H + COLUMN_GAP + PRIMARY_D + COLUMN_GAP + NEXT_H;
    let session = rect(x, cy - column_h / 2.0, COLUMN_W, SESSION_H);
    let row_y = session.bottom() + COLUMN_GAP;
    let side_y = row_y + (PRIMARY_D - SECONDARY_D) / 2.0;
    let reset = rect(x, side_y, SECONDARY_D, SECONDARY_D);
    let start_pause = rect(reset.right() + CONTROL_GAP, row_y, PRIMARY_D, PRIMARY_D);
    let skip = rect(
        start_pause.right() + CONTROL_GAP,
        side_y,
        SECONDARY_D,
        SECONDARY_D,
    );
    let next_up = rect(x, start_pause.bottom() + COLUMN_GAP, COLUMN_W, NEXT_H);
    PomodoroLayout {
        header,
        durations,
        ring_center: (cx, cy),
        ring_radius: RING_RADIUS,
        time,
        phase,
        session,
        start_pause,
        reset,
        skip,
        next_up,
    }
}

/// The title and "+ Baru" of the notes and reminders pages, and the y the
/// items start at.
fn list_header(s: ShapeRect, strip: bool) -> (Rect, Rect, f32) {
    let top = content_top(s, strip);
    let (l, r) = (left(s), right(s));
    let header = rect(l, top, r - l - NEW_W - 8.0, HEADER_H);
    let new_button = rect(r - NEW_W, top, NEW_W, HEADER_H);
    (header, new_button, header.bottom() + LIST_TOP_GAP)
}

/// Cards in two columns, newest top left, two rows of them. When there are
/// more, the page shows chunk `notes_page` (wrapping) and a "+N lagi" /
/// "Kembali ke atas" line under the cards.
pub fn notes_layout(s: ShapeRect, notes: &[Note], notes_page: usize, strip: bool) -> ListLayout {
    let (header, new_button, list_y) = list_header(s, strip);
    let (l, r) = (left(s), right(s));
    let bottom = content_bottom(s);
    let per_page = NOTE_COLUMNS * NOTE_ROWS;
    let rows_f = NOTE_ROWS as f32;
    let card_h = ((bottom - list_y - LIST_MORE_GAP - MORE_H - (rows_f - 1.0) * NOTE_GAP) / rows_f)
        .min(NOTE_MAX_H);
    let card_w = (r - l - (NOTE_COLUMNS as f32 - 1.0) * NOTE_GAP) / NOTE_COLUMNS as f32;
    let n = notes.len();
    let (start, shown) = chunk(n, per_page, notes_page);
    let rows = notes
        .iter()
        .skip(start)
        .take(shown)
        .enumerate()
        .map(|(i, note)| {
            let (row, col) = ((i / NOTE_COLUMNS) as f32, (i % NOTE_COLUMNS) as f32);
            let x = l + col * (card_w + NOTE_GAP);
            let y = list_y + row * (card_h + NOTE_GAP);
            (note.id.clone(), rect(x, y, card_w, card_h))
        })
        .collect();
    let more = (n > per_page).then(|| {
        let y = list_y + rows_f * card_h + (rows_f - 1.0) * NOTE_GAP + LIST_MORE_GAP;
        (n - start - shown, rect(l, y, LIST_MORE_W, MORE_H))
    });
    ListLayout {
        header,
        new_button,
        groups: Vec::new(),
        rows,
        more,
    }
}

/// The upcoming reminders in rows grouped by day, each group under its
/// heading: overdue ones join today's group. When they do not all fit, the
/// page shows chunk `page` (wrapping), each chunk starting with its group's
/// heading, and a "+N lagi" / "Kembali ke atas" line.
pub fn reminders_layout(
    s: ShapeRect,
    reminders: &[Reminder],
    page: usize,
    strip: bool,
    today: NaiveDate,
) -> ListLayout {
    let (header, new_button, list_y) = list_header(s, strip);
    let (l, r) = (left(s), right(s));
    let bottom = content_bottom(s);
    let days: Vec<NaiveDate> = reminders
        .iter()
        .map(|x| {
            local_time(x.due_unix)
                .map_or(today, |t| t.date())
                .max(today)
        })
        .collect();
    // Height of items `start..end` from the top of a chunk.
    let height = |start: usize, end: usize| -> f32 {
        (start..end)
            .map(|i| {
                let heading = if i == start {
                    GROUP_H
                } else if days[i] != days[i - 1] {
                    GROUP_GAP + GROUP_H
                } else {
                    0.0
                };
                heading + REMINDER_H
            })
            .sum()
    };
    let n = days.len();
    let mut chunks = Vec::new();
    if height(0, n) <= bottom - list_y {
        chunks.push((0, n));
    } else {
        let room = bottom - list_y - LIST_MORE_GAP - MORE_H;
        let mut start = 0;
        while start < n {
            let mut end = start + 1;
            while end < n && height(start, end + 1) <= room {
                end += 1;
            }
            chunks.push((start, end));
            start = end;
        }
    }
    let mut groups = Vec::new();
    let mut rows = Vec::new();
    let mut y = list_y;
    let mut more = None;
    if let Some(&(start, end)) = chunks.get(page % chunks.len().max(1)) {
        for i in start..end {
            if i == start || days[i] != days[i - 1] {
                if i != start {
                    y += GROUP_GAP;
                }
                groups.push((days[i], rect(l, y, r - l, GROUP_H)));
                y += GROUP_H;
            }
            rows.push((reminders[i].id.clone(), rect(l, y, r - l, REMINDER_H)));
            y += REMINDER_H;
        }
        if chunks.len() > 1 {
            more = Some((n - end, rect(l, y + LIST_MORE_GAP, LIST_MORE_W, MORE_H)));
        }
    }
    ListLayout {
        header,
        new_button,
        groups,
        rows,
        more,
    }
}

/// The big cover on the left; beside it, centred on it, "SEDANG DIPUTAR",
/// the title over the artist, and the transport row (prev, the round
/// play/pause, next) with "Lirik" at its right end.
pub fn media_panel_layout(s: ShapeRect, strip: bool) -> ExpandedLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));
    let side = COVER.min(bottom - top);
    let art = rect(l, top + (bottom - top - side) / 2.0, side, side);
    let x = art.right() + COVER_GAP;
    let w = r - x;
    let block_h = MEDIA_LABEL_H
        + 6.0
        + MEDIA_TITLE_H
        + 2.0
        + MEDIA_ARTIST_H
        + MEDIA_CONTROLS_GAP
        + MEDIA_PLAY;
    let label = rect(x, art.y + (side - block_h) / 2.0, w, MEDIA_LABEL_H);
    let title = rect(x, label.bottom() + 6.0, w, MEDIA_TITLE_H);
    let artist = rect(x, title.bottom() + 2.0, w, MEDIA_ARTIST_H);
    let row_y = artist.bottom() + MEDIA_CONTROLS_GAP;
    let prev = rect(x, row_y, MEDIA_SIDE_W, MEDIA_PLAY);
    let play = rect(
        prev.right() + MEDIA_BUTTON_GAP,
        row_y,
        MEDIA_PLAY,
        MEDIA_PLAY,
    );
    let next = rect(
        play.right() + MEDIA_BUTTON_GAP,
        row_y,
        MEDIA_SIDE_W,
        MEDIA_PLAY,
    );
    let lyrics = rect(
        r - LYRICS_W,
        row_y + (MEDIA_PLAY - LYRICS_H) / 2.0,
        LYRICS_W,
        LYRICS_H,
    );
    ExpandedLayout {
        art,
        label,
        title,
        artist,
        buttons: [prev, play, next],
        lyrics,
    }
}

/// Header with the status on its right; under it the card (prompt line and
/// output lines from its top) and the button at its right edge, the two
/// centred in the space left.
pub fn cli_layout(s: ShapeRect, strip: bool) -> CliLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));
    let header = rect(l, top, r - l - CLI_STATUS_W - 8.0, HEADER_H);
    let status = rect(r - CLI_STATUS_W, top, CLI_STATUS_W, HEADER_H);
    let body = header.bottom() + 8.0;
    let card_h = 2.0 * CLI_PAD + (1 + CARD_LINES) as f32 * CLI_LINE_H;
    let group_h = card_h + CLI_GAP + CLI_BUTTON_H;
    let y = body + ((bottom - body - group_h) / 2.0).max(0.0);
    let card = rect(l, y, r - l, card_h);
    let lines = std::array::from_fn(|i| {
        rect(
            card.x + CLI_PAD,
            card.y + CLI_PAD + i as f32 * CLI_LINE_H,
            card.w - 2.0 * CLI_PAD,
            CLI_LINE_H,
        )
    });
    let button = rect(
        r - CLI_BUTTON_W,
        card.bottom() + CLI_GAP,
        CLI_BUTTON_W,
        CLI_BUTTON_H,
    );
    CliLayout {
        header,
        status,
        card,
        lines,
        button,
    }
}

/// Four equal tiles per row, two rows, centred under the header.
pub fn shortcuts_layout(s: ShapeRect, strip: bool) -> ShortcutsLayout {
    let top = content_top(s, strip);
    let bottom = content_bottom(s);
    let (l, r) = (left(s), right(s));
    let header = rect(l, top, r - l, HEADER_H);
    let body = header.bottom() + 8.0;
    let w = (r - l - 3.0 * TILE_GAP) / 4.0;
    let h = ((bottom - body - TILE_GAP) / 2.0).min(TILE_MAX_H);
    let y0 = body + ((bottom - body) - (2.0 * h + TILE_GAP)) / 2.0;
    let tiles = std::array::from_fn(|i| {
        let (row, col) = ((i / 4) as f32, (i % 4) as f32);
        let x = l + col * (w + TILE_GAP);
        (Shortcut::ALL[i], rect(x, y0 + row * (h + TILE_GAP), w, h))
    });
    ShortcutsLayout { header, tiles }
}

/// A tile's icon and, under it, its label; the two centred in the tile.
pub fn tile_parts(tile: Rect) -> (Rect, Rect) {
    let h = TILE_GLYPH_H + TILE_INNER_GAP + TILE_LABEL_H;
    let y = tile.y + (tile.h - h) / 2.0;
    let glyph = rect(tile.x, y, tile.w, TILE_GLYPH_H);
    let label = rect(
        tile.x + 4.0,
        glyph.bottom() + TILE_INNER_GAP,
        tile.w - 8.0,
        TILE_LABEL_H,
    );
    (glyph, label)
}

/// Small ring on the left (center, radius 8), then the time and label.
pub fn pomodoro_compact_layout(s: ShapeRect) -> ((f32, f32), f32, Rect) {
    const RADIUS: f32 = 8.0;
    let center = (s.x + 22.0, s.y + s.h / 2.0);
    let text_x = center.0 + RADIUS + 10.0;
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
            // Month: opens that day. Day: shows the day's next chunk of events.
            if let Some((_, r)) = l.more {
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
            add(l.start_pause, Hit::PomoStartPause);
            add(l.reset, Hit::PomoReset);
            add(l.skip, Hit::PomoSkip);
        }
        PageLayout::Reminders(l) => {
            add(l.new_button, Hit::ReminderNew);
            for (id, r) in &l.rows {
                add(*r, Hit::ReminderOpen(id.clone()));
            }
            if let Some((_, r)) = l.more {
                add(r, Hit::RemindersMore);
            }
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
            add(l.lyrics, Hit::LyricsToggle);
        }
        PageLayout::Cli(l) => {
            add(l.card, Hit::CliOpen);
            add(l.button, Hit::CliOpen);
        }
        PageLayout::Shortcuts(l) => {
            for (s, r) in l.tiles {
                add(r, Hit::Shortcut(s));
            }
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

    fn at(d: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
        d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    fn timed(id: &str, start: NaiveDateTime, end: NaiveDateTime) -> CalEvent {
        use chrono::{Local, TimeZone};
        let local = |t| EventTime::At(Local.from_local_datetime(&t).earliest().unwrap());
        CalEvent {
            start: local(start),
            end: local(end),
            ..all_day(id, start.date())
        }
    }

    /// The title width the tests assume ("Oktober 2026" in Fraunces It 20).
    const TITLE_W: f32 = 130.0;

    /// The layout without a title width or a clock: tests that only page
    /// or count need neither.
    #[allow(clippy::too_many_arguments)]
    fn calendar_layout(
        s: ShapeRect,
        view: CalView,
        anchor: NaiveDate,
        selected: NaiveDate,
        events: &[CalEvent],
        status: &CalStatus,
        day_page: usize,
        strip: bool,
    ) -> CalendarLayout {
        super::calendar_layout(
            s, view, anchor, selected, events, status, day_page, strip, TITLE_W, None,
        )
    }

    fn reminders_layout(s: ShapeRect, r: &[Reminder], page: usize, strip: bool) -> ListLayout {
        super::reminders_layout(s, r, page, strip, oct25())
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

    fn reminders(n: usize) -> Vec<Reminder> {
        (0..n)
            .map(|i| Reminder {
                id: format!("r{i}"),
                text: format!("reminder {i}"),
                due_unix: 1000 + i as i64 * 60,
                repeat: Default::default(),
                done: false,
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
        // A busy timed day too: overlapping, short and late events.
        let d = date(2026, 10, 26);
        for (i, (h0, m0, h1, m1)) in [
            (9, 0, 10, 30),
            (9, 30, 11, 0),
            (10, 0, 10, 10),
            (13, 0, 14, 0),
            (13, 0, 13, 5),
            (21, 0, 23, 30),
            (23, 50, 23, 50),
        ]
        .into_iter()
        .enumerate()
        {
            events.push(timed(&format!("t{i}"), at(d, h0, m0), at(d, h1, m1)));
        }
        let mut out = Vec::new();
        for page in [0, 1, 2, 3] {
            for now in [None, Some(at(d, 13, 10)), Some(at(d, 23, 59))] {
                let l = super::calendar_layout(
                    s,
                    CalView::Day,
                    d,
                    d,
                    &events,
                    &CalStatus::NeedsLogin,
                    page,
                    strip,
                    600.0,
                    now,
                );
                out.push((
                    format!("day timed p{page} {now:?}"),
                    PageLayout::Calendar(l),
                ));
            }
        }
        for view in [CalView::Day, CalView::Week, CalView::Month] {
            for status in [CalStatus::Ok, CalStatus::Reconnect] {
                for page in [0, 1, 9] {
                    let l =
                        calendar_layout(s, view, oct25(), oct25(), &events, &status, page, strip);
                    out.push((
                        format!("calendar {view:?} {status:?} p{page}"),
                        PageLayout::Calendar(l),
                    ));
                }
            }
        }
        out.push((
            "pomodoro".into(),
            PageLayout::Pomodoro(pomodoro_layout(s, strip)),
        ));
        for (n, page) in [(0, 0), (3, 0), (30, 0), (30, 7)] {
            let l = notes_layout(s, &notes(n), page, strip);
            out.push((format!("notes {n} p{page}"), PageLayout::Notes(l)));
            let l = reminders_layout(s, &reminders(n), page, strip);
            out.push((format!("reminders {n} p{page}"), PageLayout::Reminders(l)));
        }
        out.push((
            "media".into(),
            PageLayout::Media(media_panel_layout(s, strip)),
        ));
        out.push(("cli".into(), PageLayout::Cli(cli_layout(s, strip))));
        out.push((
            "shortcuts".into(),
            PageLayout::Shortcuts(shortcuts_layout(s, strip)),
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
                    assert_eq!((d.w, d.h), (5.0, 5.0));
                    assert_eq!(d.bottom(), s.y + s.h - 11.0);
                }
                for pair in dots.windows(2) {
                    assert_eq!(pair[1].x - pair[0].right(), 6.0);
                }
                let left = dots[0].x - s.x;
                let right = s.x + s.w - dots[dots.len() - 1].right();
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
            0,
            false,
        );
        assert_eq!(l.month_cells.len(), 42);
        assert_eq!(l.month_cells[0].0, date(2026, 9, 28));
        assert_eq!(l.month_cells[41].0, date(2026, 11, 8));
        for pair in l.month_cells.windows(2) {
            assert_eq!(pair[0].0.succ_opt(), Some(pair[1].0));
        }
        let first = l.month_cells[0].1;
        assert_eq!((first.w, first.h), (36.0, 30.0));
        assert_eq!(first.x, s.x + 20.0);
        assert_eq!(l.month_cells[1].1.x, first.right());
        assert_eq!(l.month_cells[7].1.y, first.bottom() + 2.0);
        assert_eq!(l.month_cells[7].1.x, first.x);
        assert!(first.y >= l.header.bottom() + 6.0);
        // Weekday labels sit over the grid columns.
        assert_eq!(l.weekday_labels.len(), 7);
        for (label, (_, cell)) in l.weekday_labels.iter().zip(&l.month_cells) {
            assert_eq!((label.x, label.w), (cell.x, cell.w));
            assert!(label.bottom() <= cell.y);
        }
        // The list sits right of the grid, past the dividing line.
        assert_eq!(l.list.x, l.month_cells[6].1.right() + 32.0);
        // The arrows follow the title; the selector keeps to the right.
        assert_eq!(l.header.w, TITLE_W);
        assert_eq!(l.prev.x, l.header.right() + 8.0);
        assert!(l.next.x >= l.prev.right());
        assert!(l.next.right() + 12.0 <= l.view_buttons[0].1.x);
        assert_eq!(l.view_buttons[2].1.right() + SEG_PAD, s.x + s.w - 20.0);
        // A long title is cut short so nothing overlaps.
        let wide = super::calendar_layout(
            s,
            CalView::Month,
            date(2026, 10, 1),
            date(2026, 10, 25),
            &[],
            &CalStatus::Ok,
            0,
            false,
            900.0,
            None,
        );
        assert!(wide.next.right() + 12.0 <= wide.view_buttons[0].1.x - SEG_PAD + 0.01);
        assert_eq!(wide.offline, None);
        // Offline, a long title is cut shorter still: the tag always shows.
        let offline = super::calendar_layout(
            s,
            CalView::Month,
            date(2026, 10, 1),
            date(2026, 10, 25),
            &[],
            &CalStatus::Offline,
            0,
            false,
            900.0,
            None,
        );
        let tag = offline.offline.expect("offline tag");
        assert!(offline.header.w < wide.header.w);
        assert!(tag.x >= offline.next.right());
        assert!(tag.right() + 12.0 <= offline.view_buttons[0].1.x - SEG_PAD + 0.01);
        assert_eq!(l.list.right(), s.x + s.w - 20.0);
        assert!(l.week_columns.is_empty());
    }

    #[test]
    fn week_columns_monday_to_sunday() {
        let s = panel(Style::Pill);
        let events = many_events(9, date(2026, 10, 21));
        let l = calendar_layout(
            s,
            CalView::Week,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            0,
            false,
        );
        let days: Vec<NaiveDate> = l.week_columns.iter().map(|(d, _)| *d).collect();
        assert_eq!(days.first(), Some(&date(2026, 10, 19)));
        assert_eq!(days.last(), Some(&date(2026, 10, 25)));
        assert_eq!(days.len(), 7);
        let shown = l.rows.len();
        assert!((3..9).contains(&shown), "{shown} chips");
        assert_eq!(l.week_more.len(), 1);
        assert_eq!(l.week_more[0].0, date(2026, 10, 21));
        assert_eq!(l.week_more[0].1, 9 - shown);
        assert!(l.month_cells.is_empty());
        // Chips sit inside their column under its head, the "+N" at its foot.
        let (_, col) = l.week_columns[2];
        for (_, chip) in &l.rows {
            assert!(chip.x > col.x && chip.right() < col.right());
            assert!(chip.y >= col.y + WEEK_HEAD_H && chip.bottom() <= l.week_more[0].2.y);
        }
        assert!(l.week_more[0].2.bottom() <= col.bottom());
        // Few events all fit, all-day bands shorter than timed chips.
        let d = date(2026, 10, 22);
        let mut few = many_events(2, d);
        few.push(timed("t", at(d, 10, 0), at(d, 11, 0)));
        let l = calendar_layout(
            s,
            CalView::Week,
            oct25(),
            oct25(),
            &few,
            &CalStatus::Ok,
            0,
            false,
        );
        assert_eq!((l.rows.len(), l.week_more.len()), (3, 0));
        assert!(l.rows[0].1.h < l.rows[2].1.h);
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
            0,
            false,
        );
        // Twenty all-day events: two bands a page.
        assert_eq!(l.rows.len(), 2);
        let (hidden, more) = l.more.expect("overflow");
        assert_eq!(hidden, 20 - l.rows.len());
        let (dots, _) = page_dots(s, Page::Calendar);
        assert!(l.add.bottom() <= dots[0].y, "everything above the dots");
        assert!(more.y >= l.rows.last().unwrap().1.bottom());
        assert_eq!((more.y, more.h), (l.add.y, l.add.h), "beside the add row");
        for (_, r) in &l.rows {
            assert_eq!(r.h, 22.0);
            assert!(r.bottom() <= l.list.y, "bands above the timeline");
        }
        // Few events: all shown, no "+N".
        let l = calendar_layout(
            s,
            CalView::Day,
            oct25(),
            oct25(),
            &events[..2],
            &CalStatus::Ok,
            0,
            false,
        );
        assert_eq!(l.rows.len(), 2);
        assert_eq!(l.more, None);
    }

    #[test]
    fn connect_only_when_login_needed() {
        let s = panel(Style::Notch);
        let layout =
            |status| calendar_layout(s, CalView::Month, oct25(), oct25(), &[], &status, 0, false);
        assert!(layout(CalStatus::NotConfigured).connect.is_some());
        assert!(layout(CalStatus::NeedsLogin).connect.is_some());
        assert!(layout(CalStatus::Reconnect).connect.is_some());
        assert!(layout(CalStatus::Ok).connect.is_none());
        assert!(layout(CalStatus::Offline).connect.is_none());
    }

    #[test]
    fn connect_button_26_high_in_every_view() {
        let s = panel(Style::Pill);
        for view in [CalView::Day, CalView::Week, CalView::Month] {
            let l = calendar_layout(
                s,
                view,
                oct25(),
                oct25(),
                &[],
                &CalStatus::NeedsLogin,
                0,
                false,
            );
            assert_eq!(l.connect.expect("connect").h, 26.0, "{view:?}");
        }
    }

    #[test]
    fn day_list_pages_through_chunks_and_wraps() {
        let s = panel(Style::Pill);
        let events = many_events(20, oct25());
        let day = |page| {
            calendar_layout(
                s,
                CalView::Day,
                oct25(),
                oct25(),
                &events,
                &CalStatus::Ok,
                page,
                false,
            )
        };
        let first = day(0);
        let k = first.rows.len();
        assert!((1..20).contains(&k));
        assert_eq!(first.rows[0].0, "ev0");
        assert_eq!(first.more.map(|m| m.0), Some(20 - k));
        let second = day(1);
        assert_eq!(second.rows[0].0, format!("ev{k}"));
        assert_eq!(second.rows[0].1, first.rows[0].1, "same row slots");
        let chunks = 20usize.div_ceil(k);
        let last = day(chunks - 1);
        assert_eq!(last.rows.last().unwrap().0, "ev19");
        assert_eq!(last.more.map(|m| m.0), Some(0), "last chunk: back to top");
        assert_eq!(day(chunks).rows[0].0, "ev0", "wraps");
        // The month list always shows the first chunk.
        let month = calendar_layout(
            s,
            CalView::Month,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            3,
            false,
        );
        assert_eq!(month.rows[0].0, "ev0");
    }

    #[test]
    fn day_timeline_places_blocks_by_time_and_marks_now() {
        let s = panel(Style::Pill);
        let d = date(2026, 10, 26);
        let mut events = vec![all_day("expo", d)];
        events.push(timed("review", at(d, 10, 0), at(d, 11, 30)));
        events.push(timed("mep", at(d, 14, 0), at(d, 15, 0)));
        events.push(timed("late", at(d, 21, 0), at(d, 22, 0)));
        let day = |page, now| {
            super::calendar_layout(
                s,
                CalView::Day,
                d,
                d,
                &events,
                &CalStatus::Ok,
                page,
                false,
                200.0,
                now,
            )
        };
        let l = day(0, Some(at(d, 13, 10)));
        // 09.00 to 16.00, as in the design: from an hour before the first event.
        let hours: Vec<u32> = l.hours.iter().map(|(h, _)| *h).collect();
        assert_eq!(hours, (9..=16).collect::<Vec<_>>());
        let line = |h: u32| l.hours.iter().find(|(x, _)| *x == h).unwrap().1;
        for pair in l.hours.windows(2) {
            assert_eq!(pair[1].1 - pair[0].1, 22.0);
        }
        let row = |id: &str| l.rows.iter().find(|(x, _)| x == id).map(|(_, r)| *r);
        let band = row("expo").expect("all-day band");
        assert!(band.bottom() <= line(9));
        let review = row("review").expect("10.00 block");
        assert!((review.y - line(10)).abs() <= 1.0);
        assert!(
            (review.bottom() - (line(11) + 11.0)).abs() <= 1.0,
            "an hour and a half"
        );
        let mep = row("mep").expect("14.00 block");
        assert!((mep.y - line(14)).abs() <= 1.0);
        assert_eq!(row("late"), None, "21.00 is on the next page");
        // The blocks leave the hour labels' gutter free.
        assert!(review.x >= l.list.x + HOUR_GUTTER);
        // Now: between 13.00 and 14.00, a sixth of the way.
        let now = l.now_y.expect("now line");
        assert!((now - (line(13) + 22.0 / 6.0)).abs() < 0.01);
        assert_eq!(l.more.map(|m| m.0), Some(1));
        // Page two shows the late event; the all-day band stays.
        let p1 = day(1, Some(at(d, 13, 10)));
        assert!(p1.rows.iter().any(|(id, _)| id == "late"));
        assert!(p1.rows.iter().any(|(id, _)| id == "expo"));
        assert_eq!(p1.now_y, None, "13.10 is not in these hours");
        assert_eq!(p1.more.map(|m| m.0), Some(0), "last page: back to top");
        assert_eq!(day(2, None).hours, day(0, None).hours, "wraps");
        // Another day has no now line; an evening "now" moves the hours.
        assert_eq!(day(0, None).now_y, None);
        let evening = day(0, Some(at(d, 19, 30)));
        assert!(evening.now_y.is_some());
        assert!(evening.hours[0].0 <= 19 && evening.hours.last().unwrap().0 > 19);
        // Overlapping events go side by side.
        let both = [
            timed("a", at(d, 10, 0), at(d, 11, 0)),
            timed("b", at(d, 10, 30), at(d, 12, 0)),
        ];
        let l = super::calendar_layout(
            s,
            CalView::Day,
            d,
            d,
            &both,
            &CalStatus::Ok,
            0,
            false,
            200.0,
            None,
        );
        let (a, b) = (l.rows[0].1, l.rows[1].1);
        assert!(disjoint(a, b));
        assert_eq!(a.w, b.w);
        assert!(a.right() <= b.x || b.right() <= a.x);
        assert_eq!(l.more, None);
    }

    #[test]
    fn day_lanes_split_only_overlapping_blocks() {
        let s = panel(Style::Pill);
        let d = date(2026, 10, 26);
        let day = |events: &[CalEvent], page| {
            super::calendar_layout(
                s,
                CalView::Day,
                d,
                d,
                events,
                &CalStatus::Ok,
                page,
                false,
                200.0,
                None,
            )
        };
        let events = [
            timed("a", at(d, 10, 0), at(d, 11, 0)),
            timed("b", at(d, 10, 30), at(d, 12, 0)),
            timed("c", at(d, 14, 0), at(d, 15, 0)),
        ];
        let l = day(&events, 0);
        let row = |id: &str| l.rows.iter().find(|(x, _)| x == id).unwrap().1;
        let (a, b, c) = (row("a"), row("b"), row("c"));
        assert!(disjoint(a, b));
        assert_eq!(c.x, a.x);
        assert!(
            (c.w - (b.right() - a.x)).abs() < 0.01,
            "a lone block keeps the full width"
        );

        // Far more at once than fit side by side: lanes keep a minimum
        // width and the rest come on the next pages, each shown once.
        let crowd: Vec<CalEvent> = (0..100)
            .map(|i| timed(&format!("e{i}"), at(d, 10, 0), at(d, 11, 0)))
            .collect();
        let first = day(&crowd, 0);
        let per_page = first.rows.len();
        assert!(per_page > 1 && per_page < 100);
        let span = |l: &CalendarLayout| {
            let x0 = l.rows.iter().map(|(_, r)| r.x).fold(f32::MAX, f32::min);
            let x1 = l
                .rows
                .iter()
                .map(|(_, r)| r.right())
                .fold(f32::MIN, f32::max);
            (x0, x1)
        };
        let full = span(&first);
        let mut ids = std::collections::BTreeSet::new();
        let mut page = 0;
        loop {
            let l = day(&crowd, page);
            for (id, r) in &l.rows {
                assert!(r.w >= MIN_LANE_W, "{id}: {r:?}");
                ids.insert(id.clone());
            }
            // Every page's run fills the width: no blank lane.
            let (x0, x1) = span(&l);
            assert!((x0 - full.0).abs() < 0.01 && (x1 - full.1).abs() < 0.01);
            let (count, _) = l.more.expect("paged");
            if count == 0 {
                break;
            }
            assert_eq!(count, 100 - ids.len());
            page += 1;
            assert!(page < 100, "pages end");
        }
        assert_eq!(ids.len(), 100);
        assert_eq!(page + 1, 100usize.div_ceil(per_page));
    }

    #[test]
    fn notes_list_caps_rows() {
        let s = panel(Style::Pill);
        let all = notes(30);
        let l = notes_layout(s, &all, 0, false);
        let k = l.rows.len();
        assert_eq!(k, 4, "two rows of two cards");
        assert_eq!(l.rows[0].0, "n0");
        assert_eq!(l.more.as_ref().map(|m| m.0), Some(30 - k));
        for (_, r) in &l.rows {
            assert_eq!(r.h, 92.0);
        }
        // Two columns: newest top left, then across.
        let r: Vec<Rect> = l.rows.iter().map(|(_, r)| *r).collect();
        assert_eq!((r[0].y, r[1].y), (r[0].y, r[0].y));
        assert_eq!((r[2].x, r[3].x), (r[0].x, r[1].x));
        assert_eq!(r[1].x - r[0].right(), 9.0);
        assert_eq!(r[2].y - r[0].bottom(), 9.0);
        assert_eq!(r[0].x, left(s));
        assert!((r[1].right() - right(s)).abs() < 0.01);
        assert!(l.groups.is_empty());
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
    fn reminder_list_pages_like_notes() {
        let s = panel(Style::Notch);
        let all = reminders(12);
        let first = reminders_layout(s, &all, 0, false);
        let k = first.rows.len();
        assert!((3..12).contains(&k), "{k} rows");
        assert_eq!(first.rows[0].0, "r0");
        assert_eq!(first.more.as_ref().map(|m| m.0), Some(12 - k));
        let second = reminders_layout(s, &all, 1, false);
        assert_eq!(second.rows[0].0, format!("r{k}"));
        assert_eq!(second.rows[0].1, first.rows[0].1, "same row slots");
        let chunks = 12usize.div_ceil(k);
        let last = reminders_layout(s, &all, chunks - 1, false);
        assert_eq!(last.more.as_ref().map(|m| m.0), Some(0), "back to top");
        assert_eq!(reminders_layout(s, &all, chunks, false).rows[0].0, "r0");
        // Three fit without paging, under one heading (overdue ones join
        // today's group).
        let three = reminders_layout(s, &all[..3], 0, false);
        assert_eq!((three.rows.len(), three.more.is_none()), (3, true));
        assert_eq!(three.groups.len(), 1);
        assert_eq!(three.groups[0].0, oct25());
        assert!(three.groups[0].1.bottom() <= three.rows[0].1.y);
        let notes3 = notes_layout(s, &notes(3), 0, false);
        assert_eq!(three.new_button, notes3.new_button);
        // Every chunk starts with its heading.
        assert_eq!(second.groups[0].1, first.groups[0].1);
    }

    #[test]
    fn reminders_group_by_day_with_overdue_under_today() {
        use chrono::{Local, TimeZone};
        let s = panel(Style::Pill);
        let today = oct25();
        let due = |id: &str, t: NaiveDateTime| Reminder {
            id: id.into(),
            text: id.into(),
            due_unix: Local
                .from_local_datetime(&t)
                .earliest()
                .unwrap()
                .timestamp(),
            repeat: Default::default(),
            done: false,
        };
        let all = [
            due("late", at(date(2026, 10, 24), 8, 0)),
            due("today", at(today, 16, 30)),
            due("tomorrow", at(date(2026, 10, 26), 9, 30)),
            due("tomorrow2", at(date(2026, 10, 26), 13, 0)),
            due("friday", at(date(2026, 10, 30), 13, 0)),
        ];
        let l = super::reminders_layout(s, &all, 0, false, today);
        let days: Vec<NaiveDate> = l.groups.iter().map(|(d, _)| *d).collect();
        assert_eq!(days, [today, date(2026, 10, 26), date(2026, 10, 30)]);
        assert_eq!(l.rows.len(), 5);
        assert_eq!(l.more, None);
        // Each heading sits over its first row; later groups a step apart.
        let row = |id: &str| l.rows.iter().find(|(x, _)| x == id).unwrap().1;
        assert_eq!(l.groups[0].1.bottom(), row("late").y);
        assert_eq!(row("today").y, row("late").bottom());
        assert_eq!(l.groups[1].1.bottom(), row("tomorrow").y);
        assert!(l.groups[1].1.y > row("today").bottom());
        assert_eq!(l.groups[2].1.bottom(), row("friday").y);
        let (dots, _) = page_dots(s, Page::Reminders);
        assert!(row("friday").bottom() <= dots[0].y);
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
                    0,
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
            assert!(b.lyrics.y > a.lyrics.y && b.label.y > a.label.y);
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
            0,
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
            0,
            false,
        );
        let r = regions(s, &PageLayout::Calendar(l), false);
        assert!(has(&r, Hit::CalDate(date(2026, 10, 19))));
        assert!(has(&r, Hit::CalMore(oct25())));

        // Day view's "+N lagi" pages through the open day.
        let l = calendar_layout(
            s,
            CalView::Day,
            oct25(),
            oct25(),
            &events,
            &CalStatus::Ok,
            0,
            false,
        );
        assert!(l.more.is_some());
        let r = regions(s, &PageLayout::Calendar(l), false);
        assert!(has(&r, Hit::CalMore(oct25())));

        let r = regions(s, &PageLayout::Pomodoro(pomodoro_layout(s, false)), false);
        for hit in [Hit::PomoStartPause, Hit::PomoReset, Hit::PomoSkip] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }
        // Durations are set in the settings window only.
        assert!(!r.iter().any(|r| matches!(r.hit, Hit::PomoAdjust(..))));

        let r = regions(
            s,
            &PageLayout::Notes(notes_layout(s, &notes(30), 0, false)),
            false,
        );
        for hit in [Hit::NoteNew, Hit::NoteOpen("n0".into()), Hit::NotesMore] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }

        let r = regions(
            s,
            &PageLayout::Reminders(reminders_layout(s, &reminders(12), 0, false)),
            false,
        );
        for hit in [
            Hit::ReminderNew,
            Hit::ReminderOpen("r0".into()),
            Hit::RemindersMore,
        ] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }
        assert!(!r.iter().any(|r| matches!(r.hit, Hit::NoteOpen(_))));

        let r = regions(s, &PageLayout::Media(media_panel_layout(s, false)), false);
        for hit in [Hit::Prev, Hit::PlayPause, Hit::Next, Hit::LyricsToggle] {
            assert!(has(&r, hit.clone()), "{hit:?}");
        }

        let l = cli_layout(s, false);
        let r = regions(s, &PageLayout::Cli(l), false);
        assert_eq!(
            r,
            [
                Region {
                    rect: l.card,
                    hit: Hit::CliOpen
                },
                Region {
                    rect: l.button,
                    hit: Hit::CliOpen
                }
            ]
        );

        let r = regions(s, &PageLayout::Shortcuts(shortcuts_layout(s, false)), false);
        let hits: Vec<Hit> = r.iter().map(|r| r.hit.clone()).collect();
        let want: Vec<Hit> = Shortcut::ALL.into_iter().map(Hit::Shortcut).collect();
        assert_eq!(hits, want);
    }

    #[test]
    fn shortcut_tiles_two_rows_of_four_in_order() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for strip in [false, true] {
                let l = shortcuts_layout(s, strip);
                let order: Vec<Shortcut> = l.tiles.iter().map(|(t, _)| *t).collect();
                assert_eq!(order, Shortcut::ALL);
                let first = l.tiles[0].1;
                assert!(first.h >= 60.0, "{style:?} strip={strip}: {first:?}");
                assert!(first.y >= l.header.bottom());
                for (i, (_, t)) in l.tiles.iter().enumerate() {
                    assert_eq!((t.w, t.h), (first.w, first.h), "equal tiles");
                    let (row, col) = (i / 4, i % 4);
                    assert_eq!(t.y, l.tiles[row * 4].1.y, "rows line up");
                    assert_eq!(t.x, l.tiles[col].1.x, "columns line up");
                    let (glyph, label) = tile_parts(*t);
                    assert!(glyph.y >= t.y && label.bottom() <= t.bottom());
                    assert!(glyph.bottom() <= label.y);
                }
                // Edge to edge across the content width, 10 px apart.
                assert_eq!(first.x, left(s));
                assert!((l.tiles[3].1.right() - right(s)).abs() < 0.01);
                assert_eq!(l.tiles[1].1.x - first.right(), 10.0);
                assert_eq!(l.tiles[4].1.y - first.bottom(), 10.0);
            }
        }
    }

    #[test]
    fn cli_card_holds_the_prompt_and_four_lines_above_its_button() {
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for strip in [false, true] {
                let l = cli_layout(s, strip);
                assert_eq!(l.lines.len(), 5);
                for line in l.lines {
                    assert!(line.x > l.card.x && line.right() < l.card.right());
                    assert!(line.y >= l.card.y && line.bottom() <= l.card.bottom());
                }
                for pair in l.lines.windows(2) {
                    assert!(pair[0].bottom() <= pair[1].y);
                }
                assert!(l.card.y >= l.header.bottom());
                assert!(l.button.y >= l.card.bottom());
                assert_eq!(l.button.right(), l.card.right(), "right-aligned");
                assert!(l.header.right() < l.status.x);
                assert_eq!(l.status.right(), right(s));
            }
            let (a, b) = (cli_layout(s, false), cli_layout(s, true));
            assert_eq!(b.header.y - a.header.y, 24.0);
            assert!(b.card.y >= a.card.y);
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
                            let r = l.ring_radius + RING_STROKE / 2.0;
                            drawn.push(Rect {
                                x: cx - r,
                                y: cy - r,
                                w: 2.0 * r,
                                h: 2.0 * r,
                            });
                            drawn.extend([l.time, l.phase, l.session, l.durations, l.next_up]);
                        }
                        PageLayout::Notes(l) | PageLayout::Reminders(l) => {
                            drawn.push(l.header);
                            drawn.extend(l.groups.iter().map(|(_, r)| *r));
                        }
                        PageLayout::Media(l) => drawn.extend([l.art, l.label, l.title, l.artist]),
                        PageLayout::Cli(l) => {
                            drawn.extend([l.header, l.status]);
                            drawn.extend(l.lines);
                        }
                        PageLayout::Shortcuts(l) => {
                            drawn.push(l.header);
                            for (_, tile) in l.tiles {
                                let (glyph, label) = tile_parts(tile);
                                drawn.extend([glyph, label]);
                            }
                        }
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
        for style in [Style::Pill, Style::Notch] {
            let s = panel(style);
            for strip in [false, true] {
                let l = pomodoro_layout(s, strip);
                assert_eq!(l.ring_radius, 92.0);
                let (cx, cy) = l.ring_center;
                // The ring and its stroke stay between the header and the dots.
                let outer = l.ring_radius + RING_STROKE / 2.0;
                assert!(
                    cy + outer <= content_bottom(s) + 0.01,
                    "{style:?} strip={strip}"
                );
                assert!(cy - outer >= l.header.bottom() - 0.01);
                // The countdown box is centred on the ring's axis, a little
                // above its centre, and wide enough for the widest countdown
                // in Fraunces It 48 ("100:00", 157.7 px).
                assert!((l.time.x + l.time.w / 2.0 - cx).abs() < 0.01);
                let time_cy = l.time.y + l.time.h / 2.0;
                assert!(time_cy < cy && cy - time_cy <= 12.0);
                assert!(l.time.w >= 158.0, "time box {}", l.time.w);
                // The digits' ink band across the box stays inside the
                // stroke's inner edge.
                let inner = l.ring_radius - RING_STROKE / 2.0;
                let far = (time_cy - TIME_INK_HALF - cy).abs();
                let corner = (l.time.w / 2.0).hypot(far);
                assert!(corner <= inner + 0.01, "corner {corner} vs inner {inner}");
                // The phase sits under the digits, inside the ring too.
                assert!(l.phase.y >= time_cy + TIME_INK_HALF);
                assert!((l.phase.x + l.phase.w / 2.0 - cx).abs() < 0.01);
                let low = l.phase.bottom() - cy;
                assert!((l.phase.w / 2.0).hypot(low) <= inner + 0.01);
                assert!(l.phase.w >= 120.0, "phase box {}", l.phase.w);
                assert!(cx < s.x + s.w / 2.0, "ring in the left half");
                // Ring and column together sit centred in the panel.
                let group_l = cx - outer;
                let group_r = l.session.right();
                let (ml, mr) = (group_l - left(s), right(s) - group_r);
                assert!((ml - mr).abs() < 0.01, "{style:?}: margins {ml} / {mr}");
                // The column starts right of the ring, top to bottom.
                for r in [l.session, l.reset, l.next_up] {
                    assert!(r.x >= cx + outer + 8.0, "{r:?} right of the ring");
                }
                assert!(l.session.bottom() <= l.start_pause.y);
                assert!(l.start_pause.bottom() <= l.next_up.y);
                // Reset, the big primary button, skip: in order, round,
                // centred on one line, without touching.
                assert!(l.reset.right() < l.start_pause.x && l.start_pause.right() < l.skip.x);
                assert!(l.start_pause.w > l.reset.w);
                for b in [l.reset, l.start_pause, l.skip] {
                    assert_eq!(b.w, b.h, "round");
                    let mid = b.y + b.h / 2.0;
                    assert!((mid - (l.start_pause.y + l.start_pause.h / 2.0)).abs() < 0.01);
                }
                assert!(l.skip.right() <= l.session.right());
                // The column is centred on the ring, below the header.
                let mid = (l.session.y + l.next_up.bottom()) / 2.0;
                assert!((mid - cy).abs() < 0.01);
                // Durations right of the title.
                assert!(l.header.right() < l.durations.x);
                assert_eq!(l.durations.right(), right(s));
            }
        }
    }

    #[test]
    fn pomodoro_compact_fits_its_shape() {
        for style in [Style::Pill, Style::Notch] {
            let s = shape_for(style, ShapeKind::PomodoroCompact);
            let ((cx, cy), r, text) = pomodoro_compact_layout(s);
            assert_eq!(r, 8.0);
            assert_eq!(cy, s.y + s.h / 2.0);
            assert!(cx - r - 1.5 >= s.x + 4.0);
            assert!(text.x >= cx + r);
            assert!(text.right() <= s.x + s.w);
            assert!(rect_inside(style, s, text), "{style:?} {text:?}");
        }
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use crate::config::Style;
    use crate::ui::geometry::{ShapeKind, shape_for};

    #[test]
    fn lyric_button_ends_the_transport_row() {
        for style in [Style::Pill, Style::Notch] {
            let s = shape_for(style, ShapeKind::Panel);
            for strip in [false, true] {
                let l = media_panel_layout(s, strip);
                assert_eq!(l.lyrics.right(), right(s));
                // On the buttons' line, right of them, touching none.
                let [prev, play, next] = l.buttons;
                let mid = |r: Rect| r.y + r.h / 2.0;
                assert!((mid(l.lyrics) - mid(play)).abs() < 0.01);
                assert!(prev.right() < play.x && play.right() < next.x);
                assert!(next.right() < l.lyrics.x);
                assert_eq!(play.w, play.h, "the round primary button");
                // The text column: label, title, artist, then the row.
                assert!(l.label.bottom() <= l.title.y && l.title.bottom() <= l.artist.y);
                assert!(l.artist.bottom() <= play.y);
                assert!(l.label.x >= l.art.right() + 16.0);
                // A big square cover, centred between the header and the dots.
                assert_eq!(l.art.w, l.art.h);
                assert!(l.art.w >= 180.0, "{:?}", l.art);
                assert_eq!(l.art.x, left(s));
                let gap_top = l.art.y - content_top(s, strip);
                let gap_bottom = content_bottom(s) - l.art.bottom();
                assert!((gap_top - gap_bottom).abs() < 0.01);
            }
        }
    }
}
