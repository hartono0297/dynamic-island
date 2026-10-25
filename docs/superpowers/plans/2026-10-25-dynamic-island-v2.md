# Dynamic Island v2 (Panel, Google Calendar, Pomodoro, Notes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the v1 island into an always-visible notch that opens a scrollable panel with Google Calendar (day/week/month, add/edit), a Pomodoro with progress rings, local notes, and media — with smoother animation.

**Architecture:** Same single process as v1. The pure reducer `IslandState` grows a `Pomodoro` sub-machine, panel pages and calendar navigation state; it still returns `Effect`s the UI thread performs. Data that is not UI state lives outside the reducer and is owned by `App`: `NotesStore` (local JSON) and the latest `CalendarSnapshot` (sent by a Google sync thread). The renderer draws `View` + those data; hit regions come from pure layout functions. Text input happens in separate ordinary Win32 editor windows (notes, event), never inside the layered island.

**Tech Stack:** Rust stable (MSVC), `windows` 0.62 (Win32 + WinRT incl. `Windows.Web.Http`), `serde`/`serde_json`, `chrono` 0.4 (`default-features = false`, features `clock`, `std`, `serde`). No other new runtime deps.

**Spec:** `docs/superpowers/specs/2026-10-25-dynamic-island-v2-panel-design.md` (Indonesian) — extends `docs/superpowers/specs/2026-10-25-dynamic-island-design.md`. Read both before any task.

## Global Constraints

- Everything from v1 still holds unless the v2 spec changes it (notifications via official listener only; never change Windows settings; `~/.claude/settings.json` never printed; Indonesian UI strings; primary monitor; `panic = "abort"`, so no panics reachable in `extern "system"` paths).
- Both styles stay: `Pill` = floating 8 px below top, all corners round; `Notch` = flush with top edge, bottom corners round, 12 px inward shoulders. Every new shape (idle, pomodoro compact, panel) follows the chosen style.
- Island hidden only in fullscreen; otherwise an empty black shape (idle) is always visible. A new notice still shows briefly in fullscreen.
- Panel opens on hover dwell (350 ms, v1 behaviour); wheel scroll while the panel is open changes page; page order fixed: Kalender, Pomodoro, Notes, Musik. Panel reopens on the last page viewed this session.
- Small-view priority: temporary notice → Claude waiting → pomodoro active (running, paused or ready) → media → idle.
- A temporary notice (5 s) blocks the panel; Claude waiting does not (shown as a strip at the top of the open panel).
- While a notes/event editor window is open, the panel stays open.
- Google: OAuth 2.0 desktop flow, loopback `http://127.0.0.1:<port>`, PKCE S256, `access_type=offline`, `prompt=consent`. Scope minimum for: list selected calendars, list events, insert/patch events. Tokens encrypted with DPAPI; never logged. Expired/revoked token → visible "Hubungkan ulang Google" (Testing-status refresh tokens expire after 7 days).
- Calendar shows events from calendars marked `selected` in the owner's Google Calendar; new events go to `primary`. No delete, no attendees, no creating/editing recurrence.
- Pomodoro defaults 25 / 5 / 15 min, 4 sessions; next phase starts only on click; one Windows notification sound when a phase ends.
- Notes stored only locally in `%APPDATA%\DynamicIsland\notes.json`.
- Idle cost: no redraw at idle; a running pomodoro redraws at most once per second while small. RAM stays in the tens of MB.
- Owner request (2026-10-25, after plan approval): the equalizer bars in the media compact view **animate while media is playing** (static when paused). This is the only continuous redraw allowed: ~30 fps, only while `MediaCompact` is the visible view and `playing == true`; it stops in every other view, when paused, and in fullscreen. Measure its CPU cost in Task 10.

### Values pinned by this plan (spec leaves them to the builder)

| Name | Value |
|---|---|
| Window size (logical px) | 620 × 330 |
| Shapes (w × h) | idle 190 × 34; pomodoro compact 210 × 34; media compact 190 × 34 (v1); notice 360 × 76 (v1); panel 560 × 290 |
| Panel radius | Pill 26 (all corners); Notch 26 (bottom corners) |
| Pomodoro steppers | focus 5–120 step 5; short break 1–30 step 1; long break 5–60 step 5; sessions 1–12 step 1 |
| Wheel | one page per accumulated ±120 delta; clamps at first/last page (no wrap) |
| Calendar sync | on start, on panel open if data older than 60 s, every 10 min, after each save |
| Calendar cache | `%APPDATA%\DynamicIsland\calendar-cache.json` |
| Token file | `%APPDATA%\DynamicIsland\google-token.bin` (DPAPI) |
| Month grid | Monday-first, 6 rows × 7 columns starting on the Monday on/before the 1st |
| Week range | Monday 00:00 to next Monday 00:00 (local) |
| Phase sound | `PlaySoundW("Notification.Default", SND_ALIAS \| SND_ASYNC \| SND_NODEFAULT)` |
| Frame clock | animation frames paced by `DwmFlush()` on a helper thread (see Task 1) |
| Smoothness bar | during any morph, no frame interval > 25 ms on a 60 Hz display (excluding the first frame) |
| Equalizer animation | 4 bars, each height = `BAR_MAX_H × (0.35 + 0.65 × |sin(2π·f_i·t + φ_i)|)` with f = 1.7, 2.3, 1.3, 2.9 Hz and φ = 0, 1.1, 2.3, 0.6; redraw every ~33 ms via a dedicated timer/clock only while playing in `MediaCompact` |

## Review Focus

