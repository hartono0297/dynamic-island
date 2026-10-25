//! Google Calendar API v3: JSON parsing (pure) and the blocking calls the
//! sync thread makes. Errors reuse `AuthError`: 401 is `Unauthorized` (the
//! sync thread refreshes and retries), a token missing a scope is `Reconnect`,
//! other refusals are `Google`, and 5xx, rate limits and transport failures
//! are `Network`.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, SecondsFormat, TimeDelta};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{CalEvent, EventDraft, EventTime, fits_draft};
use crate::google::http::{Http, form_encode, url_encode};
use crate::google::oauth::{AuthError, short};
use crate::i18n::{self, Key, tr};

const BASE: &str = "https://www.googleapis.com/calendar/v3";
/// Google's maximum page size for events.list.
const PAGE_SIZE: &str = "2500";

/// A calendar the user shows in Google Calendar.
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarEntry {
    pub id: String,
    /// `0xRRGGBB`.
    pub color: Option<u32>,
    /// The user may write to it (`accessRole` owner or writer).
    pub writable: bool,
    /// The user's own calendar.
    pub primary: bool,
}

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
    #[serde(default)]
    primary: bool,
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
    AuthError::Network(i18n::unreadable_reply(i18n::lang(), status))
}

/// Lenient parse for the pure helpers: anything unreadable is an empty page.
/// The pure helpers exist for the tests; the app reads pages with `read_page`.
#[cfg(test)]
fn items<T: DeserializeOwned>(json: &str) -> Vec<T> {
    serde_json::from_str::<Page<T>>(json)
        .map(|page| page.items)
        .unwrap_or_default()
}

/// Calendars marked `selected` (shown in Google Calendar): id, color, and
/// whether the user may write to it (`accessRole` owner or writer).
#[cfg(test)]
pub fn parse_calendar_list(json: &str) -> Vec<CalendarEntry> {
    calendars_from(items(json))
}

fn calendars_from(entries: Vec<ListEntry>) -> Vec<CalendarEntry> {
    entries
        .into_iter()
        .filter(|c| c.selected)
        .map(|c| CalendarEntry {
            writable: matches!(c.access_role.as_str(), "owner" | "writer"),
            color: c.background_color.as_deref().and_then(parse_color),
            primary: c.primary,
            id: c.id,
        })
        .collect()
}

