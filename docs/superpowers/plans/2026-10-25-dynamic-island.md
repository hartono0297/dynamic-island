# Dynamic Island for Windows — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A lightweight native Rust app that draws a dynamic island at the top-center of the primary monitor, showing media controls, every Windows notification, Claude Code events and low battery.

**Architecture:** One process. The UI thread owns a per-pixel-alpha layered window (Direct2D draws into a DIB, pushed with `UpdateLayeredWindow`; alpha-0 pixels are click-through for free) and a pure state reducer (`IslandState`). Background threads (notification poller, media watcher, named-pipe server) send `Event`s over an `mpsc` channel and wake the UI thread with `PostMessageW(WM_APP_EVENT)`. Animation timers run only while something animates, so idle CPU is ~0. The same exe doubles as the Claude Code hook client (`--hook`).

**Tech Stack:** Rust stable (MSVC target), `windows` 0.62 (Win32 + WinRT), `serde`/`serde_json` 1, `embed-resource` 3 (build-dep). No other runtime deps.

**Spec:** `docs/superpowers/specs/2026-10-25-dynamic-island-design.md` (Indonesian). Read it before starting any task.

## Global Constraints

- Windows 11 x64, primary monitor only. Target `x86_64-pc-windows-msvc`.
- Light: idle RAM in the tens of MB or less, idle CPU ~0%. No browser engine, no .NET, no Electron.
- Notifications only through the official `UserNotificationListener`. Never read `wpndatabase.db`, never use registry/WNF tricks.
- The app never changes Windows system settings (no toggling Do Not Disturb, no banner registry edits).
- Existing hooks in `~/.claude/settings.json` (Orca's) must survive untouched. That file holds secrets: never print its contents.
- Two styles: `Pill` (black, fully rounded, 8 px below top edge) and `Notch` (black, flush with top edge, rounded bottom corners, 12 px inward-curved shoulders). Reference mockup: `.superpowers/brainstorm/*/content/island-style.html`.
- Fullscreen app in front: island hidden; a new notification still appears briefly, then hides again.
- Click a notification → open its source app (`explorer.exe shell:AppsFolder\<AUMID>`), dismiss it from the island. No inline reply.
- Notifications stay in Windows Notification Center (never removed by the island).
- Claude notices come only from the hook; Windows toasts whose app name contains `Claude` or `Orca` are muted (configurable).
- "Claude waiting" notice is sticky until the next hook event of the same `session_id` or a click.
- User-facing strings in Indonesian.

### Values pinned by this plan (builder's discretion in the spec, decided here)

| Name | Value |
|---|---|
| Notice display time | 5 s |
| Shape animation | 380 ms, `ease_out_back` (overshoot constant 1.2) |
| Notification poll interval | 1000 ms |
| Fullscreen poll interval | 1000 ms |
| Battery poll interval | 30 s; default threshold 20 % |
| Pipe name | `\\.\pipe\dynamic-island` |
| Config file | `%APPDATA%\DynamicIsland\config.json` |
| Window size (logical px @96 DPI) | 440 × 150 |
| Shapes (w × h, logical px) | compact 190 × 34; media expanded 380 × 110; notice 360 × 76 |
| Idle (no media, no notice) | hidden (fully transparent) |
| Package identity name / publisher | `DynamicIsland` / `CN=DynamicIslandDev` |

## Review Focus

1. **Notification Center already full at startup** → none of the old ones flood the island; only new arrivals show. (Task 5 test `first_update_primes_without_showing`.)
2. **Island not running when a Claude hook fires** → hook exits 0 within 200 ms, Claude is not slowed or errored. (Task 8 test `send_to_missing_pipe_is_silent_and_fast`.)
3. **Very long title/body, emoji, or 30 notices in a burst** → text trimmed with ellipsis inside the shape; notices shown one by one, none lost. (Task 3 test `burst_of_notices_shows_each_in_order`; Task 9 manual check with `scripts/send-test-toast.ps1 -Long`.)
4. **Media app closed mid-song / no media session at all** → island hides, no stale track. (Task 3 test `media_gone_hides_island`.)
5. **Display scaling or monitor change (125 %/150 %, external monitor, resolution change)** → island re-centers on the primary monitor at the right physical size. (Task 9 handles `WM_DPICHANGED` and `WM_DISPLAYCHANGE`; Task 12 manual check.)

---

## File Structure

```
Cargo.toml
build.rs                       embeds app.manifest (identity + DPI)
app.manifest
app.rc                         1 24 "app.manifest"
packaging/AppxManifest.xml     sparse package manifest
scripts/package.ps1            make logo, cert (first run), MakeAppx, SignTool, Add-AppxPackage
scripts/send-test-toast.ps1    fires a test Windows toast
src/main.rs                    arg dispatch: (none) run | --hook | --probe-notifications | --install-hooks
src/model.rs                   Notice, MediaInfo, Event, Effect, View, Hit, MediaCommand
src/state.rs                   IslandState reducer (pure)
src/config.rs                  Config load/save (pure + fs)
src/sources/mod.rs
src/sources/notifications.rs   NotificationTracker (pure) + poller thread + open_app
src/sources/media.rs           GSMTC watcher thread + send_command
src/sources/battery.rs         BatteryMonitor (pure) + read_battery
src/sources/fullscreen.rs      is_fullscreen()
src/claude/mod.rs
src/claude/hook.rs             parse_hook_input (pure), send_to_pipe
src/claude/pipe_server.rs      pipe server thread
src/claude/install.rs          add_island_hooks (pure) + install_hooks
src/ui/mod.rs
src/ui/geometry.rs             ShapeRect, shape_for, lerp, ease_out_back (pure)
src/ui/window.rs               layered window, placement, DPI
src/ui/render.rs               Direct2D/DirectWrite/WIC drawing
src/ui/tray.rs                 tray icon + menu
src/app.rs                     UI-thread loop: events → reducer → effects → render
```

Pure modules (`state`, `config` parse, `NotificationTracker`, `BatteryMonitor`, `hook` parse, `install` merge, `geometry`) carry unit tests run with `cargo test`. OS-bound code is verified with the manual commands given in each task.

---

### Task 1: Toolchain and project skeleton

**Files:** Create `Cargo.toml`, `build.rs`, `app.manifest`, `app.rc`, `src/main.rs`, `.gitignore`

**Interfaces:**
- Produces: a binary `dynamic-island.exe` at `target\release\` with the embedded manifest below; `main()` dispatching on `std::env::args().nth(1)`: `None` → `app::run()`, `"--hook"` → `claude::hook::run_hook()`, `"--probe-notifications"` → `sources::notifications::probe()`, `"--install-hooks"` → `claude::install::install_hooks()`. Unwritten targets are `todo!()` stubs until their task.

- [ ] **Step 1: Init git and ignore list.** `git init` in the project root. `.gitignore`: `/target`, `/.superpowers/`, `/packaging/dev-cert/`, `/packaging/staging/`.
- [ ] **Step 2: Install toolchain (user approves UAC prompts).**
  - `winget install --id Rustlang.Rustup -e`
  - `winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` (recommended set brings the Windows SDK, which holds `MakeAppx.exe` and `SignTool.exe`).
  - Open a new shell. Verify: `rustc --version` prints a version; `Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe"` finds a file.
- [ ] **Step 3: `cargo init --name dynamic-island`.** `Cargo.toml`: `windows = { version = "0.62", features = [...] }` starting with `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Shell`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_Graphics_Gdi`, `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`, `Win32_Graphics_DirectWrite`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Imaging`, `Win32_System_Com`, `Win32_System_Power`, `Win32_System_Pipes`, `Win32_Storage_FileSystem`, `Win32_System_Threading`, `Win32_System_Registry`, `Win32_System_LibraryLoader`, `Win32_System_WinRT`, `Foundation`, `Foundation_Collections`, `Media_Control`, `Storage_Streams`, `UI_Notifications`, `UI_Notifications_Management`, `ApplicationModel`; add features as the compiler asks. `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`; build-dep `embed-resource = "3"`. `[profile.release] opt-level = "s"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. In `main.rs`: `#![windows_subsystem = "windows"]`, but `--hook` and `--probe-notifications` must still write to stdout/stderr — call `AttachConsole(ATTACH_PARENT_PROCESS)` at the start of those two branches.
- [ ] **Step 4: Embedded manifest.** `app.manifest` (exact):

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly manifestVersion="1.0" xmlns="urn:schemas-microsoft-com:asm.v1">
  <assemblyIdentity version="0.1.0.0" name="DynamicIsland" type="win32"/>
  <msix xmlns="urn:schemas-microsoft-com:msix.v1"
        publisher="CN=DynamicIslandDev" packageName="DynamicIsland" applicationId="DynamicIsland"/>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>
```

  `app.rc`: `1 24 "app.manifest"`. `build.rs`: `embed_resource::compile("app.rc", embed_resource::NONE).manifest_required().unwrap();`
- [ ] **Step 5: Verify.** `cargo build --release` succeeds; `cargo test` runs (0 tests).
- [ ] **Step 6: Commit** `chore: rust skeleton with identity manifest` (include `docs/superpowers/` — the spec and this plan are not committed yet).

---

### Task 2: Package identity and the Do Not Disturb gate

**Files:** Create `packaging/AppxManifest.xml`, `scripts/package.ps1`, `scripts/send-test-toast.ps1`; Modify `src/sources/notifications.rs` (add `probe`)

**Interfaces:**
- Produces: `pub fn probe()` — requests listener access, then every 1 s for 60 s prints each new notification as `"<app name> | <title> | <body>"`.

- [ ] **Step 1: Sparse manifest.** `packaging/AppxManifest.xml` (exact):

```xml
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
  xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap3 uap10 rescap">
  <Identity Name="DynamicIsland" Publisher="CN=DynamicIslandDev" Version="0.1.0.0" ProcessorArchitecture="x64"/>
  <Properties>
    <DisplayName>Dynamic Island</DisplayName>
    <PublisherDisplayName>Dynamic Island Dev</PublisherDisplayName>
    <Logo>logo.png</Logo>
    <uap10:AllowExternalContent>true</uap10:AllowExternalContent>
  </Properties>
  <Resources><Resource Language="en-us"/></Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0"/>
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust"/>
    <rescap:Capability Name="unvirtualizedResources"/>
    <uap3:Capability Name="userNotificationListener"/>
  </Capabilities>
  <Applications>
    <Application Id="DynamicIsland" Executable="dynamic-island.exe"
                 uap10:TrustLevel="mediumIL" uap10:RuntimeBehavior="win32App">
      <uap:VisualElements DisplayName="Dynamic Island" Description="Dynamic Island"
        BackgroundColor="transparent" Square150x150Logo="logo.png" Square44x44Logo="logo.png"
        AppListEntry="none"/>
    </Application>
  </Applications>
</Package>
```

- [ ] **Step 2: `scripts/package.ps1`.** In order, idempotent:
  1. Locate the newest `makeappx.exe`/`signtool.exe` under `C:\Program Files (x86)\Windows Kits\10\bin\*\x64\`.
  2. If `packaging/dev-cert/dev.pfx` is missing: `New-SelfSignedCertificate -Type Custom -Subject "CN=DynamicIslandDev" -KeyUsage DigitalSignature -FriendlyName "DynamicIsland Dev" -CertStoreLocation Cert:\CurrentUser\My -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3","2.5.29.19={text}")`; export `.pfx` with a random password written to `packaging/dev-cert/password.txt`, and `.cer`. Then **print** (do not run) the one elevated command the user must run once: `Import-Certificate -FilePath "<abs>\packaging\dev-cert\dev.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople`.
  3. Stage `packaging/staging/` = `AppxManifest.xml` + a 150×150 `logo.png` (black rounded pill on transparent, drawn with `System.Drawing`).
  4. `makeappx pack /d packaging\staging /p target\DynamicIsland.msix /nv /o`; `signtool sign /fd SHA256 /f dev.pfx /p <pw> target\DynamicIsland.msix`.
  5. `Get-AppxPackage DynamicIsland | Remove-AppxPackage` if present, then `Add-AppxPackage -Path target\DynamicIsland.msix -ExternalLocation "<abs>\target\release"`.
- [ ] **Step 3: `scripts/send-test-toast.ps1`.** Fires a toast through the WinRT `ToastNotificationManager` from PowerShell's own AUMID; `-Long` switch sends a 300-character body with emoji.
- [ ] **Step 4: Implement `probe()`.** `CoInitializeEx(COINIT_APARTMENTTHREADED)` on the main thread; `UserNotificationListener::Current()?.RequestAccessAsync()?.get()`; if not `Allowed`, print `Izin notifikasi belum diberikan. Buka: ms-settings:privacy-notifications` and exit 1. Otherwise loop with `NotificationTracker` (Task 5 defines it; for now inline a `HashSet<u32>`) over `GetNotificationsAsync(NotificationKinds::Toast)`. Read text from `Notification().Visual().GetBinding(KnownNotificationBindings::ToastGeneric())?.GetTextElements()`; app name from `AppInfo().DisplayInfo().DisplayName()`.
- [ ] **Step 5: Verify identity.** `cargo build --release`, `scripts\package.ps1` (user runs the printed elevated command once), then `Get-AppxPackage DynamicIsland` shows `Status: Ok`.
- [ ] **Step 6: GATE — Do Not Disturb.** User turns Do Not Disturb **on** (Settings → System → Notifications). Run `target\release\dynamic-island.exe --probe-notifications`, accept the prompt, then run `scripts\send-test-toast.ps1` in another shell. **Expected:** no banner on screen, and the probe prints the toast line. **If the probe prints nothing, STOP and report to the user** (the spec says this changes the design). Also note whether a Steam chat message shows up in the probe, and record the result in the commit message.
- [ ] **Step 7: Commit** `feat: sparse package identity and notification probe`.

---

### Task 3: Model and state reducer

**Files:** Create `src/model.rs`, `src/state.rs`

**Interfaces:**
- Produces (`model.rs`, all `#[derive(Clone, Debug, PartialEq)]`):
  - `pub struct Notice { pub key: String, pub source: String, pub title: String, pub body: String, pub kind: NoticeKind }`
  - `pub enum NoticeKind { App { app_id: Option<String> }, ClaudeDone { session_id: String }, ClaudeWaiting { session_id: String }, BatteryLow }`
  - `pub struct MediaInfo { pub title: String, pub artist: String, pub playing: bool, pub art: Option<Vec<u8>> }`
  - `pub enum Event { Media(Option<MediaInfo>), Notice(Notice), ClaudeActivity { session_id: String }, Fullscreen(bool), Hover(bool), Click(Hit), Tick }`
  - `pub enum Hit { Body, Prev, PlayPause, Next }`
  - `pub enum MediaCommand { Prev, PlayPause, Next }`
  - `pub enum Effect { OpenApp(String), Media(MediaCommand) }`
  - `pub enum View { Hidden, MediaCompact, MediaExpanded, Notice(Notice) }`
- Produces (`state.rs`): `pub const NOTICE_DURATION: Duration = Duration::from_secs(5);` and `impl IslandState { pub fn new() -> Self; pub fn apply(&mut self, event: Event, now: Instant) -> Vec<Effect>; pub fn view(&self) -> View; pub fn media(&self) -> Option<&MediaInfo>; pub fn next_deadline(&self) -> Option<Instant> }`.

Rules: temporary notices queue FIFO, each shown `NOTICE_DURATION` from when it becomes current. A sticky `ClaudeWaiting` is shown when no temporary notice is current; temporary notices preempt it and it returns after. `ClaudeActivity{s}` and `Notice(ClaudeDone{s})` remove the waiting notice of session `s`. Fullscreen hides everything except the current notice. `Click(Body)` on a notice dismisses it and, for `App{app_id: Some(id)}`, returns `Effect::OpenApp(id)`. `Click(Prev|PlayPause|Next)` returns the matching `Effect::Media`. Hover only matters when media exists and no notice is current.

- [ ] **Step 1: Write failing tests** in `state.rs` (`t0 = Instant::now()`, helper `app_notice("k1")`, `media(true)`):

```rust
#[test] fn starts_hidden() { assert_eq!(IslandState::new().view(), View::Hidden); }
#[test] fn media_shows_compact_and_hover_expands() {
    let mut s = IslandState::new();
    s.apply(Event::Media(Some(media(true))), t0);
    assert_eq!(s.view(), View::MediaCompact);
    s.apply(Event::Hover(true), t0);
    assert_eq!(s.view(), View::MediaExpanded);
    s.apply(Event::Hover(false), t0);
    assert_eq!(s.view(), View::MediaCompact);
}
#[test] fn paused_media_still_shows() { /* Media(Some(media(false))) → MediaCompact */ }
#[test] fn media_gone_hides_island() { /* Some then None → Hidden */ }
#[test] fn notice_shows_then_returns_to_media() {
    // media, then notice at t0 → View::Notice(k1); Tick at t0+4.9s → still k1; Tick at t0+5s → MediaCompact
}
#[test] fn burst_of_notices_shows_each_in_order() {
    // 30 notices k0..k29 at t0; for i in 0..30: view == Notice(k_i) at t0 + 5s*i, then Tick at t0+5s*(i+1)
    // finally Hidden
}
#[test] fn fullscreen_hides_media_but_not_new_notice() {
    // media + Fullscreen(true) → Hidden; Notice k1 → Notice(k1); Tick +5s → Hidden; Fullscreen(false) → MediaCompact
}
#[test] fn claude_waiting_is_sticky_until_same_session_activity() {
    // waiting(s1) at t0; Tick t0+60s → still waiting; ClaudeActivity{s2} → still; ClaudeActivity{s1} → Hidden
}
#[test] fn claude_done_clears_waiting_of_same_session() { /* waiting(s1) then Notice(ClaudeDone{s1}) → shows done; after 5s → Hidden, not waiting */ }
#[test] fn temporary_notice_preempts_waiting_then_waiting_returns() { /* waiting(s1), app k1 → k1; +5s → waiting(s1) */ }
#[test] fn click_app_notice_opens_app_and_dismisses() {
    // Notice App{Some("WhatsApp.Desktop!App")}; Click(Body) → vec![Effect::OpenApp("WhatsApp.Desktop!App".into())], view Hidden
}
#[test] fn click_media_buttons_emit_commands() { /* MediaExpanded, Click(Next) → vec![Effect::Media(MediaCommand::Next)] */ }
#[test] fn next_deadline_is_current_notice_expiry() { /* notice at t0 → Some(t0+5s); no notice → None */ }
```

- [ ] **Step 2:** `cargo test state::` → FAIL (types missing).
- [ ] **Step 3:** Implement `model.rs` and `IslandState` (fields: `media: Option<MediaInfo>`, `queue: VecDeque<Notice>`, `current: Option<(Notice, Instant /*expires*/)>`, `waiting: Option<Notice>`, `fullscreen: bool`, `hover: bool`). `Tick` promotes from queue when `current` expired.
- [ ] **Step 4:** `cargo test state::` → PASS.
- [ ] **Step 5: Commit** `feat: island state reducer`.

---

### Task 4: Config

**Files:** Create `src/config.rs`

**Interfaces:**
- Produces: `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(default)] pub struct Config { pub style: Style, pub battery_threshold: u8, pub start_with_windows: bool, pub muted_apps: Vec<String>, pub onboarding_done: bool }`, `pub enum Style { Pill, Notch }`; `impl Default` = `Pill, 20, true, ["Claude","Orca"], false`; `pub fn parse(text: &str) -> Config`; `pub fn load() -> Config`; `pub fn save(&self) -> std::io::Result<()>`; `pub fn path() -> PathBuf` (`%APPDATA%\DynamicIsland\config.json`).

- [ ] **Step 1: Failing tests:** `parse("")` → default; `parse("{not json")` → default; `parse(r#"{"style":"Notch"}"#)` → `style == Notch` and other fields default; `parse(&serde_json::to_string(&c).unwrap()) == c` round-trip.
- [ ] **Step 2:** `cargo test config::` → FAIL.
- [ ] **Step 3:** Implement; `load` = read file or empty string → `parse`; `save` creates the folder, writes pretty JSON.
- [ ] **Step 4:** `cargo test config::` → PASS.
- [ ] **Step 5: Commit** `feat: config file`.

---

### Task 5: Notification source

**Files:** Modify `src/sources/notifications.rs`, create `src/sources/mod.rs`

**Interfaces:**
- Consumes: `Notice`, `NoticeKind::App`, `Event` (Task 3); `Config.muted_apps` (Task 4).
- Produces: `pub struct NotificationTracker` with `pub fn new() -> Self` and `pub fn update(&mut self, current: &[u32]) -> Vec<u32>`; `pub fn is_muted(source: &str, muted: &[String]) -> bool` (case-insensitive substring); `pub fn spawn_poller(tx: Sender<Event>, wake: impl Fn() + Send + 'static, muted: Vec<String>)`; `pub fn open_app(app_id: &str)`.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn first_update_primes_without_showing() {
    let mut t = NotificationTracker::new();
    assert!(t.update(&[1, 2, 3]).is_empty());
    assert_eq!(t.update(&[1, 2, 3, 4]), vec![4]);
}
#[test] fn returns_new_ids_in_given_order() { /* prime [1]; update [1,7,5] → [7,5] */ }
#[test] fn forgets_ids_no_longer_present() { /* prime [1,2]; update [2] → []; internal seen.len() == 1 */ }
#[test] fn muted_match_is_case_insensitive_substring() {
    let m = vec!["Claude".to_string(), "Orca".to_string()];
    assert!(is_muted("Claude", &m)); assert!(is_muted("orca agent", &m)); assert!(!is_muted("WhatsApp", &m));
}
```

- [ ] **Step 2:** `cargo test notifications::` → FAIL.
- [ ] **Step 3:** Implement tracker and `is_muted`. `spawn_poller`: thread with `CoInitializeEx(COINIT_MULTITHREADED)`; if `GetAccessStatus() != Allowed`, exit thread (onboarding in Task 11 asks). Every 1000 ms: get toasts, `tracker.update(ids)`, map each new one to `Notice { key: id.to_string(), source: app name, title: first text element, body: remaining text elements joined by "\n", kind: App { app_id: AppInfo().AppUserModelId().ok() } }`, skip if muted, `tx.send(Event::Notice(..))`, `wake()`. Swap `probe()` to use the tracker. `open_app`: `Command::new("explorer.exe").arg(format!("shell:AppsFolder\\{app_id}")).spawn()`, ignore errors.
- [ ] **Step 4:** `cargo test notifications::` → PASS.
- [ ] **Step 5: Commit** `feat: notification poller`.

---

### Task 6: Media source

**Files:** Create `src/sources/media.rs`

**Interfaces:**
- Consumes: `MediaInfo`, `MediaCommand`, `Event` (Task 3).
- Produces: `pub fn spawn_watcher(tx: Sender<Event>, wake: impl Fn() + Send + Sync + 'static)`; `pub fn send_command(cmd: MediaCommand)`.

- [ ] **Step 1: Implement `spawn_watcher`.** Thread, MTA. `GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()`. Subscribe `CurrentSessionChanged`; for the current session subscribe `MediaPropertiesChanged` and `PlaybackInfoChanged`. On any of them: build `MediaInfo` (`TryGetMediaPropertiesAsync` → `Title`, `Artist`, `Thumbnail` read fully into `Vec<u8>` via `DataReader`; `GetPlaybackInfo().PlaybackStatus() == Playing`), send `Event::Media(Some(..))`, or `Event::Media(None)` when there is no current session. Emit once at start. Keep the thread alive parked; re-subscribe when the session changes.
- [ ] **Step 2: Implement `send_command`.** Fetch the current session and call `TrySkipPreviousAsync` / `TryTogglePlayPauseAsync` / `TrySkipNextAsync`; ignore errors.
- [ ] **Step 3: Manual check.** Add a temporary `--probe-media` arg printing each `MediaInfo` (title, artist, playing, art byte count). Play a Spotify or YouTube track: lines appear on play, pause and next. Remove the probe arg afterwards.
- [ ] **Step 4: Commit** `feat: media session watcher`.

---

### Task 7: Battery and fullscreen

**Files:** Create `src/sources/battery.rs`, `src/sources/fullscreen.rs`

**Interfaces:**
- Produces: `pub struct BatteryMonitor { .. }` with `pub fn new(threshold: u8) -> Self`, `pub fn update(&mut self, percent: u8, charging: bool) -> bool` (true = raise alert now); `pub fn read_battery() -> Option<(u8, bool)>` (`GetSystemPowerStatus`; `None` when `BatteryLifePercent == 255`); `pub fn battery_notice(percent: u8) -> Notice` (`source: "Baterai"`, `title: "Baterai lemah"`, `body: format!("Sisa {percent}%. Colokin charger.")`, `kind: BatteryLow`, `key: "battery"`); `pub fn is_fullscreen() -> bool` (`SHQueryUserNotificationState` ∈ {`QUNS_BUSY`, `QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_PRESENTATION_MODE`}).

- [ ] **Step 1: Failing tests** (threshold 20):

```rust
#[test] fn alerts_once_when_crossing() {
    let mut b = BatteryMonitor::new(20);
    assert!(!b.update(25, false)); assert!(b.update(20, false)); assert!(!b.update(19, false));
}
#[test] fn charging_never_alerts_and_rearms() {
    let mut b = BatteryMonitor::new(20);
    assert!(b.update(18, false)); assert!(!b.update(18, true)); assert!(b.update(18, false));
}
#[test] fn rearms_after_rising_above_threshold() { /* 19 → true; 30 → false; 20 → true */ }
```

- [ ] **Step 2:** `cargo test battery::` → FAIL. **Step 3:** implement. **Step 4:** → PASS.
- [ ] **Step 5: Manual check.** Temporary `--probe-system` arg prints `read_battery()` and `is_fullscreen()` every second: fullscreen a YouTube video → `true`; exit → `false`. Remove afterwards.
- [ ] **Step 6: Commit** `feat: battery monitor and fullscreen detection`.

---

### Task 8: Claude hook bridge

**Files:** Create `src/claude/mod.rs`, `src/claude/hook.rs`, `src/claude/pipe_server.rs`, `src/claude/install.rs`

**Interfaces:**
- Consumes: `Notice`, `NoticeKind::{ClaudeDone, ClaudeWaiting}`, `Event::ClaudeActivity` (Task 3).
- Produces:
  - `#[derive(Serialize, Deserialize, Debug, PartialEq)] pub struct HookMessage { pub kind: HookKind, pub session_id: String, pub project: String, pub message: Option<String> }`, `pub enum HookKind { Done, Waiting, Activity }`
  - `pub fn parse_hook_input(json: &str) -> Option<HookMessage>`
  - `pub fn to_event(msg: HookMessage) -> Event` — `Done` → `Notice { source: "Claude", title: "Claude selesai", body: project, kind: ClaudeDone{session_id}, key: format!("claude-done-{session_id}") }`; `Waiting` → `Notice { title: "Claude butuh respon", body: message.unwrap_or(project), kind: ClaudeWaiting{..}, key: format!("claude-wait-{session_id}") }`; `Activity` → `Event::ClaudeActivity{session_id}`.
  - `pub fn send_to_pipe(msg: &HookMessage) -> bool` and `pub fn run_hook()` (read stdin, parse, send, always exit 0, print nothing).
  - `pub fn spawn_pipe_server(tx: Sender<Event>, wake: impl Fn() + Send + 'static)` — `\\.\pipe\dynamic-island`, one JSON line per connection.
  - `pub fn add_island_hooks(settings: &mut serde_json::Value, command: &str) -> bool` and `pub fn install_hooks()`.

Mapping of `hook_event_name`: `Stop` → `Done`; `Notification` → `Waiting` (uses `message`); `UserPromptSubmit`, `PostToolUse` → `Activity`; anything else → `None`. `project` = last component of `cwd`, splitting on both `/` and `\`.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn parses_stop() {
    let m = parse_hook_input(r#"{"hook_event_name":"Stop","session_id":"s1","cwd":"D:\\GitHub\\Dynamic Island"}"#).unwrap();
    assert_eq!(m, HookMessage { kind: HookKind::Done, session_id: "s1".into(), project: "Dynamic Island".into(), message: None });
}
#[test] fn parses_notification_with_message() { /* message "Claude needs your permission to use Bash" kept */ }
#[test] fn activity_events() { /* UserPromptSubmit and PostToolUse → Activity */ }
#[test] fn unknown_event_or_bad_json_is_none() { /* "PreCompact" → None; "{" → None */ }
#[test] fn forward_slash_cwd() { /* "/home/x/proj" → project "proj" */ }
#[test] fn send_to_missing_pipe_is_silent_and_fast() {
    // with no server running: let t = Instant::now(); assert!(!send_to_pipe(&msg)); assert!(t.elapsed() < Duration::from_millis(200));
}
#[test] fn add_hooks_preserves_existing_and_is_idempotent() {
    let mut v = serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}});
    assert!(add_island_hooks(&mut v, "\"C:\\di.exe\" --hook"));
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
    for e in ["Stop","Notification","UserPromptSubmit","PostToolUse"] { assert!(v["hooks"][e].is_array()); }
    assert!(!add_island_hooks(&mut v, "\"C:\\di.exe\" --hook"));
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
}
#[test] fn add_hooks_to_empty_object() { /* json!({}) → true, four events present */ }
```

- [ ] **Step 2:** `cargo test claude::` → FAIL.
- [ ] **Step 3: Implement.** Each added group: `{"hooks":[{"type":"command","command":<command>,"timeout":5}]}`, plus `"matcher":"*"` for `PostToolUse`. "Already present" = any hook command in that event's array equals `command`. `send_to_pipe` opens the pipe with `CreateFileW` (no wait; fail fast if absent). `install_hooks`: command = `"\"<current_exe>\" --hook"`, back up `settings.json` to `settings.json.bak-dynamic-island`, merge, write pretty JSON, print only `Hook Dynamic Island terpasang.` or `Sudah terpasang.`.
- [ ] **Step 4:** `cargo test claude::` → PASS.
- [ ] **Step 5: Commit** `feat: claude hook bridge`.

---

### Task 9: Window and rendering

**Files:** Create `src/ui/mod.rs`, `src/ui/geometry.rs`, `src/ui/window.rs`, `src/ui/render.rs`

**Interfaces:**
- Consumes: `View`, `MediaInfo`, `Hit` (Task 3); `Style` (Task 4).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct ShapeRect { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub radius: f32 }`
  - `pub const WINDOW_W: f32 = 440.0; pub const WINDOW_H: f32 = 150.0; pub const ANIM_MS: u64 = 380;`
  - `pub enum ShapeKind { Compact, MediaExpanded, Notice }` and `pub fn shape_for(style: Style, kind: ShapeKind) -> ShapeRect` (logical px)
  - `pub fn lerp_shape(a: ShapeRect, b: ShapeRect, t: f32) -> ShapeRect`, `pub fn ease_out_back(t: f32) -> f32` (c1 = 1.2)
  - `pub struct IslandWindow` with `pub fn create(wnd_proc: WNDPROC) -> Result<Self>`, `pub fn hwnd(&self) -> HWND`, `pub fn reposition(&mut self)`, `pub fn scale(&self) -> f32`
  - `pub struct Renderer` with `pub fn new() -> Result<Self>`, `pub fn draw(&mut self, win: &IslandWindow, style: Style, shape: ShapeRect, view: &View, media: Option<&MediaInfo>, opacity: f32) -> Result<()>`, `pub fn hit_test(&self, x: f32, y: f32) -> Option<Hit>` (regions from the last draw)

Shape table (logical px). Pill: y = 8; radius = h/2 for Compact, 26 otherwise. Notch: y = 0; bottom radius 16 for Compact, 22 otherwise, plus 12 px inward shoulders on both top corners (drawn outside `w`). x always centers in `WINDOW_W`.

- [ ] **Step 1: Failing tests** in `geometry.rs`:

```rust
#[test] fn pill_compact_is_centered_below_top() {
    assert_eq!(shape_for(Style::Pill, ShapeKind::Compact), ShapeRect { x: 125.0, y: 8.0, w: 190.0, h: 34.0, radius: 17.0 });
}
#[test] fn notch_is_flush_with_top() { assert_eq!(shape_for(Style::Notch, ShapeKind::Notice).y, 0.0); }
#[test] fn sizes_match_plan() { /* MediaExpanded 380x110, Notice 360x76 for both styles */ }
#[test] fn ease_endpoints_and_overshoot() {
    assert!(ease_out_back(0.0).abs() < 1e-6); assert!((ease_out_back(1.0) - 1.0).abs() < 1e-6);
    let peak = (0..=100).map(|i| ease_out_back(i as f32 / 100.0)).fold(0.0, f32::max);
    assert!(peak > 1.0 && peak < 1.1);
}
#[test] fn lerp_halfway() { /* lerp_shape(a,b,0.5).w == (a.w+b.w)/2 */ }
```

- [ ] **Step 2:** `cargo test geometry::` → FAIL. **Step 3:** implement geometry. **Step 4:** → PASS.
- [ ] **Step 5: Window.** `CreateWindowExW` with `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, style `WS_POPUP`. Physical size = logical × `GetDpiForWindow / 96`. Position: top-center of the primary monitor's `rcMonitor` (`MonitorFromPoint(0,0, MONITOR_DEFAULTTOPRIMARY)`). `reposition()` is called on `WM_DPICHANGED` and `WM_DISPLAYCHANGE`.
- [ ] **Step 6: Renderer.** 32-bpp top-down premultiplied DIB section the size of the window; `ID2D1DCRenderTarget` (`D2D1_ALPHA_MODE_PREMULTIPLIED`) bound to it; `UpdateLayeredWindow(.., ULW_ALPHA)` with `SourceConstantAlpha = opacity*255`. Everything outside the shape is alpha 0 (click-through). Shape: pure black geometry built from the rect + radius (+ shoulders for Notch). Text: DirectWrite `Segoe UI Variable` (fallback `Segoe UI`), white; title 13 px semibold, secondary 12 px at 70 % opacity; `DWRITE_TRIMMING_GRANULARITY_CHARACTER` with an ellipsis sign, word wrap off. Album art: decode `art` bytes with WIC → `ID2D1Bitmap`, cache by byte length + first 64 bytes. Layouts follow the mockup: compact = art 22 px + artist + 4 green equalizer bars (static); media expanded = art 40 px, title, artist, three buttons (⏮ ⏯ ⏭ drawn as glyphs from `Segoe Fluent Icons`); notice = 40 px circular badge with the app's first letter (green for WhatsApp, Claude orange `#D97757` for Claude kinds, amber for battery, grey otherwise) + title + body (one line each).
- [ ] **Step 7: Manual check.** Temporary `--demo` arg: cycles Hidden → MediaCompact → MediaExpanded → Notice (with `-Long`-sized text) every 2 s for both styles, animating with `lerp_shape(ease_out_back(t))`. Expected: smooth morph, text trimmed with "…", clicks on transparent area reach the window behind (click the desktop around the island). Remove `--demo` afterwards or keep it behind `#[cfg(debug_assertions)]`.
- [ ] **Step 8: Commit** `feat: layered island window and renderer`.