1. **Google token expired/revoked (7-day Testing expiry) or never connected** → calendar page says "Hubungkan Google" / "Hubungkan ulang Google" and clicking it restarts login; never a silent empty calendar, never a crash. (Task 6 test `invalid_grant_maps_to_reconnect`; Task 5 test `token_response_without_refresh_keeps_old_refresh`.)
2. **Offline at start / network drops** → cached events still shown with an "offline" mark; the UI thread never blocks on HTTP. (Task 6 tests `cache_round_trip`, `fetch_error_keeps_cached_events_offline`.)
3. **All-day and multi-day events, events crossing midnight, other time zones in Google data** → event appears on every local date it covers; all-day `end` is exclusive. (Task 6 tests `multi_day_all_day_covers_each_date`, `timed_event_crossing_midnight_on_both_days`, `offset_times_convert_to_local`.)
4. **Many events / long titles / many notes** → rows trimmed with "…", overflow shown as "+N lagi", nothing drawn outside the panel. (Task 7 tests `day_list_overflow_shows_more_count`, `notes_list_caps_rows`.)
5. **Touchpad high-resolution wheel deltas (e.g. 10–30 per event)** → exactly one page per 120 accumulated, no skipping pages. (Task 3 test `small_wheel_deltas_accumulate_to_one_page`.)

---

## File Structure

```
Cargo.toml                       + chrono; + windows features (Web_Http*, Win32_Security_Cryptography, Win32_Media_Audio, Win32_UI_Controls, Win32_Graphics_Dwm, Win32_Networking_WinSock if needed)
app.manifest                     + Common-Controls v6 dependency (editor windows)
src/model.rs                     Page, CalView, PomoField; View/Hit/Event/Effect extended; NoticeKind::Pomodoro
src/pomodoro.rs                  NEW  pure Pomodoro state machine + PomoSettings
src/state.rs                     IslandState v2 (idle, panel, pages, wheel, pomodoro, calendar nav, editor keep-open)
src/config.rs                    + pomodoro, google_client_id, google_client_secret
src/notes.rs                     NEW  NotesStore (load/save/upsert/delete)
src/google/mod.rs                NEW
src/google/http.rs               NEW  WinRT HttpClient wrapper (GET/POST/PATCH JSON, bearer)
src/google/oauth.rs              NEW  PKCE, auth URL, loopback, token exchange/refresh, DPAPI store
src/calendar/mod.rs              NEW  CalEvent, EventTime, EventDraft, CalendarSnapshot, CalStatus, date ranges, events_on
src/calendar/api.rs              NEW  calendarList / events list / insert / patch (JSON in/out)
src/calendar/sync.rs             NEW  sync thread + cache file
src/ui/geometry.rs               window 620×330, new shapes, panel shape per style
src/ui/panel_layout.rs           NEW  pure layouts + hit regions for each page, compact pomodoro, waiting strip
src/ui/render.rs (+ render_panel.rs NEW)  drawing of idle, pomodoro compact, panel pages
src/ui/frame_clock.rs            NEW  DwmFlush-paced frame thread
src/ui/editor.rs                 NEW  note + event editor windows
src/app.rs                       wiring: wheel, editor results, sync thread, pomodoro timer, sound, Google connect
README.md                        Google setup guide, new features
```

---

### Task 1: Smooth animation and bigger window

**Files:** Modify `src/ui/geometry.rs`, `src/app.rs`; Create `src/ui/frame_clock.rs`

**Interfaces:**
- Produces: `pub const WINDOW_W: f32 = 620.0; pub const WINDOW_H: f32 = 330.0;` (all existing shapes stay centered via `(WINDOW_W - w) / 2`). `pub struct FrameClock` with `pub fn start(hwnd: isize) -> Self` (thread loops `DwmFlush()` and posts `WM_APP_FRAME` to the window while running) and `pub fn stop(&self)`; `Drop` stops and joins. A debug-only frame log: when env `DI_FRAME_LOG=1`, `app.rs` appends each frame interval in ms to `%TEMP%\di-frames.log`.

- [ ] **Step 1: Measure first.** Add the frame log, build release, run with `DI_FRAME_LOG=1`, trigger 10 morphs via `scripts\send-test-toast.ps1` (each notice morphs in and out). Record max/avg interval in the report. This is the RED evidence for the smoothness bar.
- [ ] **Step 2:** Replace `TIMER_ANIM` (`SetTimer` 16 ms) with `FrameClock`: start it when an animation starts, stop it after the final t = 1 frame is drawn (keep the v1 rule). Frame time from `Instant`, as now.
- [ ] **Step 3:** Grow the window to 620 × 330; geometry tests updated for the new centering (e.g. compact pill `x = (620-190)/2 = 215`).
- [ ] **Step 4: Verify.** Same measurement as Step 1 → no interval > 25 ms except the first frame; report before/after numbers. Idle CPU unchanged (frame thread not running when idle: check thread count drops back). Click-through outside the shape still works (`WindowFromPoint` check as in v1 Task 9). `cargo test`, clippy, fmt, release build clean.
- [ ] **Step 5: Commit** `perf: vsync-paced animation and larger island window`.

---

### Task 2: Pomodoro state machine

**Files:** Create `src/pomodoro.rs`; Modify `src/model.rs` (add `PomoField`), `src/main.rs` (`mod pomodoro;`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)] #[serde(default)] pub struct PomoSettings { pub focus_min: u32, pub short_break_min: u32, pub long_break_min: u32, pub sessions: u32 }`, `Default` = 25, 5, 15, 4.
  - `pub enum PomoField { Focus, ShortBreak, LongBreak, Sessions }` (in `model.rs`, `Clone, Copy, Debug, PartialEq`).
  - `pub enum Phase { Focus, ShortBreak, LongBreak }`; `pub enum PomoStatus { Idle, Running { ends_at: Instant }, Paused { remaining: Duration }, Ready }`.
  - `pub struct PhaseDone { pub finished: Phase, pub next: Phase }`.
  - `impl Pomodoro { pub fn new(settings: PomoSettings) -> Self; pub fn settings(&self) -> PomoSettings; pub fn phase(&self) -> Phase; pub fn session(&self) -> u32 /*1-based*/; pub fn status(&self) -> PomoStatus; pub fn is_active(&self) -> bool /*Running|Paused|Ready*/; pub fn start_pause(&mut self, now: Instant); pub fn reset(&mut self); pub fn skip(&mut self, now: Instant) -> PhaseDone; pub fn tick(&mut self, now: Instant) -> Option<PhaseDone>; pub fn remaining(&self, now: Instant) -> Duration; pub fn progress(&self, now: Instant) -> f32 /*0..=1 elapsed*/; pub fn adjust(&mut self, field: PomoField, dir: i32) -> bool /*changed*/; pub fn next_deadline(&self) -> Option<Instant> }`.

