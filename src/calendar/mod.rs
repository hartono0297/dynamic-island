//! Calendar data shared by the sync thread, the reducer and the renderer:
//! events in local time, the snapshot the sync thread sends, and date ranges.

pub mod api;
pub mod sync;

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::model::CalView;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventTime {
    AllDay(NaiveDate),
    At(DateTime<Local>),
}

impl EventTime {
    /// Local wall-clock time; an all-day date counts from its midnight.
    fn naive_local(&self) -> NaiveDateTime {
        match self {
            EventTime::AllDay(d) => d.and_time(NaiveTime::MIN),
            EventTime::At(t) => t.naive_local(),
        }
    }

    pub fn date(&self) -> NaiveDate {
        match self {
            EventTime::AllDay(d) => *d,
            EventTime::At(t) => t.date_naive(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalEvent {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub start: EventTime,
    /// Exclusive, as Google sends it: an all-day event on the 26th ends on the 27th.
    pub end: EventTime,
    pub description: String,
    /// `0xRRGGBB` of the calendar, if Google gave one.
    pub color: Option<u32>,
    /// False for read-only calendars and for events an `EventDraft` cannot hold
    /// (spanning several days), so saving a draft never shortens them.
    pub editable: bool,
}

impl CalEvent {
    pub fn all_day(&self) -> bool {
        matches!(self.start, EventTime::AllDay(_))
    }
}

/// What the event editor edits: one title, one date, and either all-day or a
/// start/end time on that date.
#[derive(Clone, Debug, PartialEq)]
pub struct EventDraft {
    pub title: String,
    pub date: NaiveDate,
    pub all_day: bool,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub description: String,
}

/// Times shown for an all-day event if the user unticks "Seharian".
const DEFAULT_START: NaiveTime = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
const DEFAULT_END: NaiveTime = NaiveTime::from_hms_opt(10, 0, 0).unwrap();

pub fn draft_from(ev: &CalEvent) -> EventDraft {
    let (all_day, start, end) = match (&ev.start, &ev.end) {
        (EventTime::At(s), EventTime::At(e)) => (false, s.time(), e.time()),
        _ => (true, DEFAULT_START, DEFAULT_END),
    };
    EventDraft {
        title: ev.title.clone(),
        date: ev.start.date(),
        all_day,
        start,
        end,
        description: ev.description.clone(),
    }
}

/// The editor's starting point for a new event on `date`: timed, 09:00–10:00.
pub fn new_draft(date: NaiveDate) -> EventDraft {
    EventDraft {
        title: String::new(),
        date,
        all_day: false,
        start: DEFAULT_START,
        end: DEFAULT_END,
        description: String::new(),
    }
}

/// True when the event fits a draft: all-day on exactly one date, or timed
/// with start and end on the same local date.
pub fn fits_draft(start: &EventTime, end: &EventTime) -> bool {
    match (start, end) {
        (EventTime::AllDay(s), EventTime::AllDay(e)) => s.succ_opt() == Some(*e),
        (EventTime::At(s), EventTime::At(e)) => s.date_naive() == e.date_naive() && s <= e,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalStatus {
    /// No Google client ID in the config.
    NotConfigured,
    /// Never connected (or the consent screen was cancelled): "Hubungkan Google".
    NeedsLogin,
    /// The refresh token was revoked or expired: "Hubungkan ulang Google".
    Reconnect,
    Ok,
    /// The last fetch failed; `events` are from the cache or an earlier fetch.
    Offline,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarSnapshot {
    pub status: CalStatus,
    pub events: Vec<CalEvent>,
    /// The fetched dates `[from, to)`.
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// When `events` came from Google; `None` if they never did.
    pub fetched_at: Option<DateTime<Local>>,
}

impl CalendarSnapshot {
    pub fn empty(status: CalStatus, today: NaiveDate) -> Self {
        CalendarSnapshot {
            status,
            events: Vec::new(),
            from: today,
            to: today,
            fetched_at: None,
        }
    }
}

/// Dates `[from, to)` a calendar view shows around `anchor`: the day itself,
/// its Monday-to-Monday week, or the 6-week Monday-first month grid.
pub fn range_for(view: CalView, anchor: NaiveDate) -> (NaiveDate, NaiveDate) {
    let monday_of = |d: NaiveDate| {
        d.checked_sub_days(Days::new(u64::from(d.weekday().num_days_from_monday())))
            .unwrap_or(d)
    };
    let (from, days) = match view {
        CalView::Day => (anchor, 1),
        CalView::Week => (monday_of(anchor), 7),
        CalView::Month => (monday_of(anchor.with_day(1).unwrap_or(anchor)), 42),
    };
    (from, from.checked_add_days(Days::new(days)).unwrap_or(from))
}

/// Events touching the local `date`: all-day ones first, then by start time.
/// Ends are exclusive; a zero-length event counts on the date it starts.
pub fn events_on(events: &[CalEvent], date: NaiveDate) -> Vec<&CalEvent> {
    let day_start = date.and_time(NaiveTime::MIN);
    let Some(day_end) = date.succ_opt().map(|d| d.and_time(NaiveTime::MIN)) else {
        return Vec::new();
    };
    let mut on: Vec<&CalEvent> = events
        .iter()
        .filter(|ev| {
            let (start, end) = (ev.start.naive_local(), ev.end.naive_local());
            start < day_end && (end > day_start || (end <= start && start >= day_start))
        })
        .collect();
    on.sort_by_key(|ev| (!ev.all_day(), ev.start.naive_local()));
    on
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(&date(y, mo, d).and_hms_opt(h, mi, 0).unwrap())
            .earliest()
            .unwrap()
    }

    fn event(id: &str, start: EventTime, end: EventTime) -> CalEvent {
        CalEvent {
            id: id.into(),
            calendar_id: "primary".into(),
            title: id.into(),
            start,
            end,
            description: String::new(),
            color: None,
            editable: true,
        }
    }

    fn ids<'a>(events: &[&'a CalEvent]) -> Vec<&'a str> {
        events.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn range_month_starts_monday_before_first() {
        // Oct 1 2026 is a Thursday; 42 days from Monday Sep 28.
        assert_eq!(
            range_for(CalView::Month, date(2026, 10, 25)),
            (date(2026, 9, 28), date(2026, 11, 9))
        );
        // A month starting on a Monday starts on the 1st (Jun 1 2026).
        assert_eq!(
            range_for(CalView::Month, date(2026, 6, 30)),
            (date(2026, 6, 1), date(2026, 7, 13))
        );
    }

    #[test]
    fn range_week_monday_to_monday() {
        // Sunday Oct 25 belongs to the week of Monday Oct 19.
        assert_eq!(
            range_for(CalView::Week, date(2026, 10, 25)),
            (date(2026, 10, 19), date(2026, 10, 26))
        );
        assert_eq!(
            range_for(CalView::Week, date(2026, 10, 26)),
            (date(2026, 10, 26), date(2026, 11, 2))
        );
    }

    #[test]
    fn range_day() {
        assert_eq!(
            range_for(CalView::Day, date(2026, 12, 31)),
            (date(2026, 12, 31), date(2027, 1, 1))
        );
    }

    #[test]
    fn multi_day_all_day_covers_each_date() {
        let evs = [event(
            "trip",
            EventTime::AllDay(date(2026, 10, 26)),
            EventTime::AllDay(date(2026, 10, 29)),
        )];
        for d in [26, 27, 28] {
            assert_eq!(ids(&events_on(&evs, date(2026, 10, d))), ["trip"], "{d}");
        }
        assert!(events_on(&evs, date(2026, 10, 25)).is_empty());
        assert!(
            events_on(&evs, date(2026, 10, 29)).is_empty(),
            "end exclusive"
        );
    }

    #[test]
    fn timed_event_crossing_midnight_on_both_days() {
        let evs = [event(
            "late",
            EventTime::At(local(2026, 10, 25, 23, 0)),
            EventTime::At(local(2026, 10, 26, 1, 0)),
        )];
        assert_eq!(ids(&events_on(&evs, date(2026, 10, 25))), ["late"]);
        assert_eq!(ids(&events_on(&evs, date(2026, 10, 26))), ["late"]);
        assert!(events_on(&evs, date(2026, 10, 24)).is_empty());
        assert!(events_on(&evs, date(2026, 10, 27)).is_empty());

        // Ending exactly at midnight does not touch the next day.
        let evs = [event(
            "eve",
            EventTime::At(local(2026, 10, 25, 22, 0)),
            EventTime::At(local(2026, 10, 26, 0, 0)),
        )];
        assert!(events_on(&evs, date(2026, 10, 26)).is_empty());
    }

    #[test]
    fn zero_length_event_counts_on_its_date() {
        let t = local(2026, 10, 25, 0, 0);
        let evs = [event("ping", EventTime::At(t), EventTime::At(t))];
        assert_eq!(ids(&events_on(&evs, date(2026, 10, 25))), ["ping"]);
        assert!(events_on(&evs, date(2026, 10, 24)).is_empty());
    }

    #[test]
    fn events_on_orders_all_day_first() {
        let evs = [
            event(
                "ten",
                EventTime::At(local(2026, 10, 25, 10, 0)),
                EventTime::At(local(2026, 10, 25, 11, 0)),
            ),
            event(
                "eight",
                EventTime::At(local(2026, 10, 25, 8, 0)),
                EventTime::At(local(2026, 10, 25, 9, 0)),
            ),
            event(
                "holiday",
                EventTime::AllDay(date(2026, 10, 25)),
                EventTime::AllDay(date(2026, 10, 26)),
            ),
            event(
                "other-day",
                EventTime::At(local(2026, 10, 26, 8, 0)),
                EventTime::At(local(2026, 10, 26, 9, 0)),
            ),
        ];
        assert_eq!(
            ids(&events_on(&evs, date(2026, 10, 25))),
            ["holiday", "eight", "ten"]
        );
    }

    #[test]
    fn draft_from_timed_and_all_day() {
        let timed = event(
            "t",
            EventTime::At(local(2026, 10, 25, 13, 30)),
            EventTime::At(local(2026, 10, 25, 14, 15)),
        );
        let d = draft_from(&timed);
        assert_eq!(d.date, date(2026, 10, 25));
        assert!(!d.all_day);
        assert_eq!(d.start, NaiveTime::from_hms_opt(13, 30, 0).unwrap());
        assert_eq!(d.end, NaiveTime::from_hms_opt(14, 15, 0).unwrap());
        assert_eq!(d.title, "t");

        let all_day = event(
            "a",
            EventTime::AllDay(date(2026, 10, 26)),
            EventTime::AllDay(date(2026, 10, 27)),
        );
        let d = draft_from(&all_day);
        assert!(d.all_day);
        assert_eq!(d.date, date(2026, 10, 26));
    }

    #[test]
    fn new_draft_is_a_timed_hour_on_the_date() {
        let d = new_draft(date(2026, 10, 25));
        assert_eq!(d.date, date(2026, 10, 25));
        assert!(!d.all_day && d.title.is_empty() && d.description.is_empty());
        assert_eq!((d.start, d.end), (DEFAULT_START, DEFAULT_END));
        assert!(d.start < d.end);
    }

    #[test]
    fn only_single_day_events_fit_a_draft() {
        let ad = |a, b| {
            (
                EventTime::AllDay(date(2026, 10, a)),
                EventTime::AllDay(date(2026, 10, b)),
            )
        };
        let (s, e) = ad(26, 27);
        assert!(fits_draft(&s, &e));
        let (s, e) = ad(26, 29);
        assert!(!fits_draft(&s, &e));
        let s = EventTime::At(local(2026, 10, 25, 23, 0));
        let e = EventTime::At(local(2026, 10, 26, 1, 0));
        assert!(!fits_draft(&s, &e));
        let e = EventTime::At(local(2026, 10, 25, 23, 30));
        assert!(fits_draft(&s, &e));
    }
}