---

### Task 10: App loop and input

**Files:** Create `src/app.rs`; Modify `src/main.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–9.
- Produces: `pub fn run()`.

- [ ] **Step 1: Wire it.** `run()`: single-instance guard (`CreateMutexW("Local\\DynamicIsland")`, exit if it already exists); `CoInitializeEx(COINIT_APARTMENTTHREADED)`; load `Config`; create window, renderer, `IslandState`; channel `(tx, rx)`; `wake` = `PostMessageW(hwnd, WM_APP_EVENT, ..)` (HWND passed as `isize` to threads); spawn notification poller, media watcher, pipe server. Timers: `TIMER_SYSTEM` every 1000 ms (fullscreen → `Event::Fullscreen` only on change), `TIMER_BATTERY` every 30 s (`BatteryMonitor` → `Event::Notice(battery_notice(p))`), `TIMER_ANIM` at 16 ms only while an animation runs, `TIMER_DEADLINE` one-shot at `state.next_deadline()` → `Event::Tick`.
- [ ] **Step 2: Event handling.** On `WM_APP_EVENT` drain `rx` → `state.apply`. After every apply: run effects (`OpenApp` → `open_app`, `Media` → `send_command`), compute the target shape from `view()` (Hidden → current shape with opacity → 0), start/retarget the animation from the current interpolated shape, reset `TIMER_DEADLINE`.
- [ ] **Step 3: Input.** `WM_MOUSEMOVE` → `TrackMouseEvent(TME_LEAVE)` once, `Event::Hover(true)`; `WM_MOUSELEAVE` → `Hover(false)`; `WM_LBUTTONUP` → `renderer.hit_test` → `Event::Click(hit)`; `WM_RBUTTONUP` → tray menu (Task 11). `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`.
- [ ] **Step 4: Manual check.** `cargo build --release`, run the exe. Play music → compact pill; hover → expanded; ⏯ pauses; `scripts\send-test-toast.ps1` → notice for 5 s then back to music; click a WhatsApp notice → WhatsApp opens. `echo '{"hook_event_name":"Stop","session_id":"x","cwd":"D:\\a\\Proj"}' | target\release\dynamic-island.exe --hook` → "Claude selesai / Proj".
- [ ] **Step 5: Commit** `feat: app loop wiring`.

---

### Task 11: Tray, settings, autostart, onboarding

**Files:** Create `src/ui/tray.rs`; Modify `src/app.rs`

**Interfaces:**
- Consumes: `Config` (Task 4), `IslandWindow` (Task 9), `install_hooks` (Task 8).
- Produces: `pub fn add_tray(hwnd: HWND) -> Result<()>`, `pub fn show_menu(hwnd: HWND, cfg: &Config) -> Option<MenuCmd>`, `pub enum MenuCmd { StylePill, StyleNotch, ToggleAutostart, ShowDndHelp, InstallClaudeHooks, Exit }`; `pub fn set_autostart(enabled: bool) -> Result<()>`.

Spec note: Windows has no public API to read whether Do Not Disturb is on, so the DND hint is shown once on first run (`onboarding_done`) and is available from the menu, instead of "only when DND is off".

- [ ] **Step 1: Tray.** `Shell_NotifyIconW(NIM_ADD)` with `IDI_APPLICATION` icon, tooltip `Dynamic Island`, callback `WM_APP_TRAY`. Menu (Indonesian, check marks for current values): `Gaya: Pill`, `Gaya: Notch`, separator, `Jalan saat Windows start`, `Cara matiin popup Windows`, `Pasang hook Claude Code`, separator, `Keluar`. Style change saves config and re-renders immediately.
- [ ] **Step 2: Autostart.** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `DynamicIsland` = `"<current_exe>"`; deleted when disabled. Apply `config.start_with_windows` at startup.
- [ ] **Step 3: Onboarding.** On first run (`!onboarding_done`): if listener access is not `Allowed`, call `RequestAccessAsync` on the UI thread; then show a temporary island notice (kind `App{app_id: None}`, source `Dynamic Island`, title `Matiin popup Windows`, body `Nyalain Do Not Disturb: Settings → System → Notifications`) and open `ms-settings:notifications`; set `onboarding_done = true` and save. `ShowDndHelp` repeats the notice and the settings link.
- [ ] **Step 4: Manual check.** Delete `config.json`, run: permission prompt (if not yet allowed), hint notice, Settings opens. Tray menu switches Pill ↔ Notch live. Toggle autostart and check `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v DynamicIsland`. `Pasang hook Claude Code` → `~/.claude/settings.json` gains the four hook events, Orca's entries remain.
- [ ] **Step 5: Commit** `feat: tray menu, autostart and onboarding`.

---

### Task 12: End-to-end verification

**Files:** Create `README.md` (setup steps: Task 1 toolchain, `scripts\package.ps1` + elevated cert import, DND, `Pasang hook Claude Code`)

- [ ] **Step 1:** `cargo test` → all pass. `cargo build --release` → no warnings that matter.
- [ ] **Step 2: Resource check.** Run the island idle for 2 min with music paused: `Get-Process dynamic-island | Select WorkingSet64, PrivateMemorySize64, CPU`. Expected: working set under 40 MB, CPU seconds not increasing between two reads 30 s apart. Record the numbers in the commit message.
- [ ] **Step 3: Review Focus walkthrough.** (1) restart the island with 10+ items in Notification Center → nothing old shows; (2) stop the island, finish a Claude turn → Claude not slowed, no hook error; (3) `scripts\send-test-toast.ps1 -Long` ×10 quickly → each shown, trimmed; (4) close Spotify while playing → island hides; (5) change display scale 100 % → 150 % and back → island stays centered and sized.
- [ ] **Step 4:** Real use: WhatsApp message, Claude Code turn end + permission prompt, fullscreen YouTube with a toast during it, battery threshold temporarily set to 99 in `config.json`.
- [ ] **Step 5: Commit** `docs: readme and verification notes`.