Rules: `start_pause`: Idle/Ready → Running for the current phase's full length (Paused → Running with `remaining`); Running → Paused. `tick`/`skip` at the end of a phase: advance (Focus n<N → ShortBreak; Focus n==N → LongBreak; ShortBreak → Focus n+1; LongBreak → Focus 1) and set `Ready` (not running). `reset` → Idle, Focus, session 1. `adjust` only when Idle or Ready (otherwise returns false), clamped to the pinned ranges/steps.

- [ ] **Step 1: Failing tests** (`t0 = Instant::now()`, `m(n) = Duration::from_secs(60*n)`):

```rust
#[test] fn start_runs_focus_for_full_length() { let mut p = Pomodoro::new(PomoSettings::default()); p.start_pause(t0); assert_eq!(p.remaining(t0 + m(5)), m(20)); assert_eq!(p.next_deadline(), Some(t0 + m(25))); }
#[test] fn pause_freezes_remaining() { /* start t0, pause at t0+m(10) → Paused{m(15)}; remaining at t0+m(40) == m(15); resume at t0+m(40) → deadline t0+m(55) */ }
#[test] fn focus_end_goes_to_short_break_ready() { /* tick at t0+m(25) → Some(PhaseDone{Focus, ShortBreak}); status Ready; not running */ }
#[test] fn last_focus_goes_to_long_break_then_session_one() { /* sessions=2: F1→SB→F2→LB→F1, session numbers 1,1,2,2,1 */ }
#[test] fn tick_before_end_does_nothing() { /* tick at t0+m(24) → None */ }
#[test] fn skip_advances_immediately() { /* running focus, skip → PhaseDone{Focus,ShortBreak}, Ready */ }
#[test] fn reset_returns_to_idle_focus_one() { }
#[test] fn adjust_clamps_and_steps() { /* focus 25 → +1 → 30; set to 120 → +1 stays 120 (false); short 1 → -1 stays 1; sessions 12 → +1 stays 12 */ }
#[test] fn adjust_ignored_while_running() { /* running → adjust returns false, settings unchanged */ }
#[test] fn progress_is_elapsed_fraction() { /* at t0+m(5) of 25 → 0.2 (±1e-3) */ }
```

- [ ] **Step 2:** `cargo test pomodoro::` → FAIL. **Step 3:** implement. **Step 4:** → PASS.
- [ ] **Step 5: Commit** `feat: pomodoro state machine`.

---

### Task 3: Island state v2 (idle, panel, pages, wheel, pomodoro, calendar navigation)

**Files:** Modify `src/model.rs`, `src/state.rs`; Modify `src/ui/render.rs`/`src/app.rs` only as needed to keep compiling (renaming `MediaExpanded` uses to the new panel view; drawing of new views comes in Task 8 — until then the renderer may draw `Panel(_)` as the v1 expanded media layout and `Idle` as an empty compact shape).

**Interfaces:**
- Consumes: `Pomodoro`, `PomoSettings`, `PomoField`, `PhaseDone` (Task 2).
- Produces (`model.rs`):
  - `pub enum Page { Calendar, Pomodoro, Notes, Media }` (in that order; `Copy, Eq`); `pub enum CalView { Day, Week, Month }` (`Copy, Eq`).
  - `NoticeKind::Pomodoro` (no fields).
  - `View`: `Hidden` (fullscreen only), `Idle`, `MediaCompact`, `PomodoroCompact`, `Panel(Page)`, `Notice(Notice)` — `MediaExpanded` is removed (media moves to `Panel(Page::Media)`).
  - `Hit` gains: `CalPrev, CalNext, CalViewSel(CalView), CalDate(chrono::NaiveDate), CalEvent(String /*event id*/), CalMore(chrono::NaiveDate), CalAdd, CalConnect, PomoStartPause, PomoReset, PomoSkip, PomoAdjust(PomoField, i32), NoteNew, NoteOpen(String), NotesMore`.
  - This task adds `chrono = { version = "0.4", default-features = false, features = ["clock", "std", "serde"] }` to `Cargo.toml`.
  - `Event` gains: `Wheel(i32)` (raw `WM_MOUSEWHEEL` delta), `EditorOpen(bool)`, `Today(chrono::NaiveDate)` (date rollover).
  - `Effect` gains: `PlaySound`, `SavePomodoro(PomoSettings)`, `ConnectGoogle`, `RefreshCalendar { from: NaiveDate, to: NaiveDate }`, `OpenNoteEditor(Option<String>)`, `OpenEventEditor { event_id: Option<String>, date: NaiveDate }`.
- Produces (`state.rs`): `IslandState::new(today: NaiveDate, pomo: PomoSettings) -> Self`; existing `apply`, `view`, `media`, `next_deadline` (now also the pomodoro deadline); new `pub fn page(&self) -> Page`, `pub fn waiting_strip(&self) -> Option<&Notice>` (Some only when `view()` is `Panel(_)` and a Claude waiting exists), `pub fn pomodoro(&self) -> &Pomodoro`, `pub fn cal_view(&self) -> CalView`, `pub fn cal_anchor(&self) -> NaiveDate`, `pub fn cal_selected(&self) -> NaiveDate`, `pub fn cal_range(&self) -> (NaiveDate, NaiveDate)` (delegates to `calendar::range_for`, Task 6 — until Task 6 exists, implement `range_for` in `state.rs` and Task 6 moves it).

