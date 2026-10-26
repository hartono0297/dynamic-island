//! Where everything in an editor goes, in logical px: the frame (header,
//! close button, footer) and each form's rows, following `.ed`, `.hd`,
//! `.bd`, `.ft` in `docs/design/editor-acara.html`. Pure: text widths come
//! from a `measure` function, so tests run without DirectWrite.

use chrono::{Days, NaiveDate};

use super::geometry::Rect;
use super::icons::Icon;
use crate::reminders::Repeat;

/// Window width of every editor.
pub const WIDTH: f32 = 400.0;
const PAD_X: f32 = 18.0;
/// The header: title and close button; dragging it moves the window.
pub const HEADER_H: f32 = 48.0;
const BODY_TOP: f32 = 54.0;
const GAP: f32 = 12.0;
/// The title line: 8 above the text, 10 below, then a 1.5 px rule.
const TITLE_H: f32 = 42.0;
pub const TITLE_RULE: f32 = 1.5;
/// Chips, the repeat track and the all-day row.
pub const ROW_H: f32 = 30.0;
const PILL_H: f32 = 22.0;
const PILL_GAP: f32 = 6.0;
const NOTES_H: f32 = 62.0;
const NOTE_BOX_H: f32 = 150.0;
/// The one-line error under the time row: pulled 6 px up into the gap.
const ERR_H: f32 = 14.0;
const ERR_PULL: f32 = 6.0;
const BODY_BOTTOM: f32 = 16.0;
const FOOTER_H: f32 = 57.0;
const BUTTON_H: f32 = 29.0;
const CLOSE: f32 = 26.0;
/// The icon column left of each row, and the gap after it.
const ICON_COL: f32 = 18.0;
pub const ROW_ICON: f32 = 17.0;
const FIELD_X: f32 = PAD_X + ICON_COL + 8.0;
const TOGGLE_W: f32 = 32.0;
const TOGGLE_H: f32 = 18.0;
const SEG_W: f32 = 84.0;
const SEG_PAD: f32 = 3.0;
/// Chip padding (11 each side) plus its 1 px border.
const CHIP_PAD: f32 = 12.0;
/// The chevron after a date chip's text: a 6 px gap and an 11 px icon.
pub const CHEVRON: f32 = 11.0;
const CHEVRON_GAP: f32 = 6.0;
/// The notes box padding the EDIT sits in.
const NOTES_PAD: (f32, f32) = (11.0, 9.0);
const NOTE_PAD: (f32, f32) = (12.0, 10.0);

/// Which kind of editor a layout is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Note,
    Event,
    Reminder,
}

/// The date pills under the event's date chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Today,
    Tomorrow,
    /// The day after tomorrow, named by its weekday ("Jumat").
    DayAfter,
    /// A week from today.
    NextWeek,
}

impl Shortcut {
    pub const ALL: [Shortcut; 4] = [
        Shortcut::Today,
        Shortcut::Tomorrow,
        Shortcut::DayAfter,
        Shortcut::NextWeek,
    ];

    pub fn date(self, today: NaiveDate) -> NaiveDate {
        let days = match self {
            Shortcut::Today => 0,
            Shortcut::Tomorrow => 1,
            Shortcut::DayAfter => 2,
            Shortcut::NextWeek => 7,
        };
        today.checked_add_days(Days::new(days)).unwrap_or(today)
    }

    /// The pill matching `date`, if any (the active one).
    pub fn matching(today: NaiveDate, date: NaiveDate) -> Option<Shortcut> {
        Shortcut::ALL.into_iter().find(|s| s.date(today) == date)
    }
}

/// Something in the editor that can be clicked, focused or hold an EDIT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Close,
    /// The event title or reminder text (an EDIT).
    Title,
    DateChip,
    Pill(Shortcut),
    AllDay,
    StartChip,
    EndChip,
    /// The reminder's time.
    TimeChip,
    Repeat(Repeat),
    /// Event notes or the note's text (an EDIT).
    Notes,
    Delete,
    /// Batal, or Tutup on a read-only event.
    Cancel,
    Save,
}

impl Item {
    /// Held by a native EDIT rather than drawn.
    pub fn is_edit(self) -> bool {
        matches!(self, Item::Title | Item::Notes)
    }
}

