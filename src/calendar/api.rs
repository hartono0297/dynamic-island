//! Google Calendar API v3: JSON parsing (pure) and the blocking calls the
//! sync thread makes. Errors reuse `AuthError`: 401 means the token is no
//! longer accepted (`Reconnect`), other failures are `Network`, with messages
//! starting "HTTP " when Google answered.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, SecondsFormat, TimeDelta};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{CalEvent, EventDraft, EventTime, fits_draft};
use crate::google::http::{Http, form_encode, url_encode};
use crate::google::oauth::AuthError;

const BASE: &str = "https://www.googleapis.com/calendar/v3";
/// Google's maximum page size for events.list.
const PAGE_SIZE: &str = "2500";
/// Shown for an event with no summary (Google's own UI says "(No title)").
const NO_TITLE: &str = "(Tanpa judul)";

/// A calendar the user shows in Google Calendar.
pub type CalendarEntry = (String, Option<u32>, bool);

#[derive(Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListEntry {
    id: String,
    #[serde(default)]
    selected: bool,
    #[serde(default)]
    access_role: String,
    background_color: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiEvent {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    start: Option<ApiTime>,
    end: Option<ApiTime>,
    /// Set on each instance that `singleEvents=true` expands from a series.
    recurring_event_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiTime {
    date: Option<NaiveDate>,
    date_time: Option<String>,
}

impl ApiTime {
    fn to_event_time(&self) -> Option<EventTime> {
        if let Some(d) = self.date {
            return Some(EventTime::AllDay(d));
        }
        let t = DateTime::parse_from_rfc3339(self.date_time.as_deref()?).ok()?;
        Some(EventTime::At(t.with_timezone(&Local)))
    }
}

/// `#rrggbb` → `0xRRGGBB`.
fn parse_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

impl<T> Page<T> {
    /// `None` on the last page (Google may send an empty token there).
    fn next_token(&self) -> Option<String> {
        self.next_page_token.clone().filter(|t| !t.is_empty())
    }
}

/// Network path: a non-2xx status maps through `http_error`, and a 2xx body
/// that is not a page of `T` (a captive portal's HTML, say) is an error, never
/// an empty page that would replace the cached events.
fn read_page<T: DeserializeOwned>(status: u16, body: &str) -> Result<Page<T>, AuthError> {
    if !(200..300).contains(&status) {
        return Err(http_error(status, body));
    }
    serde_json::from_str(body).map_err(|_| unreadable(status))
}

fn unreadable(status: u16) -> AuthError {
    AuthError::Network(format!("HTTP {status}: jawaban tidak terbaca"))
}

/// Lenient parse for the pure helpers: anything unreadable is an empty page.
fn items<T: DeserializeOwned>(json: &str) -> Vec<T> {
    serde_json::from_str::<Page<T>>(json)
        .map(|page| page.items)
        .unwrap_or_default()
}

/// Calendars marked `selected` (shown in Google Calendar): id, color, and
/// whether the user may write to it (`accessRole` owner or writer).
pub fn parse_calendar_list(json: &str) -> Vec<CalendarEntry> {
    calendars_from(items(json))
}

fn calendars_from(entries: Vec<ListEntry>) -> Vec<CalendarEntry> {
    entries
        .into_iter()
        .filter(|c| c.selected)
        .map(|c| {
            let writable = matches!(c.access_role.as_str(), "owner" | "writer");
            let color = c.background_color.as_deref().and_then(parse_color);
            (c.id, color, writable)
        })
        .collect()
}

/// Events of one events.list page; cancelled and unreadable ones are skipped.
pub fn parse_events(
    json: &str,
    calendar_id: &str,
    color: Option<u32>,
    writable: bool,
) -> Vec<CalEvent> {
    events_from(items(json), calendar_id, color, writable)
}

fn events_from(
    events: Vec<ApiEvent>,
    calendar_id: &str,
    color: Option<u32>,
    writable: bool,
) -> Vec<CalEvent> {
    events
        .into_iter()
        .filter(|e| e.status != "cancelled")
        .filter_map(|e| {
            let start = e.start.as_ref()?.to_event_time()?;
            let end = e.end.as_ref()?.to_event_time()?;
            // Editing an expanded instance of a recurring event is out of scope.
            let editable = writable && e.recurring_event_id.is_none() && fits_draft(&start, &end);
            let title = if e.summary.trim().is_empty() {
                NO_TITLE.to_string()
            } else {
                e.summary
            };
            Some(CalEvent {
                id: e.id,
                calendar_id: calendar_id.to_string(),
                title,
                start,
                end,
                description: e.description,
                color,
                editable,
            })
        })
        .collect()
}

/// Local wall-clock time with its UTC offset. A time skipped by a DST jump
/// moves forward an hour, as Google Calendar does.
fn local_at(date: NaiveDate, time: NaiveTime) -> DateTime<Local> {
    let naive = date.and_time(time);
    naive
        .and_local_timezone(Local)
        .earliest()
        .or_else(|| {
            (naive + TimeDelta::hours(1))
                .and_local_timezone(Local)
                .earliest()
        })
        .unwrap_or_else(|| naive.and_utc().with_timezone(&Local))
}

fn rfc3339(t: DateTime<Local>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn day_start(date: NaiveDate) -> String {
    rfc3339(local_at(date, NaiveTime::MIN))
}

/// The events.insert / events.patch body for a draft. All-day ends are
/// exclusive (`date + 1`); timed events carry the local UTC offset.
pub fn draft_body(d: &EventDraft) -> Value {
    let (start, end) = if d.all_day {
        let next = d.date.succ_opt().unwrap_or(d.date);
        (
            json!({ "date": d.date.to_string() }),
            json!({ "date": next.to_string() }),
        )
    } else {
        (
            json!({ "dateTime": rfc3339(local_at(d.date, d.start)) }),
            json!({ "dateTime": rfc3339(local_at(d.date, d.end)) }),
        )
    };
    json!({
        "summary": d.title,
        "description": d.description,
        "start": start,
        "end": end,
    })
}

/// Patch merges nested objects, so switching between all-day and timed must
/// clear the other field explicitly (`null` deletes it).
fn patch_body(d: &EventDraft) -> Value {
    let mut body = draft_body(d);
    let cleared = if d.all_day { "dateTime" } else { "date" };
    for key in ["start", "end"] {
        if let Some(obj) = body[key].as_object_mut() {
            obj.insert(cleared.into(), Value::Null);
            if d.all_day {
                obj.insert("timeZone".into(), Value::Null);
            }
        }
    }
    body
}

fn network(e: windows::core::Error) -> AuthError {
    AuthError::Network(e.message())
}

/// Maps a non-2xx answer using only Google's `error.message`, never the
/// rest of the body.
fn http_error(status: u16, body: &str) -> AuthError {
    if status == 401 {
        return AuthError::Reconnect;
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .filter(|m| !m.is_empty());
    match message {
        Some(m) => AuthError::Network(format!(
            "HTTP {status}: {}",
            m.chars().take(120).collect::<String>()
        )),
        None => AuthError::Network(format!("HTTP {status}")),
    }
}

/// An insert/patch answer must be the saved event; a 2xx page that is not
/// (a captive portal again) means the save never reached Google.
fn check_saved((status, body): (u16, String)) -> Result<(), AuthError> {
    #[derive(Deserialize)]
    struct Saved {
        #[allow(dead_code)] // Only its presence is checked.
        id: String,
    }
    if !(200..300).contains(&status) {
        return Err(http_error(status, &body));
    }
    serde_json::from_str::<Saved>(&body)
        .map(drop)
        .map_err(|_| unreadable(status))
}

fn get_page<T: DeserializeOwned>(
    http: &Http,
    token: &str,
    url: &str,
) -> Result<Page<T>, AuthError> {
    let (status, body) = http.get_json(url, Some(token)).map_err(network)?;
    read_page(status, &body)
}

pub fn list_calendars(http: &Http, token: &str) -> Result<Vec<CalendarEntry>, AuthError> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![("maxResults", "250")];
        if let Some(t) = &page_token {
            query.push(("pageToken", t));
        }
        let url = format!("{BASE}/users/me/calendarList?{}", form_encode(&query));
        let page = get_page::<ListEntry>(http, token, &url)?;
        page_token = page.next_token();
        out.extend(calendars_from(page.items));
        if page_token.is_none() {
            return Ok(out);
        }
    }
}

/// Single (expanded) events overlapping local dates `[from, to)`.
pub fn list_events(
    http: &Http,
    token: &str,
    cal: &CalendarEntry,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalEvent>, AuthError> {
    let (cal_id, color, writable) = cal;
    let (time_min, time_max) = (day_start(from), day_start(to));
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("timeMin", time_min.as_str()),
            ("timeMax", time_max.as_str()),
            ("maxResults", PAGE_SIZE),
        ];
        if let Some(t) = &page_token {
            query.push(("pageToken", t));
        }
        let url = format!(
            "{BASE}/calendars/{}/events?{}",
            url_encode(cal_id),
            form_encode(&query)
        );
        let page = get_page::<ApiEvent>(http, token, &url)?;
        page_token = page.next_token();
        out.extend(events_from(page.items, cal_id, *color, *writable));
        if page_token.is_none() {
            return Ok(out);
        }
    }
}