Rules (in addition to v1): see Global Constraints for priority/fullscreen/blocking. `panel_open = hover || editor_open`. `Wheel(d)`: only while `Panel(_)` is the view; accumulate; each full ±120 moves one page (negative delta = next page, i.e. scrolling down goes right), clamped; leftover kept, reset when the panel closes. Pomodoro: `Tick` calls `pomodoro.tick(now)`; a `PhaseDone` queues a `Notice { source: "Pomodoro", kind: Pomodoro, title: "Fokus selesai" | "Istirahat selesai", body: "Istirahat {n} menit · klik untuk mulai" | "Fokus sesi {k} dari {N} · klik untuk mulai", key: "pomodoro" }` and emits `Effect::PlaySound`. `Click(Body)` on a Pomodoro notice → dismiss + `start_pause`. `Click(Body)` while view is `PomodoroCompact` and status is `Ready` → `start_pause`. `PomoStartPause/Reset/Skip` → the pomodoro method (Skip also queues the notice + sound). `PomoAdjust(f, d)` → if changed, `Effect::SavePomodoro(settings)`. Calendar: `CalViewSel(v)` sets view; `CalPrev/CalNext` move the anchor by one day / week / month; `CalDate(d)` selects d (and anchors Day view on it); `CalAdd` → `OpenEventEditor { None, selected }`; `CalEvent(id)` → `OpenEventEditor { Some(id), selected }`; `CalConnect` → `ConnectGoogle`; `CalMore(d)` ("+N lagi" in month/week) → Day view on d; any change of view/anchor, and the panel opening on the Calendar page, → `RefreshCalendar { from, to }` for the new range. Notes: `NoteNew` → `OpenNoteEditor(None)`, `NoteOpen(id)` → `OpenNoteEditor(Some(id))`, `NotesMore` → `notes_page += 1` (exposed as `pub fn notes_page(&self) -> usize`; the layout wraps it), reset to 0 when the panel closes. `Today(d)`: if anchor/selected were "today", move them to d.

- [ ] **Step 1: Failing tests** (keep all v1 reducer tests passing, adapting `MediaExpanded` → `Panel(Page::Media)` where the old test meant "hover with media"; page defaults to `Calendar`, so those tests first set the page or assert `Panel(Calendar)` as appropriate — keep their intent):

```rust
#[test] fn idle_shows_empty_shape() { assert_eq!(st().view(), View::Idle); }
#[test] fn fullscreen_hides_idle() { /* Fullscreen(true) → Hidden */ }
#[test] fn hover_opens_panel_on_last_page() { /* Hover(true) → Panel(Calendar); Wheel(-120) → Panel(Pomodoro); Hover(false) → Idle; Hover(true) → Panel(Pomodoro) */ }
#[test] fn small_wheel_deltas_accumulate_to_one_page() { /* 4× Wheel(-30) → Pomodoro after the 4th only; then Wheel(-119) stays */ }
#[test] fn wheel_clamps_at_ends() { /* from Calendar, Wheel(+120) stays Calendar; from Media, Wheel(-120) stays Media */ }
#[test] fn wheel_ignored_when_panel_closed() { }
#[test] fn temporary_notice_blocks_panel_then_opens_if_still_hovered() { /* notice at t0, Hover(true) → Notice; Tick t0+5s → Panel(Calendar) */ }
#[test] fn waiting_does_not_block_panel_and_shows_strip() { /* waiting(s1), Hover(true) → Panel(Calendar), waiting_strip() == Some(waiting s1); Hover(false) → Notice(waiting) */ }
#[test] fn editor_keeps_panel_open() { /* Hover(true), EditorOpen(true), Hover(false) → still Panel; EditorOpen(false) → Idle */ }
#[test] fn priority_pomodoro_over_media() { /* media + pomodoro running → PomodoroCompact; reset → MediaCompact */ }
#[test] fn phase_done_queues_notice_and_sound() { /* start, Tick at +25m → effects contains PlaySound; view Notice(kind Pomodoro, title "Fokus selesai"); +5s → PomodoroCompact (Ready) */ }
#[test] fn click_ready_compact_starts_next_phase() { }
#[test] fn click_pomodoro_notice_starts_next_phase() { }
#[test] fn adjust_emits_save() { /* PomoAdjust(Focus, 1) when idle → [SavePomodoro{focus 30..}] */ }
#[test] fn calendar_nav_moves_anchor_and_requests_range() { /* Month view anchor 2026-10-25: CalNext → anchor in Nov 2026, effect RefreshCalendar{from: 2026-10-26 (Mon on/before Nov 1), to: from+42d} */ }
#[test] fn cal_add_opens_editor_on_selected_date() { }
#[test] fn opening_panel_on_calendar_requests_refresh() { }
#[test] fn cal_more_switches_to_day_view() { }
#[test] fn notes_more_advances_and_resets_on_close() { }
```

- [ ] **Step 2:** `cargo test state::` → FAIL. **Step 3:** implement. **Step 4:** full `cargo test` → PASS; clippy/fmt/build clean.
- [ ] **Step 5: Commit** `feat: island state v2 with panel pages and pomodoro`.

---

### Task 4: Config v2 and notes store

**Files:** Modify `src/config.rs`; Create `src/notes.rs`

**Interfaces:**
- Produces: `Config` gains `pub pomodoro: PomoSettings`, `pub google_client_id: String`, `pub google_client_secret: String` (defaults: `PomoSettings::default()`, `""`, `""`), all under the existing `#[serde(default)]` — old config files keep loading.
- `pub struct Note { pub id: String, pub text: String, pub updated: i64 /*unix secs*/ }` (Serialize/Deserialize, Clone, Debug, PartialEq).
- `pub struct NotesStore { .. }` with `pub fn open(path: PathBuf) -> Self` (missing/invalid file → empty, never panics), `pub fn default_path() -> PathBuf` (`%APPDATA%\DynamicIsland\notes.json`), `pub fn list(&self) -> &[Note]` (newest `updated` first), `pub fn get(&self, id: &str) -> Option<&Note>`, `pub fn upsert(&mut self, id: Option<&str>, text: &str, now: i64) -> Option<String>` (trimmed-empty text: deletes an existing note / creates nothing, returns None; otherwise returns the id; new ids `n{now}-{counter}`), `pub fn delete(&mut self, id: &str) -> bool`, `pub fn save(&self) -> io::Result<()>` (pretty JSON, write `.tmp` then rename).