/// The text styles of the editors and popovers. Fraunces Italic only for
/// the two headings (21 and 16 px); everything else DM Sans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Text {
    /// The window title, Fraunces Italic 21.
    Heading,
    /// The calendar popover's month, Fraunces Italic 16.
    Month,
    /// Chips and time rows, 12.
    Body,
    /// The picked time row, 12 medium.
    BodyStrong,
    /// Muted captions: "Sepanjang hari", the arrow, durations, 11.
    Small,
    /// Pills, the footer account, the error line, 10.5.
    Mini,
    /// Batal, 11.5.
    Button,
    /// Simpan, 11.5 medium.
    ButtonStrong,
    /// The calendar's weekday row, 9.5.
    Weekday,
    /// Day numbers, 11.5; bold for today and the pick.
    Day,
    DayBold,
}

impl Text {
    pub const ALL: [Text; 11] = [
        Text::Heading,
        Text::Month,
        Text::Body,
        Text::BodyStrong,
        Text::Small,
        Text::Mini,
        Text::Button,
        Text::ButtonStrong,
        Text::Weekday,
        Text::Day,
        Text::DayBold,
    ];

    pub fn size(self) -> f32 {
        match self {
            Text::Heading => 21.0,
            Text::Month => 16.0,
            Text::Body | Text::BodyStrong => 12.0,
            Text::Small => 11.0,
            Text::Mini => 10.5,
            Text::Button | Text::ButtonStrong | Text::Day | Text::DayBold => 11.5,
            Text::Weekday => 9.5,
        }
    }

    /// CSS weight: 400, 500 or 600.
    pub fn weight(self) -> u16 {
        match self {
            Text::Heading | Text::Month | Text::BodyStrong | Text::ButtonStrong => 500,
            Text::DayBold => 600,
            _ => 400,
        }
    }

    /// Set in Fraunces Italic (16 px and up only).
    pub fn display(self) -> bool {
        matches!(self, Text::Heading | Text::Month)
    }
}

/// The labels whose widths shape the layout.
#[derive(Clone, Copy, Debug, Default)]
pub struct Labels<'a> {
    pub date: &'a str,
    pub start: &'a str,
    pub end: &'a str,
    pub time: &'a str,
    pub pills: [&'a str; 4],
    pub all_day: &'a str,
    pub delete: &'a str,
    pub cancel: &'a str,
    pub save: &'a str,
}

/// What the layout depends on besides the labels.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    pub form: Form,
    /// Event only: Tutup instead of Batal and Simpan.
    pub read_only: bool,
    /// Event only: hides the time row.
    pub all_day: bool,
    /// The one-line error under the time (event) or repeat (reminder) row.
    pub error: bool,
    /// Hapus on the left of the footer (editing a note or reminder).
    pub delete: bool,
    pub labels: Labels<'a>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Window size.
    pub size: (f32, f32),
    pub heading: Rect,
    /// Interactive parts in Tab order (the close button first, outside it).
    pub items: Vec<(Item, Rect)>,
    /// Where each EDIT goes: the box its text runs in.
    pub edits: Vec<(Item, Rect)>,
    /// Row icons.
    pub icons: Vec<(Icon, Rect)>,
    /// The title's underline.
    pub title_rule: Option<Rect>,
    /// The field fill around the notes EDIT.
    pub notes_box: Option<Rect>,
    pub all_day_label: Option<Rect>,
    pub toggle: Option<Rect>,
    /// "→" between the start and end chips, and the duration after them.
    pub arrow: Option<Rect>,
    pub duration: Option<Rect>,
    pub repeat_track: Option<Rect>,
    pub error: Option<Rect>,
    /// The hairline over the footer.
    pub footer_rule: Rect,
    /// The sage dot before "Google Calendar".
    pub dot: Option<Rect>,
    /// Footer text: the account, or the saving status.
    pub status: Rect,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Layout {
    pub fn item(&self, item: Item) -> Option<Rect> {
        self.items.iter().find(|(i, _)| *i == item).map(|(_, r)| *r)
    }

    /// The topmost item under a point.
    pub fn hit(&self, x: f32, y: f32) -> Option<Item> {
        self.items
            .iter()
            .rev()
            .find(|(_, r)| r.contains(x, y))
            .map(|(i, _)| *i)
    }

    /// Items in Tab order (the close button is left to the mouse and Esc).
    pub fn focus_order(&self) -> Vec<Item> {
        self.items
            .iter()
            .map(|(i, _)| *i)
            .filter(|i| *i != Item::Close)
            .collect()
    }

    /// The header strip that drags the window: everything above the body
    /// except the close button.
    pub fn is_caption(&self, x: f32, y: f32) -> bool {
        rect(0.0, 0.0, self.size.0, HEADER_H).contains(x, y)
            && !self.item(Item::Close).is_some_and(|r| r.contains(x, y))
    }
}