/// Events from every selected calendar, sorted by start.
pub fn fetch_range(
    http: &Http,
    token: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalEvent>, AuthError> {
    let mut events = Vec::new();
    for cal in list_calendars(http, token)? {
        events.extend(list_events(http, token, &cal, from, to)?);
    }
    events.sort_by_key(|e| e.start.naive_local());
    Ok(events)
}

/// Creates the event in the user's primary calendar.
pub fn insert_event(http: &Http, token: &str, draft: &EventDraft) -> Result<(), AuthError> {
    let url = format!("{BASE}/calendars/primary/events");
    let body = draft_body(draft).to_string();
    check_saved(
        http.send_json("POST", &url, Some(token), &body)
            .map_err(network)?,
    )
}

pub fn patch_event(
    http: &Http,
    token: &str,
    cal_id: &str,
    event_id: &str,
    draft: &EventDraft,
) -> Result<(), AuthError> {
    let url = format!(
        "{BASE}/calendars/{}/events/{}",
        url_encode(cal_id),
        url_encode(event_id)
    );
    let body = patch_body(draft).to_string();
    check_saved(
        http.send_json("PATCH", &url, Some(token), &body)
            .map_err(network)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::sync::merge_fetch;
    use crate::calendar::{CalStatus, CalendarSnapshot};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn draft(all_day: bool) -> EventDraft {
        EventDraft {
            title: "Rapat".into(),
            date: date(2026, 10, 26),
            all_day,
            start: hm(9, 0),
            end: hm(10, 30),
            description: "catatan".into(),
        }
    }

    const EVENTS: &str = r##"{
      "kind": "calendar#events",
      "nextPageToken": "",
      "items": [
        {"id": "a", "status": "confirmed", "summary": "Libur",
         "start": {"date": "2026-10-26"}, "end": {"date": "2026-10-29"}},
        {"id": "b", "status": "cancelled", "summary": "Batal",
         "start": {"dateTime": "2026-10-25T09:00:00+07:00"},
         "end": {"dateTime": "2026-10-25T10:00:00+07:00"}},
        {"id": "c", "summary": "Telepon", "description": "bawa laptop",
         "start": {"dateTime": "2026-10-25T10:00:00-07:00", "timeZone": "America/Los_Angeles"},
         "end": {"dateTime": "2026-10-25T10:30:00-07:00"}},
        {"id": "d", "status": "cancelled"},
        {"id": "e", "status": "confirmed",
         "start": {"date": "2026-10-27"}, "end": {"date": "2026-10-28"}}
      ]
    }"##;

    #[test]
    fn cancelled_events_skipped() {
        let evs = parse_events(EVENTS, "cal1", Some(0x4F8CFF), true);
        let ids: Vec<&str> = evs.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "e"]);
        assert!(evs.iter().all(|e| e.calendar_id == "cal1"));
        assert!(evs.iter().all(|e| e.color == Some(0x4F8CFF)));
    }

    #[test]
    fn parsed_fields_and_editability() {
        let evs = parse_events(EVENTS, "cal1", None, true);
        let a = &evs[0];
        assert_eq!(a.title, "Libur");
        assert_eq!(a.start, EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(a.end, EventTime::AllDay(date(2026, 10, 29)));
        assert!(!a.editable, "a three-day event does not fit a draft");
        assert_eq!(evs[1].description, "bawa laptop");
        assert_eq!(evs[2].title, NO_TITLE);
        assert!(evs[2].editable);
        assert!(
            parse_events(EVENTS, "cal1", None, false)
                .iter()
                .all(|e| !e.editable),
            "read-only calendar"
        );
    }

    #[test]
    fn offset_times_convert_to_local() {
        let evs = parse_events(EVENTS, "cal1", None, true);
        let c = evs.iter().find(|e| e.id == "c").unwrap();
        let expected = DateTime::parse_from_rfc3339("2026-10-25T10:00:00-07:00")
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(c.start, EventTime::At(expected));
        assert_eq!(
            super::super::events_on(&evs, expected.date_naive())
                .iter()
                .filter(|e| e.id == "c")
                .count(),
            1,
            "listed on the local date"
        );
        let EventTime::At(t) = &c.start else {
            panic!("timed")
        };
        assert_eq!(t.naive_local(), expected.naive_local());
    }

    #[test]
    fn garbage_json_gives_no_events() {
        assert!(parse_events("not json", "c", None, true).is_empty());
        assert!(parse_events("{}", "c", None, true).is_empty());
        assert!(parse_calendar_list("<html>").is_empty());
    }

    #[test]
    fn recurring_instances_are_read_only() {
        let json = r##"{
          "kind": "calendar#events",
          "items": [
            {"id": "weekly_20261026T020000Z", "recurringEventId": "weekly",
             "summary": "Standup",
             "start": {"dateTime": "2026-10-26T09:00:00+07:00"},
             "end": {"dateTime": "2026-10-26T09:15:00+07:00"}},
            {"id": "once", "summary": "Sekali",
             "start": {"dateTime": "2026-10-26T09:00:00+07:00"},
             "end": {"dateTime": "2026-10-26T09:15:00+07:00"}}
          ]
        }"##;
        let evs = parse_events(json, "cal1", None, true);
        assert_eq!(evs.len(), 2);
        assert!(!evs[0].editable, "expanded recurring instance");
        assert!(evs[1].editable);
    }

    #[test]
    fn unreadable_2xx_body_is_an_error_and_keeps_the_cache() {
        let html = "<html><body>Masuk ke Wi-Fi hotel</body></html>";
        let err = read_page::<ApiEvent>(200, html).err().unwrap();
        assert_eq!(
            err,
            AuthError::Network("HTTP 200: jawaban tidak terbaca".into())
        );
        assert!(read_page::<ListEntry>(200, "").is_err());
        assert!(read_page::<ListEntry>(204, "<html>").is_err());
        assert_eq!(
            read_page::<ApiEvent>(401, "").err(),
            Some(AuthError::Reconnect)
        );
        assert_eq!(
            read_page::<ApiEvent>(200, EVENTS).ok().unwrap().items.len(),
            5
        );

        let prev = CalendarSnapshot {
            status: CalStatus::Ok,
            events: parse_events(EVENTS, "cal1", None, true),
            from: date(2026, 10, 19),
            to: date(2026, 10, 26),
            fetched_at: Some(Local::now()),
        };
        let merged = merge_fetch(&prev, Err(err), prev.from, prev.to, Local::now());
        assert_eq!(merged.status, CalStatus::Offline);
        assert_eq!(merged.events, prev.events);
        assert_eq!(merged.fetched_at, prev.fetched_at);
    }

    #[test]
    fn save_needs_the_saved_event_back() {
        assert_eq!(
            check_saved((200, r#"{"id":"abc","kind":"calendar#event"}"#.into())),
            Ok(())
        );
        assert_eq!(
            check_saved((200, "<html>".into())),
            Err(AuthError::Network("HTTP 200: jawaban tidak terbaca".into()))
        );
        assert_eq!(check_saved((401, String::new())), Err(AuthError::Reconnect));
    }

    #[test]
    fn calendar_list_keeps_selected_only() {
        let json = r##"{
          "items": [
            {"id": "me@example.com", "selected": true, "accessRole": "owner",
             "backgroundColor": "#9fe1e7", "primary": true},
            {"id": "hidden", "selected": false, "accessRole": "owner"},
            {"id": "no-flag", "accessRole": "writer"},
            {"id": "holidays", "selected": true, "accessRole": "reader",
             "backgroundColor": "#16a765"},
            {"id": "team", "selected": true, "accessRole": "writer"}
          ],
          "nextPageToken": "p2"
        }"##;
        assert_eq!(
            parse_calendar_list(json),
            vec![
                ("me@example.com".to_string(), Some(0x9FE1E7), true),
                ("holidays".to_string(), Some(0x16A765), false),
                ("team".to_string(), None, true),
            ]
        );
        let page = read_page::<ListEntry>(200, json).ok().unwrap();
        assert_eq!(page.next_token().as_deref(), Some("p2"));
        let page = read_page::<ApiEvent>(200, EVENTS).ok().unwrap();
        assert_eq!(page.next_token(), None, "empty token ends paging");
    }

    #[test]
    fn draft_body_all_day_end_exclusive() {
        let body = draft_body(&draft(true));
        assert_eq!(body["summary"], "Rapat");
        assert_eq!(body["description"], "catatan");
        assert_eq!(body["start"], json!({ "date": "2026-10-26" }));
        assert_eq!(body["end"], json!({ "date": "2026-10-27" }));
    }

    #[test]
    fn draft_body_timed_has_offset() {
        let body = draft_body(&draft(false));
        let start = body["start"]["dateTime"].as_str().unwrap();
        let end = body["end"]["dateTime"].as_str().unwrap();
        assert!(body["start"].get("date").is_none());
        let expected = local_at(date(2026, 10, 26), hm(9, 0));
        let parsed = DateTime::parse_from_rfc3339(start).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.offset(), expected.fixed_offset().offset());
        assert!(!start.ends_with('Z'), "explicit offset, not Z: {start}");
        assert!(start.starts_with("2026-10-26T09:00:00"), "{start}");
        assert!(end.starts_with("2026-10-26T10:30:00"), "{end}");
    }

    #[test]
    fn patch_body_clears_the_other_time_kind() {
        let timed = patch_body(&draft(false));
        assert_eq!(timed["start"]["date"], Value::Null);
        assert!(timed["start"].as_object().unwrap().contains_key("date"));
        let all_day = patch_body(&draft(true));
        assert!(
            all_day["end"]
                .as_object()
                .unwrap()
                .get("dateTime")
                .is_some_and(Value::is_null)
        );
        assert_eq!(all_day["end"]["date"], "2026-10-27");
    }

    #[test]
    fn http_errors_map_to_auth_errors() {
        assert_eq!(http_error(401, ""), AuthError::Reconnect);
        assert_eq!(
            http_error(
                403,
                r#"{"error":{"code":403,"message":"Forbidden","errors":[]}}"#
            ),
            AuthError::Network("HTTP 403: Forbidden".into())
        );
        assert_eq!(
            http_error(500, "<html>"),
            AuthError::Network("HTTP 500".into())
        );
    }
}
