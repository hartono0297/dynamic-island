//! Reminders kept in a local JSON file, and the time arithmetic behind them:
//! which are due, when the next one is, and when a repeating one comes back.
//! Times are Unix seconds; repeats keep the local wall-clock time across
//! offset changes (DST), so a daily 08:00 stays 08:00. A weekly reminder may
//! ring on several weekdays (`Reminder::days`); `next_repeat` is the one rule
//! for when a repeating reminder comes next, shared with the editor's summary.

use chrono::{
    DateTime, Datelike, Days, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use crate::config::backup_path;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    #[serde(alias = "none")]
    Once,
    Daily,
    Weekly,
}

impl Repeat {
    /// Days between occurrences; `None` for a one-off.
    fn step_days(self) -> Option<u64> {
        match self {
            Repeat::Once => None,
            Repeat::Daily => Some(1),
            Repeat::Weekly => Some(7),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    /// Unix seconds of the (next) occurrence.
    pub due_unix: i64,
    #[serde(default)]
    pub repeat: Repeat,
    /// A one-off that was dismissed. Repeating reminders never get done; they
    /// move on to their next occurrence instead.
    #[serde(default)]
    pub done: bool,
    /// Only for `Weekly`: the weekdays it rings on, Monday first, each once.
    /// Empty means the weekday of the due date, as before the field existed
    /// (and always for `Once` and `Daily`), so such a reminder saves without
    /// the key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<Weekday>,
}

/// Done reminders are dropped once they are this old (seconds past due).
const KEEP_DONE_SECS: i64 = 30 * 24 * 3600;

/// A reminder as the file stores it: the reminder plus the local wall-clock
/// time it rings at, which `due_unix` alone cannot tell when an occurrence
/// fell in a skipped DST hour (a daily 02:30 rings at 03:30 that day, and at
/// 02:30 again the next). Files written before the field existed fall back to
/// the wall time of `due_unix`.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Stored {
    #[serde(flatten)]
    reminder: Reminder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time: Option<NaiveTime>,
}

/// Reminders in a local JSON file. Changes stay in memory until `save`.
pub struct RemindersStore {
    path: PathBuf,
    /// Not done first, by due time (then id); done ones after, the same way.
    items: Vec<Reminder>,
    /// Each reminder's local wall-clock time, by id (see `Stored`).
    wall: HashMap<String, NaiveTime>,
    counter: u64,
    /// The file existed but could not be read or parsed. It is moved aside
    /// (`reminders.json.bak`) before the first save instead of being overwritten.
    keep_old: bool,
}

/// The local wall-clock time of `unix` in `tz`.
fn wall_of<Tz: TimeZone>(tz: &Tz, unix: i64) -> Option<NaiveTime> {
    tz.timestamp_opt(unix, 0)
        .single()
        .map(|t| t.naive_local().time())
}

/// The reminders in a file read as `read`, and whether the file must be kept
/// aside: it exists but is unreadable or not a reminders list. A missing file
/// is simply empty.
fn load(read: io::Result<String>) -> (Vec<Stored>, bool) {
    match read {
        Ok(text) => {
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            match serde_json::from_str(text) {
                Ok(items) => (items, false),
                Err(_) => (Vec::new(), true),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Vec::new(), false),
        Err(_) => (Vec::new(), true),
    }
}

impl RemindersStore {
    /// A missing, unreadable or malformed file yields an empty store. The file
    /// itself is left untouched until the next `save`, which first renames an
    /// unreadable or malformed one to `reminders.json.bak`.
    pub fn open(path: PathBuf) -> Self {
        let (stored, keep_old) = load(std::fs::read_to_string(&path));
        let mut wall = HashMap::new();
        let mut items = Vec::with_capacity(stored.len());
        for Stored { reminder, time } in stored {
            let time = time.or_else(|| wall_of(&chrono::Local, reminder.due_unix));
            if let Some(time) = time {
                wall.insert(reminder.id.clone(), time);
            }
            items.push(reminder);
        }
        let mut store = RemindersStore {
            path,
            items,
            wall,
            counter: 0,
            keep_old,
        };
        store.sort();
        store
    }

    /// `%APPDATA%\Vale\reminders.json`, next to the config file.
    pub fn default_path() -> PathBuf {
        crate::config::path().with_file_name("reminders.json")
    }

    /// True while an existing reminders file that could not be read or parsed
    /// waits to be moved aside by the next `save`.
    pub fn load_failed(&self) -> bool {
        self.keep_old
    }

    /// Every reminder, the upcoming ones first.
    pub fn list(&self) -> &[Reminder] {
        &self.items
    }

    /// Reminders not done yet (including ones already due), earliest first.
    pub fn upcoming(&self) -> &[Reminder] {
        &self.items[..self.items.partition_point(|r| !r.done)]
    }

    pub fn get(&self, id: &str) -> Option<&Reminder> {
        self.items.iter().find(|r| r.id == id)
    }

    /// Creates or edits a reminder due at `due_unix` and returns its id; an
    /// edit is not done any more. It rings at the wall-clock time of
    /// `due_unix` (prefer `upsert_local`, which also knows the intended time
    /// when it falls in a skipped DST hour). Text that is empty after trimming
    /// creates or changes nothing and returns `None`. An unknown `id` is
    /// treated as a new reminder. `days` as in `Reminder::days`; it is kept
    /// only for `Weekly`.
    #[cfg_attr(not(test), allow(dead_code))] // The island saves through `upsert_local`.
    pub fn upsert(
        &mut self,
        id: Option<&str>,
        text: &str,
        due_unix: i64,
        repeat: Repeat,
        days: &[Weekday],
        now: i64,
    ) -> Option<String> {
        let wall = wall_of(&chrono::Local, due_unix);
        self.put(id, text, due_unix, wall, repeat, days, now)
    }

    /// `upsert` for a reminder set to local `date` and `time`: a repeating one
    /// starts at its first occurrence from `date` that is still to come
    /// (`first_due_at`; a weekly one on one of its `days`), and the intended
    /// `time` is kept even when that day skips it. `None` also when the
    /// local time cannot be worked out.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_local(
        &mut self,
        id: Option<&str>,
        text: &str,
        date: NaiveDate,
        time: NaiveTime,
        repeat: Repeat,
        days: &[Weekday],
        now: i64,
    ) -> Option<String> {
        let tz = chrono::Local;
        let due = at_local(&tz, date.and_time(time))?;
        let due = first_due_at(&tz, due, Some(time), repeat, days, now);
        self.put(id, text, due, Some(time), repeat, days, now)
    }

    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        id: Option<&str>,
        text: &str,
        due_unix: i64,
        wall: Option<NaiveTime>,
        repeat: Repeat,
        days: &[Weekday],
        now: i64,
    ) -> Option<String> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let due_day = chrono::Local
            .timestamp_opt(due_unix, 0)
            .single()
            .map(|t| t.date_naive().weekday());
        let days = stored_days(repeat, days, due_day);
        let id = match id.and_then(|id| self.items.iter_mut().find(|r| r.id == id)) {
            Some(r) => {
                r.text = text.to_owned();
                r.due_unix = due_unix;
                r.repeat = repeat;
                r.done = false;
                r.days = days;
                r.id.clone()
            }
            None => {
                let id = self.next_id(now);
                self.items.push(Reminder {
                    id: id.clone(),
                    text: text.to_owned(),
                    due_unix,
                    repeat,
                    done: false,
                    days,
                });
                id
            }
        };
        match wall {
            Some(time) => self.wall.insert(id.clone(), time),
            None => self.wall.remove(&id),
        };
        self.sort();
        Some(id)
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|r| r.id != id);
        self.wall.remove(id);
        self.items.len() != before
    }

    /// The local wall-clock time `id` rings at (`None` for one saved without
    /// it, by an older version).
    pub fn wall_time(&self, id: &str) -> Option<NaiveTime> {
        self.wall.get(id).copied()
    }

    /// The reminder was dismissed at `now`: a one-off is done, a repeating one
    /// moves to its next occurrence after `now`. Done ones older than a month
    /// are dropped on the way. False when `id` is unknown or already done.
    pub fn mark_done_or_advance(&mut self, id: &str, now: i64) -> bool {
        let Some(r) = self.items.iter_mut().find(|r| r.id == id && !r.done) else {
            return false;
        };
        // From the intended wall time, not the one `due_unix` shows: an
        // occurrence moved by a skipped hour must not move the next ones.
        let wall = self.wall.get(id).copied();
        match advance_in(&chrono::Local, r.due_unix, wall, r.repeat, &r.days, now) {
            Some(next) => r.due_unix = next,
            None => r.done = true,
        }
        self.items
            .retain(|r| !r.done || r.due_unix > now - KEEP_DONE_SECS);
        let items = &self.items;
        self.wall.retain(|id, _| items.iter().any(|r| &r.id == id));
        self.sort();
        true
    }

    /// Writes pretty JSON to a sibling `.tmp` file, then renames it over the
    /// real file so a crash mid-write never leaves a truncated store. A file
    /// that could not be loaded is renamed to a backup first; if that fails,
    /// nothing is written.
    pub fn save(&mut self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.keep_old {
            match std::fs::rename(&self.path, backup_path(&self.path)) {
                Ok(()) => {}
                // Gone meanwhile: nothing left to keep.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            self.keep_old = false;
        }
        let stored: Vec<Stored> = self
            .items
            .iter()
            .map(|r| Stored {
                reminder: r.clone(),
                time: self.wall.get(&r.id).copied(),
            })
            .collect();
        let json = serde_json::to_string_pretty(&stored).map_err(io::Error::other)?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }

    fn sort(&mut self) {
        self.items
            .sort_by(|a, b| (a.done, a.due_unix, &a.id).cmp(&(b.done, b.due_unix, &b.id)));
    }

    /// `r{now}-{counter}`, skipping ids already present: a store reopened
    /// within the same second restarts its counter at zero.
    fn next_id(&mut self, now: i64) -> String {
        loop {
            let id = format!("r{now}-{}", self.counter);
            self.counter += 1;
            if self.get(&id).is_none() {
                return id;
            }
        }
    }
}