/// Width of a chip holding `text` (and the date chip's chevron).
fn chip_w(text_w: f32, chevron: bool) -> f32 {
    let extra = if chevron { CHEVRON_GAP + CHEVRON } else { 0.0 };
    2.0 * CHIP_PAD + text_w + extra
}

/// Lays out `spec`; `measure` gives a label's one-line width in a style.
pub fn layout(spec: &Spec, measure: &dyn Fn(&str, Text) -> f32) -> Layout {
    let w = WIDTH;
    let right = w - PAD_X;
    let mut l = Layout {
        heading: rect(PAD_X, 13.0, right - CLOSE - 8.0 - PAD_X, 32.0),
        ..Layout::default()
    };
    l.items
        .push((Item::Close, rect(right - CLOSE, 16.0, CLOSE, CLOSE)));
    let labels = &spec.labels;
    let mut y = BODY_TOP;
    let icon = |l: &mut Layout, icon: Icon, y: f32| {
        l.icons.push((icon, rect(PAD_X, y, ICON_COL, ROW_H)));
    };

    if spec.form == Form::Note {
        let b = rect(PAD_X, y, right - PAD_X, NOTE_BOX_H);
        l.notes_box = Some(b);
        l.items.push((Item::Notes, b));
        let (px, py) = NOTE_PAD;
        l.edits.push((
            Item::Notes,
            rect(b.x + px, b.y + py, b.w - 2.0 * px, b.h - 2.0 * py),
        ));
        y += NOTE_BOX_H;
    } else {
        // The title line.
        let title = rect(PAD_X, y, right - PAD_X, TITLE_H);
        l.items.push((Item::Title, title));
        l.edits
            .push((Item::Title, rect(PAD_X, y + 8.0, title.w, 22.0)));
        l.title_rule = Some(rect(PAD_X, y + TITLE_H - TITLE_RULE, title.w, TITLE_RULE));
        y += TITLE_H + GAP;

        // The date row.
        icon(&mut l, Icon::Calendar, y);
        let date_w = chip_w(measure(labels.date, Text::Body), true);
        l.items
            .push((Item::DateChip, rect(FIELD_X, y, date_w, ROW_H)));
        if spec.form == Form::Event {
            let toggle = rect(
                right - TOGGLE_W,
                y + (ROW_H - TOGGLE_H) / 2.0,
                TOGGLE_W,
                TOGGLE_H,
            );
            let label_w = measure(labels.all_day, Text::Small).ceil();
            let label = rect(toggle.x - 8.0 - label_w, y, label_w, ROW_H);
            l.toggle = Some(toggle);
            l.all_day_label = Some(label);
            y += ROW_H + GAP;

            // The shortcut pills.
            let mut x = FIELD_X;
            for (s, text) in Shortcut::ALL.into_iter().zip(labels.pills) {
                let pw = measure(text, Text::Mini).ceil() + 22.0;
                l.items.push((Item::Pill(s), rect(x, y, pw, PILL_H)));
                x += pw + PILL_GAP;
            }
            // Tab reaches the toggle after the pills.
            l.items.push((
                Item::AllDay,
                rect(
                    label.x - 4.0,
                    y - GAP - ROW_H + 2.0,
                    right + 2.0 - (label.x - 4.0),
                    ROW_H - 4.0,
                ),
            ));
            y += PILL_H + GAP;

            if !spec.all_day {
                icon(&mut l, Icon::Clock, y);
                let sw = chip_w(measure(labels.start, Text::Body), false);
                let start = rect(FIELD_X, y, sw, ROW_H);
                let arrow = rect(start.right() + 4.0, y, 16.0, ROW_H);
                let ew = chip_w(measure(labels.end, Text::Body), false);
                let end = rect(arrow.right() + 4.0, y, ew, ROW_H);
                let dx = end.right() + 12.0;
                l.items.push((Item::StartChip, start));
                l.items.push((Item::EndChip, end));
                l.arrow = Some(arrow);
                l.duration = Some(rect(dx, y, (right - dx).max(0.0), ROW_H));
                y += ROW_H;
                if spec.error {
                    l.error = Some(rect(FIELD_X, y + GAP - ERR_PULL, right - FIELD_X, ERR_H));
                    y += GAP - ERR_PULL + ERR_H;
                }
                y += GAP;
            }

            // Notes.
            l.icons
                .push((Icon::Note, rect(PAD_X, y + 9.0, ICON_COL, ROW_ICON)));
            let b = rect(FIELD_X, y, right - FIELD_X, NOTES_H);
            l.notes_box = Some(b);
            l.items.push((Item::Notes, b));
            let (px, py) = NOTES_PAD;
            l.edits.push((
                Item::Notes,
                rect(b.x + px, b.y + py, b.w - 2.0 * px, b.h - 2.0 * py),
            ));
            y += NOTES_H;
        } else {
            y += ROW_H + GAP;
            icon(&mut l, Icon::Clock, y);
            let tw = chip_w(measure(labels.time, Text::Body), false);
            l.items.push((Item::TimeChip, rect(FIELD_X, y, tw, ROW_H)));
            y += ROW_H + GAP;
            icon(&mut l, Icon::Repeat, y);
            let track = rect(FIELD_X, y, 3.0 * SEG_W + 2.0 * SEG_PAD, ROW_H);
            l.repeat_track = Some(track);
            for (i, r) in [Repeat::Once, Repeat::Daily, Repeat::Weekly]
                .into_iter()
                .enumerate()
            {
                let seg = rect(
                    track.x + SEG_PAD + i as f32 * SEG_W,
                    y + SEG_PAD,
                    SEG_W,
                    ROW_H - 2.0 * SEG_PAD,
                );
                l.items.push((Item::Repeat(r), seg));
            }
            y += ROW_H;
            if spec.error {
                l.error = Some(rect(FIELD_X, y + GAP - ERR_PULL, right - FIELD_X, ERR_H));
                y += GAP - ERR_PULL + ERR_H;
            }
        }
    }

    // The footer.
    y += BODY_BOTTOM;
    l.footer_rule = rect(0.0, y, w, 1.0);
    let by = y + 12.0;
    let button = |text: &str, style: Text| measure(text, style).ceil() + 28.0;
    let mut left_of_buttons = right;
    let mut place_right = |l: &mut Layout, item: Item, bw: f32| {
        let r = rect(left_of_buttons - bw, by, bw, BUTTON_H);
        left_of_buttons = r.x - 8.0;
        l.items.push((item, r));
    };
    // Pushed right to left, then put back in reading order for Tab.
    let first_button = l.items.len();
    match spec.form {
        Form::Note => place_right(&mut l, Item::Save, button(labels.save, Text::ButtonStrong)),
        Form::Event if spec.read_only => {
            place_right(&mut l, Item::Cancel, button(labels.cancel, Text::Button))
        }
        _ => {
            place_right(&mut l, Item::Save, button(labels.save, Text::ButtonStrong));
            place_right(&mut l, Item::Cancel, button(labels.cancel, Text::Button));
        }
    }
    let mut status_x = PAD_X;
    if spec.delete && spec.form != Form::Event {
        let dw = button(labels.delete, Text::Button);
        l.items
            .insert(first_button, (Item::Delete, rect(PAD_X, by, dw, BUTTON_H)));
        status_x = PAD_X + dw + 10.0;
    }
    l.items[first_button..].sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    if spec.form == Form::Event {
        l.dot = Some(rect(PAD_X, by + (BUTTON_H - 8.0) / 2.0, 8.0, 8.0));
        status_x = PAD_X + 14.0;
    }
    l.status = rect(
        status_x,
        by,
        (left_of_buttons - status_x).max(0.0),
        BUTTON_H,
    );
    l.size = (w, y + FOOTER_H);
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today's shortcut dates, in pill order.
    fn shortcut_dates(today: NaiveDate) -> [(Shortcut, NaiveDate); 4] {
        Shortcut::ALL.map(|s| (s, s.date(today)))
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    /// A generous estimate: 0.62 em per character.
    fn estimate(s: &str, t: Text) -> f32 {
        s.chars().count() as f32 * t.size() * 0.62
    }

    fn labels() -> Labels<'static> {
        Labels {
            date: "Rab, 28 Okt 2026",
            start: "10.00",
            end: "11.30",
            time: "08.00",
            pills: ["Hari ini", "Besok", "Rabu", "Minggu depan"],
            all_day: "Seharian",
            delete: "Hapus",
            cancel: "Batal",
            save: "Simpan",
        }
    }

    fn spec(form: Form) -> Spec<'static> {
        Spec {
            form,
            read_only: false,
            all_day: false,
            error: false,
            delete: true,
            labels: labels(),
        }
    }

    fn inside(r: Rect, w: f32, h: f32) -> bool {
        r.x >= 0.0 && r.y >= 0.0 && r.right() <= w + 0.01 && r.bottom() <= h + 0.01
    }

    fn check(l: &Layout, what: &str) {
        let (w, h) = l.size;
        let mut all: Vec<(String, Rect)> = Vec::new();
        all.extend(l.items.iter().map(|(i, r)| (format!("{i:?}"), *r)));
        all.extend(l.edits.iter().map(|(i, r)| (format!("edit {i:?}"), *r)));
        all.extend(l.icons.iter().map(|(i, r)| (format!("icon {i:?}"), *r)));
        for (name, r) in [
            ("heading", Some(l.heading)),
            ("rule", l.title_rule),
            ("notes", l.notes_box),
            ("label", l.all_day_label),
            ("toggle", l.toggle),
            ("arrow", l.arrow),
            ("duration", l.duration),
            ("track", l.repeat_track),
            ("error", l.error),
            ("footer", Some(l.footer_rule)),
            ("status", Some(l.status)),
        ] {
            if let Some(r) = r {
                all.push((name.into(), r));
            }
        }
        for (name, r) in &all {
            assert!(r.w >= 0.0 && r.h > 0.0, "{what}: {name} empty");
            assert!(inside(*r, w, h), "{what}: {name} {r:?} outside {w}x{h}");
        }
        // Items never overlap each other (the all-day row and its pills are
        // separate rows; the title and note boxes hold their EDITs).
        let items: Vec<&(Item, Rect)> = l.items.iter().collect();
        for (i, (a, ra)) in items.iter().enumerate() {
            for (b, rb) in &items[i + 1..] {
                let apart = ra.right() <= rb.x
                    || rb.right() <= ra.x
                    || ra.bottom() <= rb.y
                    || rb.bottom() <= ra.y;
                assert!(apart, "{what}: {a:?} overlaps {b:?}");
            }
        }
        // Each EDIT sits inside its item's box.
        for (item, e) in &l.edits {
            let b = l.item(*item).expect("edit item");
            assert!(e.x >= b.x && e.right() <= b.right() && e.y >= b.y && e.bottom() <= b.bottom());
        }
        // The footer sits below everything else in the body.
        for (name, r) in &all {
            if !matches!(name.as_str(), "footer" | "status")
                && !name.starts_with("Save")
                && !name.starts_with("Cancel")
                && !name.starts_with("Delete")
            {
                assert!(
                    r.bottom() <= l.footer_rule.y + 0.01,
                    "{what}: {name} under the footer"
                );
            }
        }
    }

    #[test]
    fn every_form_stays_inside_its_window() {
        for form in [Form::Note, Form::Event, Form::Reminder] {
            for all_day in [false, true] {
                for error in [false, true] {
                    for delete in [false, true] {
                        let s = Spec {
                            all_day,
                            error,
                            delete,
                            ..spec(form)
                        };
                        check(
                            &layout(&s, &estimate),
                            &format!("{form:?} {all_day} {error} {delete}"),
                        );
                    }
                }
            }
        }
        let ro = Spec {
            read_only: true,
            labels: Labels {
                cancel: "Tutup",
                ..labels()
            },
            ..spec(Form::Event)
        };
        let l = layout(&ro, &estimate);
        check(&l, "read-only");
        assert!(l.item(Item::Save).is_none());
        assert!(l.item(Item::Cancel).is_some());
    }

    #[test]
    fn all_day_hides_the_time_row_and_shrinks_the_window() {
        let timed = layout(&spec(Form::Event), &estimate);
        let all_day = layout(
            &Spec {
                all_day: true,
                ..spec(Form::Event)
            },
            &estimate,
        );
        assert!(timed.item(Item::StartChip).is_some());
        assert!(all_day.item(Item::StartChip).is_none());
        assert!(all_day.item(Item::EndChip).is_none());
        assert_eq!(timed.size.1 - all_day.size.1, ROW_H + GAP);
        // The design's card: about 360 px tall at 400 wide.
        assert_eq!(timed.size, (400.0, 361.0));
        let bad = layout(
            &Spec {
                error: true,
                ..spec(Form::Event)
            },
            &estimate,
        );
        let err = bad.error.expect("error line");
        let end = bad.item(Item::EndChip).unwrap();
        assert!(err.y >= end.bottom() && err.bottom() <= bad.notes_box.unwrap().y);
    }

    #[test]
    fn tab_order_follows_the_design() {
        let l = layout(&spec(Form::Event), &estimate);
        assert_eq!(
            l.focus_order(),
            [
                Item::Title,
                Item::DateChip,
                Item::Pill(Shortcut::Today),
                Item::Pill(Shortcut::Tomorrow),
                Item::Pill(Shortcut::DayAfter),
                Item::Pill(Shortcut::NextWeek),
                Item::AllDay,
                Item::StartChip,
                Item::EndChip,
                Item::Notes,
                Item::Cancel,
                Item::Save,
            ]
        );
        let r = layout(&spec(Form::Reminder), &estimate);
        assert_eq!(
            r.focus_order(),
            [
                Item::Title,
                Item::DateChip,
                Item::TimeChip,
                Item::Repeat(Repeat::Once),
                Item::Repeat(Repeat::Daily),
                Item::Repeat(Repeat::Weekly),
                Item::Delete,
                Item::Cancel,
                Item::Save,
            ]
        );
        let n = layout(&spec(Form::Note), &estimate);
        assert_eq!(n.focus_order(), [Item::Notes, Item::Delete, Item::Save]);
        let new_note = layout(
            &Spec {
                delete: false,
                ..spec(Form::Note)
            },
            &estimate,
        );
        assert_eq!(new_note.focus_order(), [Item::Notes, Item::Save]);
    }

    #[test]
    fn header_drags_except_the_close_button() {
        let l = layout(&spec(Form::Event), &estimate);
        let close = l.item(Item::Close).unwrap();
        assert!(l.is_caption(40.0, 20.0));
        assert!(!l.is_caption(close.x + 5.0, close.y + 5.0));
        assert!(!l.is_caption(40.0, HEADER_H + 1.0));
        assert_eq!(l.hit(close.x + 5.0, close.y + 5.0), Some(Item::Close));
    }

    #[test]
    fn shortcut_dates_are_today_tomorrow_the_next_weekday_and_a_week_on() {
        let mon = ymd(2026, 10, 26);
        assert_eq!(
            shortcut_dates(mon),
            [
                (Shortcut::Today, mon),
                (Shortcut::Tomorrow, ymd(2026, 10, 27)),
                (Shortcut::DayAfter, ymd(2026, 10, 28)),
                (Shortcut::NextWeek, ymd(2026, 11, 2)),
            ]
        );
        // Across a year end.
        let eve = ymd(2026, 12, 31);
        assert_eq!(Shortcut::Tomorrow.date(eve), ymd(2027, 1, 1));
        assert_eq!(Shortcut::NextWeek.date(eve), ymd(2027, 1, 7));
        assert_eq!(
            Shortcut::matching(mon, ymd(2026, 10, 28)),
            Some(Shortcut::DayAfter)
        );
        assert_eq!(Shortcut::matching(mon, ymd(2026, 10, 29)), None);
    }
}