- [ ] **Step 1: Failing tests:** config `old_file_without_new_fields_loads_defaults`, `pomodoro_round_trip`; notes (temp dir per test): `missing_file_is_empty`, `upsert_new_then_edit_keeps_id`, `list_is_newest_first`, `empty_text_deletes_existing`, `empty_text_creates_nothing`, `save_and_reopen_round_trip`, `corrupt_file_is_empty_not_panic`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS + clippy/fmt.
- [ ] **Step 5: Commit** `feat: config v2 and local notes store`.

---

### Task 5: Google OAuth (PKCE, loopback, tokens, DPAPI) and HTTP client

**Files:** Create `src/google/mod.rs`, `src/google/http.rs`, `src/google/oauth.rs`; Modify `Cargo.toml`

**Interfaces:**
- `http.rs`: `pub struct Http` with `pub fn new() -> Result<Self>`; `pub fn get_json(&self, url: &str, bearer: Option<&str>) -> Result<(u16, String)>`, `pub fn send_json(&self, method: &str /*POST|PATCH*/, url: &str, bearer: Option<&str>, body: &str) -> Result<(u16, String)>`, `pub fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)>`. Built on WinRT `Windows.Web.Http.HttpClient` (blocking `.join()`; call only from worker threads). Status + body returned; non-2xx is not an `Err`.
- `oauth.rs`:
  - `pub const SCOPES: &str` — the minimum scopes; verify the exact scope strings against Google docs via `ctx7` during this task (candidates: `https://www.googleapis.com/auth/calendar.events` + `https://www.googleapis.com/auth/calendar.calendarlist.readonly`; fall back to `calendar.readonly` for the list if the granular scope is not documented). Record the source in the report.
  - pure: `pub fn base64url(bytes: &[u8]) -> String` (no padding); `pub fn pkce_challenge(verifier: &str) -> String` (SHA-256 via BCrypt, then base64url); `pub fn auth_url(client_id: &str, redirect: &str, challenge: &str, state: &str) -> String`; `pub fn parse_redirect(request_line: &str) -> Result<(String /*code*/, String /*state*/), String /*error*/>`; `#[derive(Deserialize)] pub struct TokenResponse { access_token, expires_in: u64, refresh_token: Option<String>, .. }`; `pub struct Tokens { pub access: String, pub refresh: String, pub expires_at_unix: i64 }`; `pub fn merge_tokens(old: Option<&Tokens>, resp: TokenResponse, now: i64) -> Option<Tokens>` (keeps old refresh when the response has none; None if no refresh at all).
  - `pub enum AuthError { NotConfigured, NeedsLogin, Reconnect /*invalid_grant*/, Network(String) }`; `pub fn classify_token_error(status: u16, body: &str) -> AuthError`.
  - OS: `pub fn random_urlsafe(n_bytes: usize) -> String` (BCryptGenRandom → base64url); `pub fn login(http: &Http, client_id: &str, client_secret: &str) -> Result<Tokens, AuthError>` (bind `127.0.0.1:0`, open browser with `ShellExecuteW`, accept one connection with a 5-minute timeout, answer with a short Indonesian HTML page "Berhasil terhubung. Kembali ke island ya.", verify `state`, exchange code with `code_verifier`); `pub fn refresh(http, client_id, client_secret, tokens: &Tokens) -> Result<Tokens, AuthError>`; `pub fn load_tokens() -> Option<Tokens>` / `pub fn save_tokens(&Tokens) -> io::Result<()>` / `pub fn clear_tokens()` (DPAPI `CryptProtectData` / `CryptUnprotectData`, current-user scope, file per Values table); `pub fn access_token(http, cfg) -> Result<String, AuthError>` (refresh when within 60 s of expiry, save new tokens; `Reconnect` clears tokens).

- [ ] **Step 1: Failing tests (pure only):**

```rust
#[test] fn pkce_rfc7636_vector() { assert_eq!(pkce_challenge("dBjftJeZ4CVP-mJ92K1yOdVpcbHw6m6l6YRHnPj2Pl8"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"); }
#[test] fn base64url_has_no_padding() { assert_eq!(base64url(b"ab"), "YWI"); assert_eq!(base64url(&[0xfb, 0xff]), "-_8"); }
#[test] fn auth_url_has_required_params() { /* contains response_type=code, code_challenge_method=S256, access_type=offline, prompt=consent, redirect_uri=http%3A%2F%2F127.0.0.1%3A5555, state=xyz, url-encoded scope */ }
#[test] fn parse_redirect_ok_and_error() { /* "GET /?state=s1&code=4%2F0Ab HTTP/1.1" → ("4/0Ab","s1"); "GET /?error=access_denied&state=s1 HTTP/1.1" → Err("access_denied") */ }
#[test] fn token_response_without_refresh_keeps_old_refresh() { }
#[test] fn invalid_grant_maps_to_reconnect() { assert!(matches!(classify_token_error(400, r#"{"error":"invalid_grant"}"#), AuthError::Reconnect)); }
#[test] fn dpapi_round_trip() { /* save to temp path via an internal fn taking the path, load back equal; file bytes do not contain the access token text */ }
```

- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS. Do NOT perform a real Google login (no credentials exist yet); `login` is exercised by the user in Task 11.
- [ ] **Step 5: Commit** `feat: google oauth with pkce, loopback and dpapi token store`.

---

### Task 6: Calendar model, Google Calendar API and sync thread

**Files:** Create `src/calendar/mod.rs`, `src/calendar/api.rs`, `src/calendar/sync.rs`; Modify `src/state.rs` (use `calendar::range_for`)

