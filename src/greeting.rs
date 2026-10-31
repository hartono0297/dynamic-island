//! The cat's greeting follows the local time of day: five periods, a face
//! for each, and the mark `config.last_greeting` keeps so a greeting shows
//! at most once per period per calendar day. Pure; `app.rs` feeds it the
//! clock and `i18n` words the title and the body.

use chrono::{NaiveDate, NaiveTime, Timelike};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ui::robot::Expr;

/// A part of the local day. Night is 00:00-03:59: the user is still up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DayPeriod {
    Morning,
    Midday,
    Afternoon,
    Evening,
    Night,
}

impl DayPeriod {
    /// The id written in the config file.
    pub fn id(self) -> &'static str {
        match self {
            DayPeriod::Morning => "morning",
            DayPeriod::Midday => "midday",
            DayPeriod::Afternoon => "afternoon",
            DayPeriod::Evening => "evening",
            DayPeriod::Night => "night",
        }
    }

    /// Every period, in the order Settings lists them.
    pub const ALL: [DayPeriod; 5] = [
        DayPeriod::Morning,
        DayPeriod::Midday,
        DayPeriod::Afternoon,
        DayPeriod::Evening,
        DayPeriod::Night,
    ];

    /// The first and last hour (0-23) that falls in this period; the same
    /// ranges `period_at` matches on.
    pub fn hours(self) -> (u32, u32) {
        match self {
            DayPeriod::Morning => (4, 10),
            DayPeriod::Midday => (11, 14),
            DayPeriod::Afternoon => (15, 17),
            DayPeriod::Evening => (18, 23),
            DayPeriod::Night => (0, 3),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "morning" => DayPeriod::Morning,
            "midday" => DayPeriod::Midday,
            "afternoon" => DayPeriod::Afternoon,
            "evening" => DayPeriod::Evening,
            "night" => DayPeriod::Night,
            _ => return None,
        })
    }
}

/// The period `t` falls in: Morning 04:00-10:59, Midday 11:00-14:59,
/// Afternoon 15:00-17:59, Evening 18:00-23:59, Night 00:00-03:59.
pub fn period_at(t: NaiveTime) -> DayPeriod {
    match t.hour() {
        4..=10 => DayPeriod::Morning,
        11..=14 => DayPeriod::Midday,
        15..=17 => DayPeriod::Afternoon,
        18..=23 => DayPeriod::Evening,
        _ => DayPeriod::Night,
    }
}

/// The cat's face for a greeting in `period`.
pub fn face(period: DayPeriod) -> Expr {
    match period {
        DayPeriod::Morning | DayPeriod::Afternoon => Expr::Wave,
        DayPeriod::Midday | DayPeriod::Evening => Expr::Happy,
        DayPeriod::Night => Expr::Sleep,
    }
}

/// What the body of a greeting counts. Evening and Night use `events` only;
/// what the numbers stand for depends on the period (see `app::greeting_counts`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GreetCounts {
    pub events: usize,
    pub reminders: usize,
}

/// The last greeting shown: its day and period. In the config file one
/// string, "2026-10-31 evening".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GreetMark {
    pub date: NaiveDate,
    pub period: DayPeriod,
}

impl GreetMark {
    /// "2026-10-31 evening". A plain old "2026-10-27" reads as that date's
    /// Morning (the old once-a-day greeting was a morning one). Anything else
    /// is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split_whitespace();
        let date = NaiveDate::parse_from_str(parts.next()?, "%Y-%m-%d").ok()?;
        let period = match parts.next() {
            None => DayPeriod::Morning,
            Some(id) => DayPeriod::from_id(id)?,
        };
        parts.next().is_none().then_some(GreetMark { date, period })
    }

    pub fn text(self) -> String {
        format!("{} {}", self.date.format("%Y-%m-%d"), self.period.id())
    }
}

