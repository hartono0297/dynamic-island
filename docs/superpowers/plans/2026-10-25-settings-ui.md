# Settings UI, themes, language and logo — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A minimalist, modern settings window for Dynamic Island with themes (accent colour, island surface, light/dark mode for windows), language (Bahasa Indonesia / English), island style, and the other user settings; plus an app logo used everywhere.

**Owner request (2026-10-25):** "UI tempat setting2nya … bisa ada tema, bisa ganti bahasa, ganti style, dll, bikin UI-nya bagus minimalist dan modern … jangan lupa bikin logo." The owner asked to build it directly, without a brainstorming round; the design decisions below are the controller's.

**Architecture:** Same process. A new ordinary top-level settings window (not layered) drawn with Direct2D/DirectWrite like the island, with pure layout + hit-test functions; native EDIT children only for the two Google credential fields. Settings changes go through `App` (config merge/save from v2 — island-owned fields) and apply live. A small i18n module supplies every user-facing string in two languages. A palette module turns config (accent, surface, mode) into colours for the island renderer, editors and settings window.

**Tech stack:** existing (Rust, `windows` 0.62, D2D/DWrite/WIC, `serde`, `chrono`). No new crates.

## Pinned design

| Item | Value |
|---|---|
| Settings window | 720 × 520 logical px, resizable no, centered on the primary monitor's work area, `WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX`, DWM immersive dark/light title bar per mode, Mica backdrop (`DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW`) when available, else solid background. Single instance (focus existing). |
| Layout | Left sidebar 184 px: logo (28 px) + "Dynamic Island" at top, then section rows (icon glyph from Segoe Fluent Icons + label, 36 px tall, 8 px radius selection pill in a subtle tint of the accent). Right content: section title 20 px semibold, then cards (12 px radius, 1 px hairline border) each with rows 52 px: label (14 px) + description (12 px, secondary) on the left, control on the right. 24 px outer padding, 12 px between cards. |
| Sections | **Tampilan / Appearance**, **Umum / General**, **Pomodoro**, **Integrasi / Integrations**, **Tentang / About** |
| Tampilan | Gaya island: segmented [Pill | Notch] · Warna aksen: 6 round swatches (24 px, selected has 2 px ring) · Latar island: segmented [Hitam | Grafit] · Mode jendela: segmented [Ikuti Windows | Gelap | Terang] · live preview card at top showing the compact island in the chosen style/surface/accent (equalizer bars in accent) |
| Umum | Bahasa: segmented [Indonesia | English] · Jalan saat Windows start: toggle · Peringatan baterai: stepper 5–50 % step 5 (hidden note "Tidak ada baterai" when `read_battery()` is None) · Cara matikan popup Windows: button "Buka petunjuk" |
| Pomodoro | Fokus / Istirahat / Istirahat panjang / Jumlah sesi: steppers with the v2 ranges (disabled while a pomodoro is running, with a note) |
| Integrasi | Google Calendar card: status line (Belum diatur / Belum terhubung / Terhubung / Perlu login ulang / Offline) + Client ID and Client secret native EDIT fields (secret masked with `ES_PASSWORD`) + buttons [Simpan] [Hubungkan] [Putuskan] (Putuskan = delete token file); Claude Code card: status (Terpasang / Belum) + [Pasang hook] [Hapus hook]; link "Panduan setup Google" opens README anchor on GitHub |
| Tentang | Logo 64 px, "Dynamic Island", version from `CARGO_PKG_VERSION`, links: GitHub repo, Releases (check for updates) — open in browser; data folder button "Buka folder data" (`%APPDATA%\DynamicIsland`) |
| Accent palette | Biru `#4F8CFF` (default), Ungu `#A78BFA`, Hijau `#30D158`, Oranye `#FF9F0A`, Pink `#FF6482`, Merah `#FF453A` |
| Accent use (island) | today fill ring/selection, event dot fallback, equalizer bars, toggles/segmented/selection in settings, primary buttons, page dot active. Pomodoro keeps red (focus) / green (break) semantics. |
| Island surface | Hitam `#000000` (default), Grafit `#1C1C1E`; panel cards/rows lighten relative to surface |
| Window modes | Dark: bg `#1B1B1D`, card `#242427`, border `#FFFFFF14`, text `#F2F2F2`, secondary `#A0A0A5`. Light: bg `#F3F3F5`, card `#FFFFFF`, border `#00000014`, text `#1B1B1D`, secondary `#6B6B70`. "Ikuti Windows" reads `AppsUseLightTheme` (HKCU Personalize) and follows `WM_SETTINGCHANGE` "ImmersiveColorSet". Editors (notes/event) follow the same mode. The island itself stays dark by design (surface option only). |
| Controls | Toggle 40×22 (knob 16), segmented control 28 px tall with 3 px inner padding and sliding selection (120 ms ease-out), stepper `−  value  +` 28 px buttons, buttons 32 px tall 8 px radius (primary = accent fill, secondary = subtle fill). Hover states (pointer tracking), focus not required (mouse UI) but Tab/Enter/Esc: Esc closes the window. |
| Animations | Section switch: content crossfade 120 ms; control state changes 120 ms. Driven by the existing `FrameClock` pattern or a 16 ms timer only while animating. No idle redraws. |
| Language | `language: "id" \| "en"` in config (island-owned), default `id`. All user-facing strings (island notices, panel labels, weekday/month names, tray menu, editors, settings, onboarding, OAuth browser pages) via `i18n`. Dates: Indonesian vs English month/day names. Hook/CLI stdout strings may stay Indonesian. |
| Config | New island-owned fields: `language`, `accent` (enum `Blue|Purple|Green|Orange|Pink|Red`), `surface` (`Black|Graphite`), `window_theme` (`System|Dark|Light`). Merge rules from v2 apply (`merge_owned` updated). |
| Tray menu | Simplified: "Pengaturan…" (opens settings), separator, "Gaya: Pill/Notch" kept, "Keluar". Other items move into settings. Island right-click opens the same menu. |
| Logo | Concept: a dark squircle tile with the island pill (white) near the top and a small accent-blue dot at its right end (the "camera"), subtle top-to-bottom gradient on the tile (`#2A2A2E` → `#0E0E10`). Source `assets/logo.svg`; generated `assets/icon.ico` (16, 20, 24, 32, 40, 48, 64, 128, 256 px) and `assets/logo-256.png` by `scripts/make-icon.ps1` (System.Drawing, anti-aliased), committed. Embedded via `app.rc` (`1 ICON "assets/icon.ico"`), used as the window-class icon (settings, editors), tray icon (16/20/24/32 by DPI via `LoadIconWithScaleDown`), Start Menu shortcut (exe icon), README header. At 16 px the pill must stay legible (snap to pixels). |