**Interfaces:**
- `mod.rs`:
  - `pub enum EventTime { AllDay(NaiveDate), At(DateTime<Local>) }`; `pub struct CalEvent { pub id: String, pub calendar_id: String, pub title: String, pub start: EventTime, pub end: EventTime, pub description: String, pub color: Option<u32>, pub editable: bool }` (Serialize/Deserialize for the cache).
  - `pub struct EventDraft { pub title: String, pub date: NaiveDate, pub all_day: bool, pub start: NaiveTime, pub end: NaiveTime, pub description: String }`; `pub fn draft_from(ev: &CalEvent) -> EventDraft`.
  - `pub enum CalStatus { NotConfigured, NeedsLogin, Reconnect, Ok, Offline }`; `pub struct CalendarSnapshot { pub status: CalStatus, pub events: Vec<CalEvent>, pub from: NaiveDate, pub to: NaiveDate, pub fetched_at: Option<DateTime<Local>> }`.
  - `pub fn range_for(view: CalView, anchor: NaiveDate) -> (NaiveDate, NaiveDate)` per the Values table (Day: `[d, d+1)`).
  - `pub fn events_on(events: &[CalEvent], date: NaiveDate) -> Vec<&CalEvent>` (all-day first, then by start; all-day end exclusive; timed events covering any part of the local date).
- `api.rs`: `pub fn parse_calendar_list(json: &str) -> Vec<(String /*id*/, Option<u32> /*color*/, bool /*writable*/)>` (only `selected == true`; writable = accessRole owner|writer); `pub fn parse_events(json: &str, calendar_id: &str, color: Option<u32>, writable: bool) -> Vec<CalEvent>` (skips cancelled); `pub fn draft_body(d: &EventDraft) -> serde_json::Value` (all-day → `{date}` start/end with end = date+1; timed → `{dateTime}` RFC 3339 with local offset); network fns: `list_calendars(http, token) -> Result<..>`, `list_events(http, token, cal_id, from, to) -> Result<Vec<CalEvent>>` (`singleEvents=true`, `orderBy=startTime`, `timeMin`/`timeMax` RFC 3339, follows `nextPageToken`), `insert_event(http, token, draft) -> Result<()>` (calendar `primary`), `patch_event(http, token, cal_id, event_id, draft) -> Result<()>`.
- `sync.rs`: `pub enum SyncRequest { Range { from, to }, Save { event_id: Option<String>, calendar_id: Option<String>, draft: EventDraft, reply: HWND as isize }, Connect, Shutdown }`; `pub fn spawn_sync(tx: Sender<Event>, wake: impl Fn() + Send + 'static, cfg: Config) -> Sender<SyncRequest>`; on any result it sends `Event::Calendar(CalendarSnapshot)` (add this variant to `Event`; the reducer ignores it — `App` keeps the snapshot) and, for saves, posts `WM_APP_EDITOR_STATUS` to `reply` with `Ok`/`Err(message)` (Indonesian, e.g. "Gagal simpan: tidak ada koneksi"). Cache: load at start (status `Offline` until the first fetch succeeds), save after each successful fetch; a fetch error keeps cached events with status `Offline` (or `Reconnect`/`NeedsLogin` per `AuthError`). Timer: every 10 min re-fetch the last requested range.

- [ ] **Step 1: Failing tests (pure, with JSON fixtures written inline in the test module):** `range_month_starts_monday_before_first` (Oct 2026 → 2026-09-28 .. 2026-11-09), `range_week_monday_to_monday`, `range_day`, `multi_day_all_day_covers_each_date` (`date` 2026-10-26 → `date` 2026-10-29 on 26, 27, 28 only), `timed_event_crossing_midnight_on_both_days`, `offset_times_convert_to_local` (input `2026-10-25T10:00:00-07:00` lands on the local date/time computed via `chrono::Local`), `events_on_orders_all_day_first`, `cancelled_events_skipped`, `calendar_list_keeps_selected_only`, `draft_body_all_day_end_exclusive`, `draft_body_timed_has_offset`, `cache_round_trip` (temp dir), `fetch_error_keeps_cached_events_offline` (pure helper `merge_fetch(prev: &CalendarSnapshot, result: Result<Vec<CalEvent>, AuthError>, ..) -> CalendarSnapshot`).
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS + clippy/fmt. No real network calls in tests.
- [ ] **Step 5: Commit** `feat: google calendar model, api and sync`.

---

### Task 7: Panel geometry, layouts and hit regions

**Files:** Modify `src/ui/geometry.rs`; Create `src/ui/panel_layout.rs`

**Interfaces:**
- `ShapeKind` becomes `{ Idle, MediaCompact, PomodoroCompact, Notice, Panel }` (sizes per Values table; idle and media compact share 190 × 34). `shape_for(style, kind)` keeps the v1 style rules (Pill y = 8, radius h/2 for 34-high shapes, 26 otherwise; Notch y = 0, radius 16 for 34-high shapes, 26 for the panel, 22 for notice).
- `panel_layout.rs` (all rects relative to the window, logical px):
  - `pub struct Region { pub rect: Rect, pub hit: Hit }`.
  - `pub fn page_dots(s: ShapeRect, page: Page) -> [Rect; 4]` (centered row, 6 px dots, 6 px gap, 12 px above the bottom edge).
  - `pub fn waiting_strip(s: ShapeRect) -> Rect` (full-width 24 px strip under the top edge inside the panel; content below shifts down by 24 when present).
  - `pub struct CalendarLayout { header: Rect, view_buttons: [(CalView, Rect); 3], prev: Rect, next: Rect, month_cells: Vec<(NaiveDate, Rect)>, week_columns: Vec<(NaiveDate, Rect)>, list: Rect, rows: Vec<(String /*event id*/, Rect)>, more: Option<(usize, Rect)>, add: Rect, connect: Option<Rect> }`; `pub fn calendar_layout(s, view, anchor, selected, day_events: &[&CalEvent], status: &CalStatus, strip: bool) -> CalendarLayout`. Month: 7 × 6 cells 28 × 24 with 4 px column / 2 px row gaps at (s.x+20, header bottom+6); list to the right (x from cell grid right + 16 to s.x+s.w-20). Week: 7 columns across the content width, each listing up to 3 event rows then "+N". Day: rows of 34 px. Row cap: as many as fit above the page dots; overflow → `more = (hidden count, rect)` drawn as "+N lagi".
  - `pub struct PomodoroLayout { ring_center: (f32, f32), ring_radius: f32, steppers: [(PomoField, Rect /*minus*/, Rect /*plus*/, Rect /*label/value*/); 4], start_pause: Rect, reset: Rect, skip: Rect }`; `pub fn pomodoro_layout(s, strip) -> PomodoroLayout` (ring radius 70 at left third, controls right, per mockup).
  - `pub struct NotesLayout { new_button: Rect, rows: Vec<(String, Rect)>, more: Option<(usize, Rect)> }`; `pub fn notes_layout(s, notes: &[Note], notes_page: usize, strip) -> NotesLayout` (rows 40 px, cap `k` as fits; shows notes `[(notes_page*k) % len ..]` up to k; `more` is clickable → `Hit::NotesMore`, label "+N lagi" / "Kembali ke atas" on the last chunk). Month/week "+N" regions → `Hit::CalMore(date)`.
  - `pub fn media_panel_layout(s, strip) -> ExpandedLayout` (reuse the v1 expanded layout, scaled into the panel content area).
  - `pub fn pomodoro_compact_layout(s: ShapeRect) -> (ring_center, ring_radius /*11*/, text: Rect)`.
  - `pub fn regions(...)` per page returning `Vec<Region>` built from the layouts above (the renderer stores them for `hit_test`).