impl Serialize for GreetMark {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

/// For `config.last_greeting`: an unreadable value (bad date, unknown
/// period, a number, null) is `None`, so a hand edit never invalidates the
/// rest of the file.
pub fn lenient_mark<'de, D: Deserializer<'de>>(d: D) -> Result<Option<GreetMark>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_str().and_then(GreetMark::parse))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn every_hour_falls_in_the_period_whose_range_holds_it() {
        use DayPeriod::*;
        assert_eq!(
            DayPeriod::ALL,
            [Morning, Midday, Afternoon, Evening, Night]
        );
        for h in 0..=23u32 {
            let owners: Vec<DayPeriod> = DayPeriod::ALL
                .into_iter()
                .filter(|p| {
                    let (from, to) = p.hours();
                    (from..=to).contains(&h)
                })
                .collect();
            for m in [0, 59] {
                assert_eq!(owners, [period_at(at(h, m))], "hour {h}");
            }
        }
    }

    #[test]
    fn periods_change_on_their_boundaries() {
        use DayPeriod::*;
        for (h, m, want) in [
            (0, 0, Night),
            (3, 59, Night),
            (4, 0, Morning),
            (10, 59, Morning),
            (11, 0, Midday),
            (14, 59, Midday),
            (15, 0, Afternoon),
            (17, 59, Afternoon),
            (18, 0, Evening),
            (23, 59, Evening),
        ] {
            assert_eq!(period_at(at(h, m)), want, "{h}:{m:02}");
        }
    }

    #[test]
    fn every_period_has_its_face() {
        use DayPeriod::*;
        for (p, e) in [
            (Morning, Expr::Wave),
            (Midday, Expr::Happy),
            (Afternoon, Expr::Wave),
            (Evening, Expr::Happy),
            (Night, Expr::Sleep),
        ] {
            assert_eq!(face(p), e, "{p:?}");
        }
    }

    #[test]
    fn mark_is_one_string_with_the_period() {
        let mark = GreetMark {
            date: day(31),
            period: DayPeriod::Evening,
        };
        assert_eq!(
            serde_json::to_string(&mark).unwrap(),
            r#""2026-10-31 evening""#
        );
        for p in [
            DayPeriod::Morning,
            DayPeriod::Midday,
            DayPeriod::Afternoon,
            DayPeriod::Evening,
            DayPeriod::Night,
        ] {
            let m = GreetMark {
                date: day(5),
                period: p,
            };
            assert_eq!(GreetMark::parse(&m.text()), Some(m), "{p:?}");
        }
    }

    #[test]
    fn old_plain_date_reads_as_that_mornings_greeting() {
        assert_eq!(
            GreetMark::parse("2026-10-27"),
            Some(GreetMark {
                date: day(27),
                period: DayPeriod::Morning
            })
        );
    }

    #[test]
    fn unreadable_marks_read_as_none() {
        for bad in [
            "kemarin",
            "",
            "2026-13-40 morning",
            "2026-10-31 lunch",
            "2026-10-31 evening extra",
            "morning",
        ] {
            assert_eq!(GreetMark::parse(bad), None, "{bad:?}");
        }
        #[derive(Deserialize)]
        struct F {
            #[serde(default, deserialize_with = "lenient_mark")]
            m: Option<GreetMark>,
            other: u32,
        }
        for bad in [
            "7",
            "null",
            "true",
            r#""kemarin""#,
            r#""2026-10-31 lunch""#,
            "[]",
        ] {
            let f: F = serde_json::from_str(&format!(r#"{{"m":{bad},"other":3}}"#)).unwrap();
            assert_eq!((f.m, f.other), (None, 3), "{bad}");
        }
        let f: F = serde_json::from_str(r#"{"m":"2026-10-31 night","other":3}"#).unwrap();
        assert_eq!(
            f.m,
            Some(GreetMark {
                date: day(31),
                period: DayPeriod::Night
            })
        );
    }
}
