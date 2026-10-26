//! The month grid shared by the panel's Calendar page and the editors'
//! calendar popover: 42 days from the Monday on or before the 1st, one
//! circle per day, and the rules for which day is filled, ringed or dim.
//! Geometry and the cell's look are pure; each renderer supplies the drawing
//! primitives through `CellCanvas`.

use chrono::{Datelike, Days, NaiveDate};

use super::geometry::Rect;
use crate::calendar::range_for;
use crate::model::CalView;

/// Days in the grid: six weeks.
pub const CELLS: usize = 42;
/// Diameter of the small dot under a day with events.
const EVENT_DOT: f32 = 3.5;

/// The 42 days of `anchor`'s month view, Monday first, each with its cell:
/// columns `cell_w` wide from `x`, rows `pitch` apart from `y`, each cell
/// `pitch - gap_y` high.
pub fn month_cells(
    anchor: NaiveDate,
    x: f32,
    y: f32,
    cell_w: f32,
    pitch: f32,
    gap_y: f32,
) -> Vec<(NaiveDate, Rect)> {
    let (first, _) = range_for(CalView::Month, anchor);
    (0..CELLS)
        .filter_map(|i| {
            let d = first.checked_add_days(Days::new(i as u64))?;
            let cell = Rect {
                x: x + (i % 7) as f32 * cell_w,
                y: y + (i / 7) as f32 * pitch,
                w: cell_w,
                h: pitch - gap_y,
            };
            Some((d, cell))
        })
        .collect()
}

/// Where a day's parts go inside its cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayGeometry {
    /// The box the number is centred in.
    pub digits: Rect,
    /// The day's circle (fill or ring), centred in `digits`.
    pub circle: Rect,
    /// The event dot under the number.
    pub dot: Rect,
}

/// The cell `r` laid out: the number's box is the cell less `bottom_trim`,
/// the circle one px shorter than that box, at most `max_side`.
pub fn day_geometry(r: Rect, bottom_trim: f32, max_side: f32) -> DayGeometry {
    let digits = Rect {
        h: r.h - bottom_trim,
        ..r
    };
    let side = (digits.h - 1.0).min(max_side).max(0.0);
    DayGeometry {
        digits,
        circle: Rect {
            x: r.x + (r.w - side) / 2.0,
            y: digits.y + (digits.h - side) / 2.0,
            w: side,
            h: side,
        },
        dot: Rect {
            x: r.x + (r.w - EVENT_DOT) / 2.0,
            y: r.bottom() - EVENT_DOT - 1.5,
            w: EVENT_DOT,
            h: EVENT_DOT,
        },
    }
}

/// What is true of one day in the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DayState {
    pub today: bool,
    pub selected: bool,
    /// In the month the grid shows.
    pub in_month: bool,
    pub events: bool,
    pub hover: bool,
}

impl DayState {
    pub fn of(d: NaiveDate, today: NaiveDate, selected: NaiveDate, month_of: NaiveDate) -> Self {
        DayState {
            today: d == today,
            selected: d == selected,
            in_month: d.month() == month_of.month() && d.year() == month_of.year(),
            events: false,
            hover: false,
        }
    }
}

/// Which mark gets the solid accent circle; the other one gets the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filled {
    /// The panel: today filled, the selected day ringed.
    Today,
    /// The date picker: the picked day filled, today ringed.
    Selected,
}

/// Colours and weights of a grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLook {
    pub filled: Filled,
    pub ring_w: f32,
    /// The ringed day's number in the accent and bold (the picker), or as
    /// any other day (the panel).
    pub ring_ink_accent: bool,
    pub accent: u32,
    pub on_accent: u32,
    pub text: u32,
    /// Days of other months.
    pub dim: u32,
    pub dot: u32,
    /// Fill under a hovered day; `None` draws no hover.
    pub hover: Option<u32>,
}

/// How a day's parts are drawn, as decided by `day_marks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayMarks {
    /// Solid circle colour.
    pub disc: Option<u32>,
    /// Ring colour.
    pub ring: Option<u32>,
    pub ink: u32,
    pub bold: bool,
    pub dot: Option<u32>,
}

/// The look of one day: pure, so both grids can be tested without drawing.
pub fn day_marks(s: DayState, look: &CellLook) -> DayMarks {
    let (filled, ringed) = match look.filled {
        Filled::Today => (s.today, s.selected),
        Filled::Selected => (s.selected, s.today),
    };
    if filled {
        return DayMarks {
            disc: Some(look.accent),
            ring: None,
            ink: look.on_accent,
            bold: true,
            // A filled circle fills the cell: its dot would only clip it.
            dot: None,
        };
    }
    let accent_ink = ringed && look.ring_ink_accent;
    DayMarks {
        disc: look.hover.filter(|_| s.hover),
        ring: ringed.then_some(look.accent),
        ink: if accent_ink {
            look.accent
        } else if s.in_month {
            look.text
        } else {
            look.dim
        },
        bold: accent_ink,
        dot: s.events.then_some(look.dot),
    }
}