- [ ] **Step 1: Failing tests:** `panel_centered_both_styles` (x = (620-560)/2 = 30; Pill y 8, Notch y 0), `idle_matches_v1_compact_size`, `page_dots_centered_and_highlight_index`, `month_cells_42_starting_monday` (Oct 2026 first cell date 2026-09-28, last 2026-11-08), `day_list_overflow_shows_more_count` (20 events → `more` = (20 - rows.len(), _)), `notes_list_caps_rows` (30 notes), `strip_shifts_content_down_24`, `regions_have_no_overlaps_within_page` (for each page, region rects are pairwise disjoint), `all_regions_inside_panel`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS + clippy/fmt.
- [ ] **Step 5: Commit** `feat: panel layouts and hit regions`.

---

### Task 8: Rendering the new views

**Files:** Modify `src/ui/render.rs`; Create `src/ui/render_panel.rs` (page drawing; keeps render.rs from growing further)

**Interfaces:**
- Consumes: Tasks 3, 6, 7 types. `Renderer::draw` signature becomes `draw(&mut self, win: &IslandWindow, style: Style, shape: ShapeRect, frame: &Frame, opacity: f32)` where `pub struct Frame<'a> { pub view: &'a View, pub media: Option<&'a MediaInfo>, pub pomodoro: &'a Pomodoro, pub now: Instant, pub page: Page, pub strip: Option<&'a Notice>, pub cal_view: CalView, pub cal_anchor: NaiveDate, pub cal_selected: NaiveDate, pub calendar: &'a CalendarSnapshot, pub notes: &'a [Note], pub notes_page: usize, pub page_slide: f32 /*-1..1 during page transitions, 0 at rest*/ }`.
- Visuals (per mockups `panel-pages.html`, `pill-vs-notch.html`): idle = empty black shape; media compact = v1 layout but equalizer bar heights from the Values-table formula using `frame.now` (static mid heights when paused); pomodoro compact = small ring (radius 11, 3 px stroke; red `#EF5350` focus, green `#2ECC71` breaks; grey track `#2A2A2C`) + `mm:ss` + short label ("Fokus 2/4", "Istirahat", or "Siap · klik"); panel pages as the mockup (calendar month grid with today filled white, selected outlined, event dots `#4F8CFF` or the calendar color; week and day views; "+ Tambah acara"; "Hubungkan Google" / "Hubungkan ulang Google" button and an "offline" tag for those statuses; pomodoro big ring radius 70, 8 px stroke, steppers, buttons — "Mulai" / "Jeda" / "Lanjut" by status; notes rows with first line + relative time "hari ini 09.12" / "kemarin" / "23 Okt"; media page = v1 expanded media content), page dots, waiting strip (orange `#D97757` dot + "Claude butuh respon · {project}"). Text trimming with ellipsis everywhere (v1 rules). Page transition: content slides horizontally by `page_slide × panel width` with clipping to the panel.
- [ ] **Step 1:** Implement; extend `--demo` (debug only) to cycle idle → pomodoro compact → each panel page (both styles, with a fake snapshot of 6 events incl. all-day and multi-day, and 5 notes).
- [ ] **Step 2: Verify visually:** screenshots of every page in both styles under `.superpowers/sdd/<plan>/task-8-shots/`, Read them, compare with the mockups; `WindowFromPoint` click-through check outside the panel. `cargo test`, clippy, fmt, release build clean.
- [ ] **Step 3: Commit** `feat: draw idle notch, pomodoro ring and panel pages`.

---

### Task 9: Editor windows (notes and events)

**Files:** Create `src/ui/editor.rs`; Modify `app.manifest` (add the Common-Controls v6 `<dependency>`), `Cargo.toml` (features as needed)