/// The earliest due time of the reminders not done.
pub fn next_due(reminders: &[Reminder]) -> Option<i64> {
    reminders
        .iter()
        .filter(|r| !r.done)
        .map(|r| r.due_unix)
        .min()
}

/// Ids of the reminders not done whose time has come by `now`, earliest first.
pub fn due_now(reminders: &[Reminder], now: i64) -> Vec<String> {
    let mut due: Vec<&Reminder> = reminders
        .iter()
        .filter(|r| !r.done && r.due_unix <= now)
        .collect();
    due.sort_by_key(|r| r.due_unix);
    due.into_iter().map(|r| r.id.clone()).collect()
}

/// `days` Monday first, each once.
pub fn sorted_days(days: &[Weekday]) -> Vec<Weekday> {
    let mut days = days.to_vec();
    days.sort_by_key(|d| d.num_days_from_monday());
    days.dedup();
    days
}

/// The `days` a reminder keeps: none unless it is `Weekly`, and none when
/// they are just the weekday it is due on (`due_day`), so a reminder that
/// rings on one day keeps the shape of the file before `days` existed.
fn stored_days(repeat: Repeat, days: &[Weekday], due_day: Option<Weekday>) -> Vec<Weekday> {
    if repeat != Repeat::Weekly {
        return Vec::new();
    }
    let days = sorted_days(days);
    if days.len() == 1 && Some(days[0]) == due_day {
        Vec::new()
    } else {
        days
    }
}

