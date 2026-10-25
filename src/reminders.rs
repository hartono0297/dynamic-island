//! Reminders kept in a local JSON file, and the time arithmetic behind them:
//! which are due, when the next one is, and when a repeating one comes back.
//! Times are Unix seconds; repeats keep the local wall-clock time across
//! offset changes (DST), so a daily 08:00 stays 08:00.

use chrono::{DateTime, Days, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
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
}

/// Done reminders are dropped once they are this old (seconds past due).
const KEEP_DONE_SECS: i64 = 30 * 24 * 3600;

/// Reminders in a local JSON file. Changes stay in memory until `save`.
pub struct RemindersStore {
    path: PathBuf,
    /// Not done first, by due time (then id); done ones after, the same way.
    items: Vec<Reminder>,
    counter: u64,
    /// The file existed but could not be read or parsed. It is moved aside
    /// (`reminders.json.bak`) before the first save instead of being overwritten.
    keep_old: bool,
}

/// The reminders in a file read as `read`, and whether the file must be kept
/// aside: it exists but is unreadable or not a reminders list. A missing file
/// is simply empty.
fn load(read: io::Result<String>) -> (Vec<Reminder>, bool) {
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
        let (items, keep_old) = load(std::fs::read_to_string(&path));
        let mut store = RemindersStore {
            path,
            items,
            counter: 0,
            keep_old,
        };
        store.sort();
        store
    }

    /// `%APPDATA%\DynamicIsland\reminders.json`, next to the config file.
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

    /// Creates or edits a reminder and returns its id; an edit is not done any
    /// more. Text that is empty after trimming creates or changes nothing and
    /// returns `None`. An unknown `id` is treated as a new reminder.
    pub fn upsert(
        &mut self,
        id: Option<&str>,
        text: &str,
        due_unix: i64,
        repeat: Repeat,
        now: i64,
    ) -> Option<String> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let id = match id.and_then(|id| self.items.iter_mut().find(|r| r.id == id)) {
            Some(r) => {
                r.text = text.to_owned();
                r.due_unix = due_unix;
                r.repeat = repeat;
                r.done = false;
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
                });
                id
            }
        };
        self.sort();
        Some(id)
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|r| r.id != id);
        self.items.len() != before
    }

    /// The reminder was dismissed at `now`: a one-off is done, a repeating one
    /// moves to its next occurrence after `now`. Done ones older than a month
    /// are dropped on the way. False when `id` is unknown or already done.
    pub fn mark_done_or_advance(&mut self, id: &str, now: i64) -> bool {
        let Some(r) = self.items.iter_mut().find(|r| r.id == id && !r.done) else {
            return false;
        };
        match advance(r, now) {
            Some(next) => r.due_unix = next,
            None => r.done = true,
        }
        self.items
            .retain(|r| !r.done || r.due_unix > now - KEEP_DONE_SECS);
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
        let json = serde_json::to_string_pretty(&self.items).map_err(io::Error::other)?;
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

/// The next occurrence of a repeating reminder after both its due time and
/// `now`, at the same local wall-clock time; `None` for a one-off.
pub fn advance(r: &Reminder, now: i64) -> Option<i64> {
    advance_in(&chrono::Local, r.due_unix, r.repeat, now)
}

/// `advance` in time zone `tz`.
pub fn advance_in<Tz: TimeZone>(tz: &Tz, due: i64, repeat: Repeat, now: i64) -> Option<i64> {
    let step = repeat.step_days()?;
    let wall = tz.timestamp_opt(due, 0).single()?.naive_local();
    let today = tz.timestamp_opt(now, 0).single()?.naive_local().date();
    // Skip the whole steps that are surely past; the loop finds the exact one.
    let behind = (today - wall.date()).num_days().max(0) as u64;
    let mut k = (behind / step).saturating_sub(1).max(1);
    loop {
        let date = wall.date().checked_add_days(Days::new(k * step))?;
        let at = at_local(tz, date.and_time(wall.time()))?;
        if at > now {
            return Some(at);
        }
        k += 1;
    }
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

/// The due time to store for a reminder set to `due`: a repeating one set in
/// the past starts at its next occurrence instead of firing at once.
pub fn first_due(due: i64, repeat: Repeat, now: i64) -> i64 {
    first_due_in(&chrono::Local, due, repeat, now)
}

fn first_due_in<Tz: TimeZone>(tz: &Tz, due: i64, repeat: Repeat, now: i64) -> i64 {
    if repeat == Repeat::Once || due > now {
        due
    } else {
        advance_in(tz, due, repeat, now).unwrap_or(due)
    }
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
                "dynamic-island-reminders-{name}-{}-{}",
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
        }
    }

    #[test]
    fn once_never_advances() {
        assert_eq!(advance_in(&Utc, 1000, Repeat::Once, 5000), None);
    }

    #[test]
    fn daily_and_weekly_step_to_after_now() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let due = at(1, 8);
        // Dismissed a minute after it fired: tomorrow at the same time.
        assert_eq!(
            advance_in(&Utc, due, Repeat::Daily, due + 60),
            Some(at(2, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, Repeat::Weekly, due + 60),
            Some(at(8, 8))
        );
        // Off for days: the first occurrence still to come, not one per day missed.
        assert_eq!(
            advance_in(&Utc, due, Repeat::Daily, at(10, 9)),
            Some(at(11, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, Repeat::Daily, at(10, 7)),
            Some(at(10, 8))
        );
        assert_eq!(
            advance_in(&Utc, due, Repeat::Weekly, at(20, 12)),
            Some(at(22, 8))
        );
        // Exactly at an occurrence: the next one (strictly after now).
        assert_eq!(
            advance_in(&Utc, due, Repeat::Weekly, at(8, 8)),
            Some(at(15, 8))
        );
        // Not due yet (dismissed early): still the occurrence after this one.
        assert_eq!(
            advance_in(&Utc, due, Repeat::Daily, due - 3600),
            Some(at(2, 8))
        );
        // A year away still lands on the right day.
        let later = utc(2027, 6, 1, 9, 0).and_utc().timestamp();
        assert_eq!(
            advance_in(&Utc, due, Repeat::Daily, later),
            Some(utc(2027, 6, 2, 8, 0).and_utc().timestamp())
        );
    }

    #[test]
    fn repeats_keep_the_local_time_across_dst() {
        // 08:00 the day before clocks go forward: the next day is 23 h later.
        let due = cet(2026, 3, 28, 8, 0);
        let next = advance_in(&Cet, due, Repeat::Daily, due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 3, 29, 8, 0));
        assert_eq!(next - due, 23 * 3600);
        // ...and back in autumn: 25 h.
        let due = cet(2026, 10, 24, 8, 0);
        let next = advance_in(&Cet, due, Repeat::Daily, due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 10, 25, 8, 0));
        assert_eq!(next - due, 25 * 3600);
        // Weekly across the change.
        let due = cet(2026, 3, 25, 19, 30);
        let next = advance_in(&Cet, due, Repeat::Weekly, due + 60).unwrap();
        assert_eq!(wall(next), utc(2026, 4, 1, 19, 30));
    }

    #[test]
    fn repeats_through_the_skipped_and_doubled_hour() {
        // 02:30 does not exist on 29 Mar: that day it rings at 03:30, and
        // the day after at 02:30 again.
        let due = cet(2026, 3, 28, 2, 30);
        let gap = advance_in(&Cet, due, Repeat::Daily, due + 60).unwrap();
        assert_eq!(wall(gap), utc(2026, 3, 29, 3, 30));
        let after = advance_in(&Cet, due, Repeat::Daily, gap).unwrap();
        assert_eq!(wall(after), utc(2026, 3, 30, 2, 30));
        // 02:30 happens twice on 25 Oct: the first one.
        let due = cet(2026, 10, 24, 2, 30);
        let fold = advance_in(&Cet, due, Repeat::Daily, due + 60).unwrap();
        assert_eq!(wall(fold), utc(2026, 10, 25, 2, 30));
        assert_eq!(fold, utc(2026, 10, 25, 0, 30).and_utc().timestamp());
    }

    #[test]
    fn repeating_reminder_set_in_the_past_starts_at_its_next_time() {
        let at = |d, h| utc(2026, 6, d, h, 0).and_utc().timestamp();
        let now = at(5, 10);
        assert_eq!(first_due_in(&Utc, at(5, 8), Repeat::Daily, now), at(6, 8));
        assert_eq!(first_due_in(&Utc, at(5, 8), Repeat::Weekly, now), at(12, 8));
        assert_eq!(first_due_in(&Utc, at(5, 12), Repeat::Daily, now), at(5, 12));
        // A past one-off stays as it is (the editor refuses it).
        assert_eq!(first_due_in(&Utc, at(5, 8), Repeat::Once, now), at(5, 8));
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
            .upsert(None, "kemarin", 1_000, Repeat::Once, 1)
            .unwrap();
        let daily = store
            .upsert(None, "harian", 2_000, Repeat::Daily, 1)
            .unwrap();
        let later = store
            .upsert(None, "nanti", 90_000, Repeat::Once, 1)
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
        let old = store.upsert(None, "lama", 100, Repeat::Once, 1).unwrap();
        let new = store.upsert(None, "baru", 200, Repeat::Once, 1).unwrap();
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
            .upsert(None, " minum obat ", 300, Repeat::Daily, 7)
            .unwrap();
        assert!(b.starts_with("r7-"), "{b}");
        let a = store.upsert(None, "telepon", 100, Repeat::Once, 7).unwrap();
        assert_ne!(a, b);
        let ids: Vec<&str> = store.upcoming().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [a.as_str(), b.as_str()]);
        assert_eq!(store.get(&b).unwrap().text, "minum obat", "trimmed");
        // Editing moves it and keeps the id; an edited done reminder is live again.
        store.mark_done_or_advance(&a, 150);
        assert_eq!(
            store.upsert(Some(&a), "telepon ibu", 400, Repeat::Weekly, 8),
            Some(a.clone())
        );
        let r = store.get(&a).unwrap();
        assert_eq!((r.due_unix, r.repeat, r.done), (400, Repeat::Weekly, false));
        assert_eq!(store.upcoming()[1].id, a);
        // Empty text changes nothing.
        assert_eq!(store.upsert(Some(&a), "  ", 1, Repeat::Once, 9), None);
        assert_eq!(store.upsert(None, "\n", 1, Repeat::Once, 9), None);
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
            .upsert(None, "a \"kutip\"", 5, Repeat::Once, 1)
            .unwrap();
        store.upsert(None, "b", 9, Repeat::Weekly, 1).unwrap();
        store.mark_done_or_advance(&a, 6);
        store.save().unwrap();
        assert!(!tmp.0.join("reminders.json.tmp").exists());
        let reopened = RemindersStore::open(tmp.file());
        assert_eq!(reopened.list(), store.list());
        assert!(!reopened.load_failed());
        // The id counter never collides with saved ids from the same second.
        let mut reopened = reopened;
        let c = reopened.upsert(None, "c", 7, Repeat::Once, 1).unwrap();
        assert!(store.get(&c).is_none(), "id {c} collided");
    }

    #[test]
    fn file_format_is_plain_json() {
        let json = r#"[{"id":"r1-0","text":"x","due_unix":10,"repeat":"daily","done":false},
                       {"id":"r1-1","text":"y","due_unix":5,"repeat":"none"},
                       {"id":"r1-2","text":"z","due_unix":7}]"#;
        let (items, keep) = load(Ok(json.into()));
        assert!(!keep);
        let repeats: Vec<Repeat> = items.iter().map(|r| r.repeat).collect();
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
        store.upsert(None, "baru", 1, Repeat::Once, 1);
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