/// Events of one events.list page; cancelled and unreadable ones are skipped.
#[cfg(test)]
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
            // Kept empty when Google has no summary: the panel draws
            // "(Tanpa judul)" in the current language (`CalEvent::shown_title`).
            let title = if e.summary.trim().is_empty() {
                String::new()
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

/// Maps a non-2xx answer using only Google's `error.message` and the error
/// reasons (`error.errors[].reason`, `error.details[].reason`), never the
/// rest of the body.
fn http_error(status: u16, body: &str) -> AuthError {
    if status == 401 {
        return AuthError::Unauthorized;
    }
    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    let error = &parsed["error"];
    let message = error["message"].as_str().unwrap_or("").trim();
    let reasons: Vec<&str> = ["errors", "details"]
        .iter()
        .filter_map(|list| error[*list].as_array())
        .flatten()
        .filter_map(|item| item["reason"].as_str())
        .collect();
    let has = |reason: &str| reasons.contains(&reason);
    if status == 403
        && (has("insufficientPermissions")
            || has("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
            || message
                .to_ascii_lowercase()
                .contains("insufficient authentication scopes"))
    {
        // The token lacks a scope: only a new login with both ticked helps.
        return AuthError::Reconnect;
    }
    if status == 403 && (has("accessNotConfigured") || has("SERVICE_DISABLED")) {
        // The Calendar API is off in the owner's Google Cloud project.
        return AuthError::Google(tr(Key::ApiDisabled).into());
    }
    // Rate limits pass; they read as offline until the next fetch.
    let rate_limited = status == 429
        || (status == 403 && (has("rateLimitExceeded") || has("userRateLimitExceeded")));
    let text = if message.is_empty() {
        format!("HTTP {status}")
    } else {
        short(message)
    };
    if (400..500).contains(&status) && !rate_limited {
        AuthError::Google(text)
    } else if message.is_empty() {
        AuthError::Network(text)
    } else {
        AuthError::Network(format!("HTTP {status}: {text}"))
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
    let cal_id = &cal.id;
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
        out.extend(events_from(page.items, cal_id, cal.color, cal.writable));
        if page_token.is_none() {
            return Ok(out);
        }
    }
}

/// An error that says the login itself is no good: every other call would
/// fail the same way.
fn is_auth(e: &AuthError) -> bool {
    matches!(
        e,
        AuthError::NotConfigured
            | AuthError::NeedsLogin
            | AuthError::Reconnect
            | AuthError::MissingScopes
            | AuthError::Unauthorized
    )
}

/// Events from every selected calendar, sorted by start. A secondary
/// calendar that fails (deleted meanwhile, not shared any more) is left out
/// while another calendar loads; an auth error stops the whole fetch.
pub fn fetch_range(
    http: &Http,
    token: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalEvent>, AuthError> {
    let mut results = Vec::new();
    for cal in list_calendars(http, token)? {
        let result = list_events(http, token, &cal, from, to);
        if let Err(e) = &result
            && is_auth(e)
        {
            return Err(result.unwrap_err());
        }
        results.push((cal.primary, result));
    }
    combine_calendars(results)
}

/// The events of every calendar's result (`primary`, events or error),
/// sorted by start. Fails with the first auth error, a failed primary
/// calendar's error, or the first error when no calendar loaded at all;
/// a failed secondary calendar is otherwise skipped.
fn combine_calendars(
    results: Vec<(bool, Result<Vec<CalEvent>, AuthError>)>,
) -> Result<Vec<CalEvent>, AuthError> {
    let any_ok = results.iter().any(|(_, r)| r.is_ok());
    let errors = || {
        results
            .iter()
            .filter_map(|(primary, r)| Some((*primary, r.as_ref().err()?)))
    };
    let fatal = errors()
        .find(|(_, e)| is_auth(e))
        .or_else(|| errors().find(|(primary, _)| *primary || !any_ok));
    if let Some((_, e)) = fatal {
        return Err(e.clone());
    }
    let mut events: Vec<CalEvent> = results
        .into_iter()
        .filter_map(|(_, r)| r.ok())
        .flatten()
        .collect();
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
        assert_eq!(evs[2].title, "");
        assert_eq!(evs[2].shown_title(), "(Tanpa judul)");
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
            Some(AuthError::Unauthorized)
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
        assert_eq!(
            check_saved((401, String::new())),
            Err(AuthError::Unauthorized)
        );
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
        let cal = |id: &str, color, writable, primary| CalendarEntry {
            id: id.into(),
            color,
            writable,
            primary,
        };
        assert_eq!(
            parse_calendar_list(json),
            vec![
                cal("me@example.com", Some(0x9FE1E7), true, true),
                cal("holidays", Some(0x16A765), false, false),
                cal("team", None, true, false),
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
        assert_eq!(http_error(401, ""), AuthError::Unauthorized);
        assert_eq!(
            http_error(
                403,
                r#"{"error":{"code":403,"message":"Forbidden","errors":[]}}"#
            ),
            AuthError::Google("Forbidden".into())
        );
        assert_eq!(
            http_error(404, r#"{"error":{"code":404,"message":"Not Found"}}"#),
            AuthError::Google("Not Found".into())
        );
        assert_eq!(
            http_error(400, "<html>"),
            AuthError::Google("HTTP 400".into())
        );
        assert_eq!(
            http_error(500, "<html>"),
            AuthError::Network("HTTP 500".into())
        );
        assert_eq!(
            http_error(503, r#"{"error":{"message":"Backend Error"}}"#),
            AuthError::Network("HTTP 503: Backend Error".into())
        );
        let long = format!(r#"{{"error":{{"message":"{}"}}}}"#, "x".repeat(300));
        let AuthError::Google(m) = http_error(400, &long) else {
            panic!("not a Google error")
        };
        assert_eq!(m.chars().count(), 120);
    }

    #[test]
    fn missing_scope_403_means_reconnect() {
        // Google's answer to a token without the calendar.events scope.
        let scope = r#"{"error":{"code":403,
          "message":"Request had insufficient authentication scopes.",
          "errors":[{"message":"Insufficient Permission","domain":"global","reason":"insufficientPermissions"}],
          "status":"PERMISSION_DENIED",
          "details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo",
            "reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT","domain":"googleapis.com"}]}}"#;
        assert_eq!(http_error(403, scope), AuthError::Reconnect);
        let details_only = r#"{"error":{"code":403,"message":"x",
          "details":[{"reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#;
        assert_eq!(http_error(403, details_only), AuthError::Reconnect);
        let message_only =
            r#"{"error":{"code":403,"message":"Request had insufficient authentication scopes."}}"#;
        assert_eq!(http_error(403, message_only), AuthError::Reconnect);
        let errors_only = r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#;
        assert_eq!(http_error(403, errors_only), AuthError::Reconnect);
    }

    #[test]
    fn disabled_calendar_api_is_a_google_refusal() {
        let disabled = r#"{"error":{"code":403,
          "message":"Google Calendar API has not been used in project 123 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/calendar-json.googleapis.com/overview?project=123 then retry.",
          "errors":[{"message":"...","domain":"usageLimits","reason":"accessNotConfigured","extendedHelp":"https://console.developers.google.com"}],
          "status":"PERMISSION_DENIED",
          "details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SERVICE_DISABLED"}]}}"#;
        assert_eq!(
            http_error(403, disabled),
            AuthError::Google(tr(Key::ApiDisabled).into())
        );
        let details_only = r#"{"error":{"message":"x","details":[{"reason":"SERVICE_DISABLED"}]}}"#;
        assert_eq!(
            http_error(403, details_only),
            AuthError::Google(tr(Key::ApiDisabled).into())
        );
    }

    #[test]
    fn rate_limits_stay_offline() {
        let limited = r#"{"error":{"code":403,"message":"Rate Limit Exceeded",
          "errors":[{"reason":"rateLimitExceeded"}]}}"#;
        assert_eq!(
            http_error(403, limited),
            AuthError::Network("HTTP 403: Rate Limit Exceeded".into())
        );
        assert_eq!(http_error(429, ""), AuthError::Network("HTTP 429".into()));
    }

    fn one_event(id: &str, day: u32) -> CalEvent {
        CalEvent {
            id: id.into(),
            calendar_id: id.into(),
            title: id.into(),
            start: EventTime::AllDay(date(2026, 10, day)),
            end: EventTime::AllDay(date(2026, 10, day + 1)),
            description: String::new(),
            color: None,
            editable: false,
        }
    }

    fn ids(result: &Result<Vec<CalEvent>, AuthError>) -> Vec<String> {
        result
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.id.clone())
            .collect()
    }

    #[test]
    fn a_failing_secondary_calendar_is_skipped() {
        let gone = || Err(AuthError::Google("Not Found".into()));
        let offline = || Err(AuthError::Network("HTTP 500".into()));
        let result = combine_calendars(vec![
            (true, Ok(vec![one_event("mine", 27)])),
            (false, gone()),
            (false, Ok(vec![one_event("team", 26)])),
            (false, offline()),
        ]);
        assert_eq!(ids(&result), ["team", "mine"], "sorted by start");

        // Only a secondary calendar, and it works.
        let result = combine_calendars(vec![(false, Ok(vec![])), (false, gone())]);
        assert_eq!(ids(&result), Vec::<String>::new());
        assert_eq!(combine_calendars(vec![]), Ok(vec![]));
    }

    #[test]
    fn primary_auth_or_total_failures_fail_the_fetch() {
        let ok = || Ok(vec![one_event("x", 26)]);
        // The primary calendar failed.
        assert_eq!(
            combine_calendars(vec![
                (true, Err(AuthError::Network("HTTP 500".into()))),
                (false, ok()),
            ]),
            Err(AuthError::Network("HTTP 500".into()))
        );
        // Nothing loaded.
        assert_eq!(
            combine_calendars(vec![
                (false, Err(AuthError::Google("Not Found".into()))),
                (false, Err(AuthError::Network("HTTP 500".into()))),
            ]),
            Err(AuthError::Google("Not Found".into()))
        );
        // Auth errors win over everything, even on a secondary calendar.
        for auth in [
            AuthError::Reconnect,
            AuthError::Unauthorized,
            AuthError::NeedsLogin,
            AuthError::MissingScopes,
        ] {
            assert_eq!(
                combine_calendars(vec![
                    (true, Err(AuthError::Network("HTTP 500".into()))),
                    (false, ok()),
                    (false, Err(auth.clone())),
                ]),
                Err(auth)
            );
        }
    }
}