/// The rule for when a repeating reminder rings next, shared by the store
/// (in a time zone) and the editor's summary (in local dates): the first
/// date from `start` on (`inclusive`) or after it that is one of its days
/// and for which `ring` gives a value (an occurrence still to come), with
/// that value. Its days: every day for `Daily`; for `Weekly` the weekdays in
/// `days`, or `start`'s own weekday when `days` is empty. `today` is the
/// local date of now: the dates before yesterday cannot ring after now and
/// are skipped by whole periods. `None` for a one-off, or at the end of the
/// calendar.
pub fn next_repeat<T>(
    repeat: Repeat,
    start: NaiveDate,
    days: &[Weekday],
    inclusive: bool,
    today: NaiveDate,
    mut ring: impl FnMut(NaiveDate) -> Option<T>,
) -> Option<(NaiveDate, T)> {
    let step = repeat.step_days()?;
    let rings_on = |d: NaiveDate| match repeat {
        Repeat::Weekly if days.is_empty() => d.weekday() == start.weekday(),
        Repeat::Weekly => days.contains(&d.weekday()),
        _ => true,
    };
    let mut date = if inclusive { start } else { start.succ_opt()? };
    // Whole periods (a day, a week) whose dates all lie before yesterday:
    // the pattern repeats each period, so the dates left are the same days.
    let behind = (today - date).num_days() - 1;
    if behind >= step as i64 {
        date = date.checked_add_days(Days::new(behind as u64 / step * step))?;
    }
    loop {
        if rings_on(date)
            && let Some(value) = ring(date)
        {
            return Some((date, value));
        }
        date = date.succ_opt()?;
    }
}

/// The occurrence of a repeating reminder in `tz` at wall time `time` after
/// `now`, from `start` (`inclusive`) or the day after (`next_repeat`).
fn next_in<Tz: TimeZone>(
    tz: &Tz,
    start: NaiveDate,
    time: NaiveTime,
    repeat: Repeat,
    days: &[Weekday],
    inclusive: bool,
    now: i64,
) -> Option<i64> {
    let today = tz.timestamp_opt(now, 0).single()?.naive_local().date();
    let ring = |d: NaiveDate| at_local(tz, d.and_time(time)).filter(|&at| at > now);
    next_repeat(repeat, start, days, inclusive, today, ring).map(|(_, at)| at)
}

/// The next occurrence of a repeating reminder (`None` for a one-off) after
/// both `due` and `now` in time zone `tz`, on the days after `due`'s local
/// date that are its `days` (`Reminder::days`; empty: `due`'s weekday for a
/// weekly one), at the wall-clock time `wall` (`None`: the wall time of
/// `due`). Pass the intended time: when `due` fell in a skipped DST hour,
/// its own wall time is an hour late.
pub fn advance_in<Tz: TimeZone>(
    tz: &Tz,
    due: i64,
    wall: Option<NaiveTime>,
    repeat: Repeat,
    days: &[Weekday],
    now: i64,
) -> Option<i64> {
    repeat.step_days()?;
    let shown = tz.timestamp_opt(due, 0).single()?.naive_local();
    let (base, time) = (shown.date(), wall.unwrap_or(shown.time()));
    next_in(tz, base, time, repeat, days, false, now)
}

/// Unix seconds of a local date and time. In a fold (clocks turned back) the
/// earlier instant; in a gap (clocks turned forward) the same time an hour
/// later, which is the wall-clock time the gap skipped to.
pub fn at_local<Tz: TimeZone>(tz: &Tz, t: NaiveDateTime) -> Option<i64> {
    tz.from_local_datetime(&t)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(t + Duration::hours(1))).earliest())
        .map(|t| t.timestamp())
}

/// `at_local` in the system time zone.
pub fn local_unix(date: NaiveDate, time: NaiveTime) -> Option<i64> {
    at_local(&chrono::Local, date.and_time(time))
}

/// The local date and time of `unix`.
pub fn local_time(unix: i64) -> Option<NaiveDateTime> {
    DateTime::from_timestamp(unix, 0).map(|t| t.with_timezone(&chrono::Local).naive_local())
}