**Interfaces:**
- `pub enum EditorKind { Note { id: Option<String>, text: String }, Event { event_id: Option<String>, calendar_id: Option<String>, draft: EventDraft } }`.
- `pub enum EditorResult { NoteSaved { id: Option<String>, text: String }, NoteDeleted { id: String }, EventSave { event_id: Option<String>, calendar_id: Option<String>, draft: EventDraft }, Closed }`.
- `pub const WM_APP_EDITOR_RESULT: u32` (lparam = `Box<EditorResult>` raw pointer, owner frees) and `pub const WM_APP_EDITOR_STATUS: u32` (sent by the app to the editor: wparam 1 = saved → editor closes; 0 = error, lparam = `Box<String>` message shown in a red label, Simpan re-enabled).
- `pub fn open_editor(owner: HWND, anchor_bottom_center: POINT /*screen px*/, kind: EditorKind) -> Result<HWND>` — at most one editor at a time (opening a second focuses the first).
- Note editor: 400 × 220 logical, title "Note", multi-line EDIT (`ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN`, word wrap), "Hapus" button for existing notes; closing (X / Esc / click outside → `WM_ACTIVATE` inactive) posts `NoteSaved` with the current text (the store treats empty as delete). Ctrl+Enter also saves & closes.
- Event editor: 420 × 260 logical: "Judul" EDIT, "Tanggal" date picker, "Seharian" checkbox, "Mulai"/"Selesai" time pickers (disabled when all-day), "Catatan" multi-line EDIT, "Simpan" / "Batal", status label. Simpan with empty title → inline error "Judul wajib diisi". Simpan → posts `EventSave`, shows "Menyimpan…", disables Simpan until `WM_APP_EDITOR_STATUS`. Batal / Esc → `Closed` without saving. Read-only events (calendar not writable) open with fields disabled and only "Tutup".
- Dark theme: `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`, `WM_CTLCOLOREDIT/STATIC/BTN` dark brushes (`#202022` bg, `#EEEEEE` text), `SetWindowTheme(.., "DarkMode_Explorer", None)` on buttons/pickers. `WS_EX_TOOLWINDOW`, topmost, owned by the island window; placed centered under `anchor_bottom_center` + 8 px, clamped to the monitor work area. On close, focus returns to the window that was foreground before opening (store it on open).
- Pure, tested: `pub fn editor_rect(anchor: POINT, size: SIZE, work: RECT) -> RECT` (`centered_below_anchor`, `clamped_to_work_area`), `pub fn validate_event(d: &EventDraft) -> Result<(), &'static str>` (`empty_title_rejected`, `end_before_start_rejected` → "Jam selesai harus setelah jam mulai", `all_day_ignores_times`).
- [ ] **Step 1:** failing pure tests → **Step 2:** FAIL → **Step 3:** implement → **Step 4:** PASS.
- [ ] **Step 5: Verify:** debug-only `--demo-editor note|event` opens each editor standalone; screenshot both; type text with `SendKeys`/`SendInput` from PowerShell is optional — at least confirm the windows render dark, positioned, and close cleanly (`WM_CLOSE`). clippy/fmt/build clean.
- [ ] **Step 6: Commit** `feat: dark note and event editor windows`.

---

### Task 10: App wiring v2

**Files:** Modify `src/app.rs`, `src/main.rs`; small touches in `src/ui/tray.rs` if needed

**Interfaces:** consumes everything above.

- [ ] **Step 1: Startup:** `IslandState::new(Local::now().date_naive(), cfg.pomodoro)`; `NotesStore::open(default_path())`; `spawn_sync(...)` (holds the request sender); initial `RefreshCalendar` for the default range; a `TIMER_DAY` checks every 60 s for date rollover → `Event::Today`.
- [ ] **Step 2: Input:** `WM_MOUSEWHEEL` → `Event::Wheel(GET_WHEEL_DELTA_WPARAM)` (works on the non-activated island; verify). Clicks map through the renderer's regions to the new `Hit`s.
- [ ] **Step 3: Effects:** `PlaySound` → Values-table call; `SavePomodoro` → `cfg.pomodoro = s; cfg.save()`; `ConnectGoogle` → `SyncRequest::Connect` (sync thread runs `oauth::login` off the UI thread); if `google_client_id` is empty, show a notice "Isi Google client ID dulu — lihat README" instead; `RefreshCalendar` → `SyncRequest::Range` (skip if the same range was fetched < 60 s ago, except on explicit navigation); `OpenNoteEditor` / `OpenEventEditor` → `open_editor` anchored at the panel's bottom center in screen px, then `Event::EditorOpen(true)`; `WM_APP_EDITOR_RESULT` → notes upsert/delete + `save()`, or `SyncRequest::Save`; editor destroyed → `Event::EditorOpen(false)`. `Event::Calendar(snapshot)` from the sync thread → stored in `App`, redraw.
- [ ] **Step 3b: Timers:** equalizer → `TIMER_EQ` (~33 ms) redraws only while the visible view is `MediaCompact` and media is playing; killed otherwise (measure CPU with music playing vs paused and report both). Pomodoro running → `TIMER_POMO` every 1000 ms redraws (and `Tick` at `next_deadline`); stopped when not running. Page switch → animate `page_slide` from ±1 to 0 over 220 ms with ease-out via the `FrameClock`.
- [ ] **Step 4: Verify (automatable):** release build; island shows idle notch on start (screenshot); `send-test-toast.ps1` still works and blocks the panel 5 s; `--hook` Stop still shows; scripted hover is not possible — instead use debug-only `--demo` paths from Task 8/9 for visuals. Idle CPU: same poll-only cost as v1 (≈0.45–0.8 s per 30 s), RAM tens of MB (report numbers). Restore `%APPDATA%\DynamicIsland` and the Run value to their pre-test state (the owner's real config and notes must be backed up before and restored after). Never run `--install-hooks`.
- [ ] **Step 5: Commit** `feat: wire panel, calendar sync, pomodoro and editors`.

---

### Task 11: README and end-to-end verification

**Files:** Modify `README.md`

- [ ] **Step 1: README (Indonesian):** what's new; how to use the panel (hover, wheel, pages); Pomodoro; Notes (where stored); **Google Calendar setup** step by step: Google Cloud Console → new project → enable "Google Calendar API" → OAuth consent screen (External, add yourself as test user) → Credentials → OAuth client ID → type **Desktop app** → copy client ID + secret into `%APPDATA%\DynamicIsland\config.json` (`google_client_id`, `google_client_secret`) → restart island → Kalender page → "Hubungkan Google". State plainly the 7-day re-login while the project is "Testing" and the option to publish to "In production" (unverified-app warning) for personal use. Commands use `powershell -ExecutionPolicy Bypass -File ...` for scripts.
- [ ] **Step 2:** `cargo test`, clippy, fmt, release build clean; resource numbers (idle, panel open, pomodoro running) recorded in the commit body.
- [ ] **Step 3:** Human checklist in the report (owner does these): hover feel + smoothness, wheel paging, Google login, add + edit an event and see it in Google Calendar web, offline behaviour (disable network), pomodoro full cycle with sound, notes create/edit/delete, fullscreen game/video hides everything, both styles.
- [ ] **Step 4: Commit** `docs: v2 readme and verification notes`.