/// The drawing a renderer provides for `paint_day`.
pub trait CellCanvas {
    /// A solid circle filling `circle`.
    fn disc(&self, circle: Rect, rgb: u32);
    /// A ring of `width` just inside `circle`.
    fn ring(&self, circle: Rect, width: f32, rgb: u32);
    /// The day's number centred in `area`.
    fn number(&self, text: &str, area: Rect, rgb: u32, bold: bool);
}

/// Draws day `d` in cell `r` (laid out by `geometry`) as `day_marks` says.
pub fn paint_day(
    canvas: &impl CellCanvas,
    d: NaiveDate,
    geometry: DayGeometry,
    state: DayState,
    look: &CellLook,
) {
    let marks = day_marks(state, look);
    if let Some(rgb) = marks.disc {
        canvas.disc(geometry.circle, rgb);
    }
    if let Some(rgb) = marks.ring {
        canvas.ring(geometry.circle, look.ring_w, rgb);
    }
    canvas.number(&d.day().to_string(), geometry.digits, marks.ink, marks.bold);
    if let Some(rgb) = marks.dot {
        canvas.disc(geometry.dot, rgb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn forty_two_days_from_the_monday_before_the_first() {
        let cells = month_cells(date(2026, 10, 15), 10.0, 20.0, 30.0, 34.0, 2.0);
        assert_eq!(cells.len(), CELLS);
        assert_eq!(cells[0].0, date(2026, 9, 28));
        assert_eq!(cells[41].0, date(2026, 11, 8));
        assert_eq!(
            cells[0].1,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 32.0
            }
        );
        assert_eq!(
            cells[8].1,
            Rect {
                x: 40.0,
                y: 54.0,
                w: 30.0,
                h: 32.0
            }
        );
        // A month starting on a Monday starts the grid on its 1st.
        assert_eq!(
            month_cells(date(2026, 6, 30), 0.0, 0.0, 1.0, 1.0, 0.0)[0].0,
            date(2026, 6, 1)
        );
    }

    #[test]
    fn day_geometry_centres_the_circle() {
        let g = day_geometry(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 32.0,
            },
            0.0,
            30.0,
        );
        assert_eq!(
            g.circle,
            Rect {
                x: 5.0,
                y: 1.0,
                w: 30.0,
                h: 30.0
            }
        );
        // The panel's cells: 3 px trimmed for the dot, circles up to 26.
        let p = day_geometry(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 24.0,
            },
            3.0,
            26.0,
        );
        assert_eq!(p.digits.h, 21.0);
        assert_eq!(p.circle.w, 20.0);
        assert_eq!(p.dot.y, 24.0 - 3.5 - 1.5);
    }

    fn look(filled: Filled) -> CellLook {
        CellLook {
            filled,
            ring_w: 1.5,
            ring_ink_accent: filled == Filled::Selected,
            accent: 1,
            on_accent: 2,
            text: 3,
            dim: 4,
            dot: 5,
            hover: Some(6),
        }
    }

    #[test]
    fn panel_fills_today_and_rings_the_selection() {
        let l = look(Filled::Today);
        let today = DayState {
            today: true,
            in_month: true,
            events: true,
            ..Default::default()
        };
        let m = day_marks(today, &l);
        assert_eq!(
            (m.disc, m.ring, m.ink, m.bold, m.dot),
            (Some(1), None, 2, true, None)
        );
        let sel = DayState {
            selected: true,
            in_month: true,
            events: true,
            ..Default::default()
        };
        let m = day_marks(sel, &l);
        assert_eq!(
            (m.disc, m.ring, m.ink, m.bold, m.dot),
            (None, Some(1), 3, false, Some(5))
        );
        let other = DayState::default();
        assert_eq!(day_marks(other, &l).ink, 4);
    }

    #[test]
    fn picker_fills_the_selection_and_rings_today_in_the_accent() {
        let l = look(Filled::Selected);
        let sel = DayState {
            selected: true,
            today: true,
            ..Default::default()
        };
        let m = day_marks(sel, &l);
        assert_eq!((m.disc, m.ring, m.ink, m.bold), (Some(1), None, 2, true));
        let today = DayState {
            today: true,
            in_month: true,
            ..Default::default()
        };
        let m = day_marks(today, &l);
        assert_eq!((m.disc, m.ring, m.ink, m.bold), (None, Some(1), 1, true));
        let hovered = DayState {
            hover: true,
            in_month: true,
            ..Default::default()
        };
        let m = day_marks(hovered, &l);
        assert_eq!((m.disc, m.ink), (Some(6), 3));
        let state = DayState::of(
            date(2026, 9, 30),
            date(2026, 10, 26),
            date(2026, 10, 28),
            date(2026, 10, 1),
        );
        assert!(!state.in_month && !state.today && !state.selected);
    }
}