/// The due time to store for a reminder set to `due` in `tz`: a repeating one
/// set in the past starts at its next occurrence instead of firing at once,
/// repeating at wall-clock time `wall` (see `advance_in`). A weekly one
/// whose `days` leave out `due`'s weekday starts on the first of them from
/// `due`'s date on.
fn first_due_at<Tz: TimeZone>(
    tz: &Tz,
    due: i64,
    wall: Option<NaiveTime>,
    repeat: Repeat,
    days: &[Weekday],
    now: i64,
) -> i64 {
    if repeat == Repeat::Once {
        return due;
    }
    let Some(shown) = tz.timestamp_opt(due, 0).single().map(|t| t.naive_local()) else {
        return due;
    };
    let (start, time) = (shown.date(), wall.unwrap_or(shown.time()));
    let on_its_day = repeat == Repeat::Daily || days.is_empty() || days.contains(&start.weekday());
    if due > now && on_its_day {
        return due;
    }
    next_in(tz, start, time, repeat, days, true, now).unwrap_or(due)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, MappedLocalTime, NaiveDate, NaiveTime, Utc};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh directory under the system temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "vale-reminders-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("reminders.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Central European time for 2026: UTC+1, and UTC+2 from 29 Mar 02:00
    /// (clocks jump to 03:00) until 25 Oct 03:00 (clocks fall back to 02:00).
    #[derive(Clone, Copy, Debug)]
    struct Cet;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    impl Cet {
        fn offset_at_utc(t: &NaiveDateTime) -> FixedOffset {
            let summer = *t >= utc(2026, 3, 29, 1, 0) && *t < utc(2026, 10, 25, 1, 0);
            FixedOffset::east_opt(if summer { 7200 } else { 3600 }).unwrap()
        }
    }

    impl TimeZone for Cet {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Cet
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            // Offsets that map this wall time back onto themselves; summer first,
            // since it is the earlier instant in a fold.
            let fits: Vec<FixedOffset> = [7200, 3600]
                .into_iter()
                .map(|s| FixedOffset::east_opt(s).unwrap())
                .filter(|off| {
                    let at = *local - Duration::seconds(off.local_minus_utc().into());
                    Cet::offset_at_utc(&at) == *off
                })
                .collect();
            match fits[..] {
                [] => MappedLocalTime::None,
                [one] => MappedLocalTime::Single(one),
                [a, b, ..] => MappedLocalTime::Ambiguous(a, b),
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            Cet::offset_at_utc(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            Cet::offset_at_utc(utc)
        }
    }

    fn cet(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        at_local(&Cet, utc(y, m, d, h, min)).unwrap()
    }

    fn wall(unix: i64) -> NaiveDateTime {
        Cet.timestamp_opt(unix, 0).unwrap().naive_local()
    }

    fn reminder(id: &str, due: i64, repeat: Repeat) -> Reminder {
        Reminder {
            id: id.into(),
            text: id.into(),
            due_unix: due,
            repeat,
            done: false,
            days: Vec::new(),
        }
    }

    #[test]
    fn once_never_advances() {
        assert_eq!(advance_in(&Utc, 1000, None, Repeat::Once, &[], 5000), None);
    }

    #[test]
    fn daily_and_weekly_step_to_after_now() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let due = at(1, 8);
        // Dismissed a minute after it fired: tomorrow at the same time.
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Daily, &[], due + 60),
            Some(at(2, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Weekly, &[], due + 60),
            Some(at(8, 8))
        );
        // Off for days: the first occurrence still to come, not one per day missed.
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Daily, &[], at(10, 9)),
            Some(at(11, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Daily, &[], at(10, 7)),
            Some(at(10, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Weekly, &[], at(20, 12)),
            Some(at(22, 8))
        );
        // Exactly at an occurrence: the next one (strictly after now).
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Weekly, &[], at(8, 8)),
            Some(at(15, 8))
        );
        // Not due yet (dismissed early): still the occurrence after this one.
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Daily, &[], due - 3600),
            Some(at(2, 8))
        );
        // A year away still lands on the right day.
        let later = utc(2027, 6, 1, 9, 0).and_utc().timestamp();
        assert_eq!(
            advance_in(&Utc, due, None, Repeat::Daily, &[], later),
            Some(utc(2027, 6, 2, 8, 0).and_utc().timestamp())
        );
    }

    #[test]
    fn repeats_keep_the_local_time_across_dst() {
        // 08:00 the day before clocks go forward: the next day is 23 h later.
        let due = cet(2026, 3, 28, 8, 0);
        let next = advance_in(&Cet, due, None, Repeat::Daily, &[], due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 3, 29, 8, 0));
        assert_eq!(next - due, 23 * 3600);
        // ...and back in autumn: 25 h.
        let due = cet(2026, 10, 24, 8, 0);
        let next = advance_in(&Cet, due, None, Repeat::Daily, &[], due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 10, 25, 8, 0));
        assert_eq!(next - due, 25 * 3600);
        // Weekly across the change.
        let due = cet(2026, 3, 25, 19, 30);
        let next = advance_in(&Cet, due, None, Repeat::Weekly, &[], due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 4, 1, 19, 30));
    }

    #[test]
    fn repeats_through_the_skipped_and_doubled_hour() {
        // 02:30 does not exist on 29 Mar: that day it rings at 03:30, and
        // the day after at 02:30 again.
        let half_two = NaiveTime::from_hms_opt(2, 30, 0);
        let due = cet(2026, 3, 28, 2, 30);
        let gap = advance_in(&Cet, due, half_two, Repeat::Daily, &[], due + 60).unwrap();
        assert_eq!(wall(gap), utc(2026, 3, 29, 3, 30));
        // Advancing from the stored (moved) instant goes back to 02:30...
        let after = advance_in(&Cet, gap, half_two, Repeat::Daily, &[], gap + 60).unwrap();
        assert_eq!(wall(after), utc(2026, 3, 30, 2, 30));
        let weekly = advance_in(&Cet, gap, half_two, Repeat::Weekly, &[], gap + 60).unwrap();
        assert_eq!(wall(weekly), utc(2026, 4, 5, 2, 30));
        // ...which the instant alone cannot tell: it reads 03:30.
        let drifted = advance_in(&Cet, gap, None, Repeat::Daily, &[], gap + 60).unwrap();
        assert_eq!(wall(drifted), utc(2026, 3, 30, 3, 30));
        // Set at 02:30 on that very day and already past: next day 02:30.
        let first = first_due_at(&Cet, gap, half_two, Repeat::Daily, &[], gap + 60);
        assert_eq!(wall(first), utc(2026, 3, 30, 2, 30));
        // 02:30 happens twice on 25 Oct: the first one.
        let due = cet(2026, 10, 24, 2, 30);
        let fold = advance_in(&Cet, due, None, Repeat::Daily, &[], due + 60).unwrap();
        assert_eq!(wall(fold), utc(2026, 10, 25, 2, 30));
        assert_eq!(fold, utc(2026, 10, 25, 0, 30).and_utc().timestamp());
    }

    #[test]
    fn repeating_reminder_set_in_the_past_starts_at_its_next_time() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let now = at(5, 10);
        assert_eq!(
            first_due_at(&Utc, at(5, 8), None, Repeat::Daily, &[], now),
            at(6, 8)
        );
        assert_eq!(
            first_due_at(&Utc, at(5, 8), None, Repeat::Weekly, &[], now),
            at(12, 8)
        );
        assert_eq!(
            first_due_at(&Utc, at(5, 12), None, Repeat::Daily, &[], now),
            at(5, 12)
        );
        // A past one-off stays as it is (the editor refuses it).
        assert_eq!(
            first_due_at(&Utc, at(5, 8), None, Repeat::Once, &[], now),
            at(5, 8)
        );
    }

    use chrono::Weekday::{Fri, Mon, Sat, Sun, Thu, Tue, Wed};

    #[test]
    fn multi_day_weekly_steps_through_its_days() {
        // 1 June 2026 is a Monday.
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let days = [Mon, Wed, Fri];
        let step = |due: i64| advance_in(&Utc, due, None, Repeat::Weekly, &days, due + 60);
        let wed = step(at(1, 8)).unwrap();
        assert_eq!(wed, at(3, 8));
        let fri = step(wed).unwrap();
        assert_eq!(fri, at(5, 8));
        assert_eq!(step(fri), Some(at(8, 8)), "then the next Monday");
        // Dismissed early, before it rang: still the day after its own.
        assert_eq!(
            advance_in(&Utc, at(3, 8), None, Repeat::Weekly, &days, at(3, 7)),
            Some(at(5, 8))
        );
    }

    #[test]
    fn multi_day_weekly_jumps_over_missed_weeks() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let days = [Mon, Wed, Fri];
        let due = at(1, 8);
        let next = |now| advance_in(&Utc, due, None, Repeat::Weekly, &days, now);
        // Off for three weeks: the first one still to come.
        assert_eq!(next(at(25, 12)), Some(at(26, 8)));
        assert_eq!(next(at(26, 9)), Some(at(29, 8)));
        assert_eq!(next(at(24, 7)), Some(at(24, 8)), "later that day");
        // A year on lands on the right weekday.
        let later = utc(2027, 6, 2, 9, 0).and_utc().timestamp(); // a Wednesday
        assert_eq!(
            next(later),
            Some(utc(2027, 6, 4, 8, 0).and_utc().timestamp())
        );
    }

    #[test]
    fn weekly_with_no_days_is_the_old_weekly() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let due = at(3, 8); // a Wednesday
        for now in (0..40).map(|i| due - 86_400 + i * 43_200 + 1_800) {
            let old = advance_in(&Utc, due, None, Repeat::Weekly, &[], now);
            let own = advance_in(&Utc, due, None, Repeat::Weekly, &[Wed], now);
            assert_eq!(old, own, "{now}");
            assert_eq!(
                wall_of(&Utc, old.unwrap()),
                NaiveTime::from_hms_opt(8, 0, 0)
            );
            assert_eq!(
                first_due_at(&Utc, due, None, Repeat::Weekly, &[], now),
                first_due_at(&Utc, due, None, Repeat::Weekly, &[Wed], now)
            );
        }
    }

    #[test]
    fn weekly_starting_on_a_day_it_skips() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        // Set for Tuesday 2 June, but only on Mondays and Thursdays.
        let start = at(2, 8);
        let days = [Mon, Thu];
        assert_eq!(
            first_due_at(&Utc, start, None, Repeat::Weekly, &days, at(1, 0)),
            at(4, 8)
        );
        // Set in the past: the first of its days still to come.
        assert_eq!(
            first_due_at(&Utc, start, None, Repeat::Weekly, &days, at(4, 9)),
            at(8, 8)
        );
        // A start on one of its days, still ahead: that very day.
        assert_eq!(
            first_due_at(&Utc, at(4, 8), None, Repeat::Weekly, &days, at(1, 0)),
            at(4, 8)
        );
        // Later the same day it was set for, on one of its days.
        assert_eq!(
            first_due_at(&Utc, at(4, 8), None, Repeat::Weekly, &days, at(4, 7)),
            at(4, 8)
        );
    }

    #[test]
    fn multi_day_weekly_across_dst() {
        // Spring: Sunday 29 Mar skips 02:00-03:00. Wed and Sun at 19:30.
        let due = cet(2026, 3, 25, 19, 30);
        let sun = advance_in(&Cet, due, None, Repeat::Weekly, &[Wed, Sun], due + 60).unwrap();
        assert_eq!(wall(sun), utc(2026, 3, 29, 19, 30));
        assert_eq!(sun - due, 4 * 86_400 - 3600, "an hour shorter");
        // Autumn: 02:30 happens twice on Sunday 25 Oct; the first one rings.
        let half_two = NaiveTime::from_hms_opt(2, 30, 0);
        let due = cet(2026, 10, 23, 2, 30);
        let fold = advance_in(&Cet, due, half_two, Repeat::Weekly, &[Fri, Sun], due + 60);
        let fold = fold.unwrap();
        assert_eq!(wall(fold), utc(2026, 10, 25, 2, 30));
        assert_eq!(fold, utc(2026, 10, 25, 0, 30).and_utc().timestamp());
        // Not again at the second 02:30: on to Friday.
        let fri = advance_in(&Cet, fold, half_two, Repeat::Weekly, &[Fri, Sun], fold + 60);
        assert_eq!(wall(fri.unwrap()), utc(2026, 10, 30, 2, 30));
    }

    #[test]
    fn multi_day_weekly_keeps_its_time_after_a_skipped_hour() {
        // Saturday, Sunday and Monday at 02:30; Sunday 29 Mar skips 02:30.
        let half_two = NaiveTime::from_hms_opt(2, 30, 0);
        let days = [Sat, Sun, Mon];
        let due = cet(2026, 3, 28, 2, 30);
        let gap = advance_in(&Cet, due, half_two, Repeat::Weekly, &days, due + 60).unwrap();
        assert_eq!(wall(gap), utc(2026, 3, 29, 3, 30));
        let mon = advance_in(&Cet, gap, half_two, Repeat::Weekly, &days, gap + 60).unwrap();
        assert_eq!(wall(mon), utc(2026, 3, 30, 2, 30), "back at 02:30");
        let drifted = advance_in(&Cet, gap, None, Repeat::Weekly, &days, gap + 60).unwrap();
        assert_eq!(wall(drifted), utc(2026, 3, 30, 3, 30));
        // Set for that Sunday at 02:30 and already past: Monday 02:30.
        let first = first_due_at(&Cet, gap, half_two, Repeat::Weekly, &days, gap + 60);
        assert_eq!(wall(first), utc(2026, 3, 30, 2, 30));
    }

    #[test]
    fn next_repeat_is_none_for_a_one_off() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert_eq!(next_repeat(Repeat::Once, d, &[], true, d, Some), None);
        let daily = next_repeat(Repeat::Daily, d, &[], false, d, Some);
        assert_eq!(daily.map(|(d, _)| d), d.succ_opt());
    }

    #[test]
    fn days_are_kept_sorted_and_only_for_weekly() {
        assert_eq!(sorted_days(&[Fri, Mon, Wed, Mon]), [Mon, Wed, Fri]);
        assert_eq!(sorted_days(&[Sun, Mon]), [Mon, Sun], "Monday first");
        let tmp = TempDir::new("days");
        let mut store = RemindersStore::open(tmp.file());
        // Tuesday 4 June 2030, an hour from now.
        let tue = NaiveDate::from_ymd_opt(2030, 6, 4).unwrap();
        let nine = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let now = local_unix(tue, nine).unwrap() - 3600;
        let put = |store: &mut RemindersStore, repeat, days: &[Weekday]| {
            let id = store
                .upsert_local(None, "x", tue, nine, repeat, days, now)
                .unwrap();
            store.get(&id).unwrap().clone()
        };
        let r = put(&mut store, Repeat::Weekly, &[Fri, Mon, Fri]);
        assert_eq!(r.days, [Mon, Fri]);
        // Starts on the first of them: Friday 7 June.
        let first = local_time(r.due_unix).unwrap();
        assert_eq!(first.date(), NaiveDate::from_ymd_opt(2030, 6, 7).unwrap());
        assert_eq!(first.time(), nine);
        // Just the day it starts on: stored as an old weekly one.
        assert!(put(&mut store, Repeat::Weekly, &[Tue]).days.is_empty());
        // One other day: the first due is that day, so the same.
        let thu = put(&mut store, Repeat::Weekly, &[Thu]);
        assert!(thu.days.is_empty());
        assert_eq!(local_time(thu.due_unix).unwrap().date().weekday(), Thu);
        // Days mean nothing to a daily or one-off reminder.
        assert!(put(&mut store, Repeat::Daily, &[Mon, Wed]).days.is_empty());
        assert!(put(&mut store, Repeat::Once, &[Mon]).days.is_empty());
        // Dismissing moves it to its next day.
        let id = store
            .upsert_local(None, "y", tue, nine, Repeat::Weekly, &[Tue, Fri], now)
            .unwrap();
        let due = store.get(&id).unwrap().due_unix;
        assert!(store.mark_done_or_advance(&id, due + 60));
        let next = local_time(store.get(&id).unwrap().due_unix).unwrap();
        assert_eq!(next.date(), NaiveDate::from_ymd_opt(2030, 6, 7).unwrap());
    }

    #[test]
    fn old_reminders_file_loads_and_saves_without_days() {
        let tmp = TempDir::new("old-file");
        let old = serde_json::to_string_pretty(&serde_json::json!([
            {"id": "r1-0", "text": "Backup", "due_unix": 1_780_000_000_i64,
             "repeat": "weekly", "done": false, "time": "09:00:00"},
            {"id": "r1-1", "text": "Obat", "due_unix": 1_780_100_000_i64,
             "repeat": "daily", "done": false, "time": "08:00:00"},
        ]))
        .unwrap();
        std::fs::write(tmp.file(), &old).unwrap();
        let mut store = RemindersStore::open(tmp.file());
        assert!(!store.load_failed());
        let r = store.get("r1-0").unwrap();
        assert_eq!((r.repeat, r.days.is_empty()), (Repeat::Weekly, true));
        assert_eq!(store.wall_time("r1-0"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(store.get("r1-1").unwrap().repeat, Repeat::Daily);
        store.save().unwrap();
        let saved = std::fs::read_to_string(tmp.file()).unwrap();
        assert_eq!(saved, old, "the same file, no `days` key");
    }

    #[test]
    fn new_reminders_file_with_days_round_trips() {
        let tmp = TempDir::new("new-file");
        let new = serde_json::to_string_pretty(&serde_json::json!([
            {"id": "r1-0", "text": "Backup", "due_unix": 1_780_000_000_i64,
             "repeat": "weekly", "done": false, "days": ["Mon", "Wed", "Fri"],
             "time": "09:00:00"},
        ]))
        .unwrap();
        std::fs::write(tmp.file(), &new).unwrap();
        let mut store = RemindersStore::open(tmp.file());
        assert!(!store.load_failed());
        assert_eq!(store.get("r1-0").unwrap().days, [Mon, Wed, Fri]);
        store.save().unwrap();
        assert_eq!(std::fs::read_to_string(tmp.file()).unwrap(), new);
        // Day names load in any case or length, as chrono reads them.
        let (items, keep) = load(Ok(
            r#"[{"id":"a","text":"x","due_unix":1,"repeat":"weekly","days":["tuesday","SAT"]}]"#
                .into(),
        ));
        assert!(!keep);
        assert_eq!(items[0].reminder.days, [Tue, Sat]);
    }

    #[test]
    fn next_due_skips_done_and_due_now_lists_the_past_ones() {
        let mut done = reminder("done", 50, Repeat::Once);
        done.done = true;
        let list = vec![
            reminder("c", 300, Repeat::Daily),
            done,
            reminder("a", 100, Repeat::Once),
            reminder("b", 200, Repeat::Weekly),
        ];
        assert_eq!(next_due(&list), Some(100));
        assert_eq!(next_due(&[]), None);
        assert_eq!(due_now(&list, 99), Vec::<String>::new());
        assert_eq!(due_now(&list, 200), vec!["a", "b"]);
        assert_eq!(due_now(&list, 1000), vec!["a", "b", "c"]);
    }

    #[test]
    fn missed_while_off_are_due_at_startup() {
        let tmp = TempDir::new("missed");
        let mut store = RemindersStore::open(tmp.file());
        let yesterday = store
            .upsert(None, "kemarin", 1_000, Repeat::Once, &[], 1)
            .unwrap();
        let daily = store
            .upsert(None, "harian", 2_000, Repeat::Daily, &[], 1)
            .unwrap();
        let later = store
            .upsert(None, "nanti", 90_000, Repeat::Once, &[], 1)
            .unwrap();
        store.save().unwrap();
        // The island starts again long after both were due.
        let store = RemindersStore::open(tmp.file());
        let now = 50_000;
        assert_eq!(due_now(store.list(), now), vec![yesterday, daily.clone()]);
        assert_eq!(next_due(store.list()), Some(1_000));
        // Dismissing the daily one skips the missed days to the next one to come.
        let mut store = store;
        assert!(store.mark_done_or_advance(&daily, now));
        let next = store.get(&daily).unwrap().due_unix;
        assert!(next > now && (next - 2_000) % 86_400 == 0, "{next}");
        assert!(!store.get(&daily).unwrap().done);
        assert!(store.get(&later).is_some());
    }

    #[test]
    fn mark_done_finishes_once_and_drops_old_done() {
        let tmp = TempDir::new("done");
        let mut store = RemindersStore::open(tmp.file());
        let old = store
            .upsert(None, "lama", 100, Repeat::Once, &[], 1)
            .unwrap();
        let new = store
            .upsert(None, "baru", 200, Repeat::Once, &[], 1)
            .unwrap();
        assert!(store.mark_done_or_advance(&old, 150));
        assert!(store.get(&old).unwrap().done);
        assert!(!store.mark_done_or_advance(&old, 150), "already done");
        assert!(!store.mark_done_or_advance("r0-gone", 150));
        assert_eq!(store.upcoming().len(), 1);
        assert_eq!(store.upcoming()[0].id, new);
        assert_eq!(store.list().len(), 2, "done ones are kept a while");
        // A month later, finishing another drops the old done one.
        assert!(store.mark_done_or_advance(&new, 100 + KEEP_DONE_SECS + 1));
        assert!(store.get(&old).is_none());
        assert!(store.get(&new).is_some());
        assert!(store.upcoming().is_empty());
    }

    #[test]
    fn upsert_creates_edits_and_sorts_by_due() {
        let tmp = TempDir::new("upsert");
        let mut store = RemindersStore::open(tmp.file());
        let b = store
            .upsert(None, " minum obat ", 300, Repeat::Daily, &[], 7)
            .unwrap();
        assert!(b.starts_with("r7-"), "{b}");
        let a = store
            .upsert(None, "telepon", 100, Repeat::Once, &[], 7)
            .unwrap();
        assert_ne!(a, b);
        let ids: Vec<&str> = store.upcoming().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [a.as_str(), b.as_str()]);
        assert_eq!(store.get(&b).unwrap().text, "minum obat", "trimmed");
        // Editing moves it and keeps the id; an edited done reminder is live again.
        store.mark_done_or_advance(&a, 150);
        assert_eq!(
            store.upsert(Some(&a), "telepon ibu", 400, Repeat::Weekly, &[], 8),
            Some(a.clone())
        );
        let r = store.get(&a).unwrap();
        assert_eq!((r.due_unix, r.repeat, r.done), (400, Repeat::Weekly, false));
        assert_eq!(store.upcoming()[1].id, a);
        // Empty text changes nothing.
        assert_eq!(store.upsert(Some(&a), "  ", 1, Repeat::Once, &[], 9), None);
        assert_eq!(store.upsert(None, "\n", 1, Repeat::Once, &[], 9), None);
        assert_eq!(store.list().len(), 2);
        assert_eq!(store.get(&a).unwrap().text, "telepon ibu");
        assert!(store.delete(&a));
        assert!(!store.delete(&a));
    }

    #[test]
    fn save_and_reopen_round_trip() {
        let tmp = TempDir::new("roundtrip");
        let mut store = RemindersStore::open(tmp.file());
        let a = store
            .upsert(None, "a \"kutip\"", 5, Repeat::Once, &[], 1)
            .unwrap();
        store.upsert(None, "b", 9, Repeat::Weekly, &[], 1).unwrap();
        store.mark_done_or_advance(&a, 6);
        store.save().unwrap();
        assert!(!tmp.0.join("reminders.json.tmp").exists());
        let reopened = RemindersStore::open(tmp.file());
        assert_eq!(reopened.list(), store.list());
        assert!(!reopened.load_failed());
        // The id counter never collides with saved ids from the same second.
        let mut reopened = reopened;
        let c = reopened.upsert(None, "c", 7, Repeat::Once, &[], 1).unwrap();
        assert!(store.get(&c).is_none(), "id {c} collided");
    }

    #[test]
    fn wall_time_is_kept_saved_and_derived_for_old_files() {
        let tmp = TempDir::new("wall");
        let mut store = RemindersStore::open(tmp.file());
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let eight = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        // Set in the past: a repeating one starts at its next time.
        let now = local_unix(date, eight).unwrap() + 3600;
        let id = store
            .upsert_local(None, "obat", date, eight, Repeat::Daily, &[], now)
            .unwrap();
        let r = store.get(&id).unwrap().clone();
        assert!(r.due_unix > now);
        assert_eq!(local_time(r.due_unix).unwrap().time(), eight);
        assert_eq!(store.wall_time(&id), Some(eight));
        // Dismissing keeps the wall time; it is written and read back.
        assert!(store.mark_done_or_advance(&id, r.due_unix + 60));
        assert_eq!(store.wall_time(&id), Some(eight));
        store.save().unwrap();
        let text = std::fs::read_to_string(tmp.file()).unwrap();
        assert!(text.contains(r#""time": "08:00:00""#), "{text}");
        let reopened = RemindersStore::open(tmp.file());
        assert_eq!(reopened.wall_time(&id), Some(eight));
        assert_eq!(reopened.list(), store.list());
        // `upsert` takes the wall time of the instant; delete forgets it.
        let mut store = reopened;
        let nine = local_unix(date, NaiveTime::from_hms_opt(9, 15, 0).unwrap()).unwrap();
        let other = store
            .upsert(None, "lain", nine, Repeat::Weekly, &[], 1)
            .unwrap();
        assert_eq!(store.wall_time(&other), NaiveTime::from_hms_opt(9, 15, 0));
        assert!(store.delete(&other));
        assert_eq!(store.wall_time(&other), None);
        // A file from before the field: the wall time of `due_unix`.
        let old = format!(r#"[{{"id":"r1-0","text":"x","due_unix":{nine},"repeat":"daily"}}]"#);
        std::fs::write(tmp.file(), old).unwrap();
        let store = RemindersStore::open(tmp.file());
        assert_eq!(store.wall_time("r1-0"), NaiveTime::from_hms_opt(9, 15, 0));
    }

    #[test]
    fn file_format_is_plain_json() {
        let json = r#"[{"id":"r1-0","text":"x","due_unix":10,"repeat":"daily","done":false},
                       {"id":"r1-1","text":"y","due_unix":5,"repeat":"none"},
                       {"id":"r1-2","text":"z","due_unix":7}]"#;
        let (items, keep) = load(Ok(json.into()));
        assert!(!keep);
        let repeats: Vec<Repeat> = items.iter().map(|r| r.reminder.repeat).collect();
        assert_eq!(repeats, [Repeat::Daily, Repeat::Once, Repeat::Once]);
        let out = serde_json::to_string(&items[0]).unwrap();
        assert_eq!(
            out,
            r#"{"id":"r1-0","text":"x","due_unix":10,"repeat":"daily","done":false}"#
        );
    }

    #[test]
    fn corrupt_file_is_kept_as_backup() {
        let tmp = TempDir::new("corrupt");
        std::fs::write(tmp.file(), "{not json").unwrap();
        let mut store = RemindersStore::open(tmp.file());
        assert!(store.list().is_empty());
        assert!(store.load_failed());
        store.upsert(None, "baru", 1, Repeat::Once, &[], 1);
        assert_eq!(std::fs::read_to_string(tmp.file()).unwrap(), "{not json");
        store.save().unwrap();
        assert!(!store.load_failed());
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("reminders.json.bak")).unwrap(),
            "{not json"
        );
        assert_eq!(RemindersStore::open(tmp.file()).list().len(), 1);
    }

    #[test]
    fn load_decides_what_to_keep() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(load(Err(missing)), (vec![], false));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(load(Err(denied)), (vec![], true));
        assert_eq!(load(Ok(String::new())), (vec![], true));
        assert_eq!(load(Ok("\u{feff}[]".into())), (vec![], false));
        assert_eq!(load(Ok(r#"[{"id":1}]"#.into())), (vec![], true));
    }

    #[test]
    fn missing_file_is_empty_and_unwritten() {
        let tmp = TempDir::new("missing");
        let store = RemindersStore::open(tmp.file());
        assert!(store.list().is_empty() && store.upcoming().is_empty());
        assert!(!store.load_failed());
        assert!(!tmp.file().exists());
    }
}