## Tasks

### Task 1: Logo and icon
Files: `assets/logo.svg`, `scripts/make-icon.ps1`, `assets/icon.ico`, `assets/logo-256.png`, `app.rc`, `src/ui/tray.rs`, `src/ui/editor.rs` (class icon), `README.md` (header image).
- [ ] Draw the logo (SVG); script renders PNGs at all sizes (pixel-snapped small sizes) and packs a multi-size ICO (PNG-compressed 256, BMP for ≤ 64 is fine). Read the PNGs to check legibility at 16/24/32.
- [ ] Embed icon resource; tray uses it (DPI-aware size); editor windows use it.
- [ ] Verify: exe shows the icon in Explorer (check via `ExtractIconEx` count ≥ 1 or screenshot of a folder), tray icon screenshot. Commit `feat: app logo and icon`.

### Task 2: Language (i18n)
Files: `src/i18n.rs` (new), all modules with user-facing strings, `src/config.rs`.
- [ ] `pub enum Lang { Id, En }`, `pub fn set_lang(Lang)` / `pub fn lang() -> Lang` (thread-safe global, e.g. `AtomicU8`), `pub fn tr(key: Key) -> &'static str` with `pub enum Key { … }` and `match (key, lang)` so a missing translation is a compile error; formatted strings via small helpers (`fn phase_done_body(...) -> String`). Month/day names for both languages.
- [ ] Replace every user-facing literal (grep for Indonesian text in `src/`) — island notices, panel, calendar labels and relative dates, pomodoro, notes times, tray, editors (labels, buttons, validation), onboarding, OAuth result pages, config/notes warnings. Tests that exercise strings set the language explicitly.
- [ ] Unit tests: every `Key` has a non-empty string in both languages (iterate a `Key::ALL` list), date formatting both languages. Commit `feat: indonesian and english ui strings`.

### Task 3: Themes
Files: `src/theme.rs` (new), `src/config.rs`, `src/ui/render.rs`, `src/ui/render_panel.rs`, `src/ui/editor.rs`.
- [ ] Config fields (`accent`, `surface`, `window_theme`) + merge rules + tests. `pub struct Palette { … }` from config (+ system light/dark for `System`), pure and tested.
- [ ] Island renderer uses the palette (surface + accent uses listed above); editors follow window mode (dark/light brushes, DWM title bar). Live change: a `Theme` change redraws island and restyles open editors.
- [ ] Screenshots of the island in each surface × 2 accents, and both editor modes. Commit `feat: accent colour, island surface and light/dark windows`.

### Task 4: Settings window
Files: `src/ui/settings.rs` (window + drawing), `src/ui/settings_layout.rs` (pure layout, hit regions), `src/app.rs` (open/close, apply changes), `src/ui/tray.rs` (menu).
- [ ] Pure layout per section with hit regions; tests for no overlaps and controls inside cards for both languages.
- [ ] Window, drawing per the pinned design, hover/press states, animations, native EDITs for Google credentials (themed), Esc closes, single instance, DPI changes.
- [ ] Every change applies live through `App` and is saved (island-owned fields; credentials saved through the v2 merge so hand edits stay safe; then `adopt_config`). Google status reflects the sync snapshot; Hubungkan/Putuskan work; Claude hook buttons call install/uninstall and show the result.
- [ ] Tray menu simplified as pinned. Screenshots of every section in dark and light, both languages. Commit `feat: settings window`.

### Task 5: Wiring check, README, release
- [ ] README: settings section (with a screenshot from Task 4 saved to `assets/screenshots/settings.png`), language note; English summary at the top for non-Indonesian visitors.
- [ ] Full tests/clippy/fmt/release build; idle CPU unchanged; RAM with settings open and after closing.
- [ ] Bump version in `Cargo.toml` to 2.1.0. Commit `docs: settings, themes and language`.
