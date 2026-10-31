---
tags: [decision, notifications, panel, privacy, dpapi, timers, v3.9]
date: 2026-10-31
---

# Notifikasi: the history of the toasts Vale catches

`Page::Notifications` (13th page, right after Clipboard, [[panel-pages]]): a
capped, optionally persisted list of the Windows toasts the island already
caught for the notch. Code: `src/notif/` (pure: `store`, `time`, `tile`, the
`persist` format), `src/app_notif.rs` (the island's side, a child of `app`),
`panel_layout::notif_layout`, `render_panel::paint_notifications`, the
Settings card in `settings_layout::notifications_card`.

## What is recorded

- **Every fresh toast the poller sees**, never what was in the Action Center
  when Vale started (`NotificationTracker` priming is untouched). The poller
  no longer filters: it sends `Event::Toast(Toast { notice, at, logo })` and
  `App::on_toast` decides with `notif::route_toast(source, muted, record,
  paused) -> (notch, history)`: muted -> nowhere; else the notch always, the
  history while *Simpan notifikasi* is on and *Jeda* is off. `muted_apps` is
  therefore read live (it used to be copied into the poller thread at start).
- Entry: id, unix seconds, read flag, app display name, AUMID (optional),
  title and body cut to 1024 characters. Newest first, capped 10-200 by 10
  (default 50); the oldest go first.
- **Vale never touches the Windows Action Center**: no `RemoveNotification`
  anywhere. The X removes a row from Vale's list only.

## Exclusion rule

*Kecualikan app ini* adds the entry's source to `config.muted_apps` (cut to
64 characters, any-case duplicates skipped) with `save_merged_with`, editing
only that field: `muted_apps` stays a file-owned field, it is **not** in
`merge_owned`. Every way `muted_apps` changes (the row button, Settings,
a hand edit picked up by `adopt_config`) ends in `notif_follow_config`, which
removes the entries now muted (`NotifHistory::remove_muted`, the same
case-insensitive substring rule as `is_muted`). Settings-added exclusions
therefore purge matching rows too. The deserializer is lenient: not a list ->
`[Claude, Orca]`; non-string items dropped; names trimmed; empty and
case-insensitive repeats dropped.

## File: `notification-history.bin`

Next to `config.json`, magic `DINH`, version u16 1, sealed with DPAPI
(current user, `dpapi.rs`), plaintext wiped, written to `.bin.tmp` and renamed
on a short-lived thread ordered by a generation counter (the shape of
[[clipboard-history]]'s file). Body: next id, count, entries (id, at, read,
source, app id flag and string, title, body), then the logo table (count; key,
w u16, h u16, len, BGRA premultiplied) holding **only logos some entry uses**.
Guards: `MAX_COUNT` 10 000, `MAX_FIELD` 4 MB, a logo side at most 64 and its
length exactly w x h x 4. Missing, foreign, wrong version, truncated, trailing
bytes or undecryptable -> an empty history, never a panic. *Simpan setelah
restart* off: nothing is written, the file is deleted at startup and when the
setting turns off. **Logos are persisted inside the encrypted file** (decision).
Tests only use `target/test-tmp/`, never `%APPDATA%`.

## Timer: `TIMER_NOTIF_SAVE` = 34

One coalesced one-shot, 1.5 s after the last change; a pending save is
flushed on `WM_DESTROY` and `WM_ENDSESSION` (where the clipboard flushes).
"Hapus semua" is **not** a timer of its own: its armed state lives in the
reducer next to the cleaner's (`notif_armed`, `CLEAN_CONFIRM` 4 s) and rides
`next_deadline` / `TIMER_DEADLINE`; it disarms on expiry, any other click, a
page change or the panel closing. See [[bata-idle-power-budget]]: no repeating
timer, relative times are worked out only when a frame is drawn.

## Keyboard: only by an explicit click

The island never takes the keyboard implicitly (`MA_NOACTIVATE`). A click on
the list's background or the title (the last region of the page,
`Hit::NotifFocus`) gives the page the keyboard through a fourth hidden 1 x 1
EDIT (`CliInput::create_with`, control id 204), exactly as
[[cli-panel-keyboard-focus]] says: remember the foreground, activate, focus
the EDIT, `Event::NotifFocus(true)` holds the panel open; Esc, a page change,
a collapse, `WA_INACTIVE` and Blur let go. Up/Down move the selection (the
list scrolls to keep it in view), Enter = Buka, Delete = X (a new
`cli_input::Key::Delete`, sent **only** by this EDIT: the other inputs keep
Delete for editing). Typed characters are dropped at once. Not offered
without listener access. Hover vs keyboard follows [[clipboard-history]]
("Hover selects"): `pointer_moved` ignores a move at the same spot and,
after a key move, the pointer selects nothing until it really moves; a wheel
scroll keeps the selection on a row that shows (`notif_visible_row`).
Decision: the selection starts empty (no first-row default as the
Clipboard has), so Enter and Delete never act on a row nobody lit.

## Access

`notifications::access_allowed()` is read at start (`notif_start`), when the
page opens and after the existing `WM_APP_ACCESS` answer; the poller starts
once, as soon as access is allowed (`poller_started`). The page never asks for
access: its no-access button opens `ms-settings:privacy-notifications`.

## Logo path

On the poller thread (MTA, never the UI thread), the first time a logo key
(AUMID, else the app name) is seen this run:
`AppInfo().DisplayInfo().GetLogo(48 x 48)` -> `OpenReadAsync` -> bytes (cap
1 MB) -> WIC (`SHCreateMemStream`, `CreateDecoderFromStream`, a Fant scaler
only when larger than 48, `32bppPBGRA`, `CopyPixels`) -> `Thumb`. Any failure
-> none and a letter tile (`notif::tile`: a hue from an FNV hash of the
lowercase name, the first letter); a key that failed is not retried this run.
The renderer caches one `ID2D1Bitmap` per key (`notif_logos`), dropping keys
no longer drawn each frame. The WIC decode of real logos is **not verified
with a real toast** here (the demo draws a made-up logo); only the garbage
case and the size arithmetic are tested.

## Page

Header: title, "n / max", the *Jeda* switch, *Hapus semua*. A one-line note
under it while *Simpan notifikasi* is off. One column of rows (44 dip on a
48 pitch, whole rows only, four fit), selected row lit with its three
buttons; the wheel steps one row a notch (`notif_wheel_step` = `clip_wheel_step`
rules: nothing to scroll -> the notch turns the page, an end spends it); a 3 px
scrollbar thumb in the right margin. Empty states: "Belum ada notifikasi",
"Izin notifikasi belum diberikan" with *Buka pengaturan*. Relative times:
`notif::time::relative_time` (baru saja, N mnt, HH.MM, Kemarin, d Mon, plus
the year when it differs). Demo scenes: `notifications`, `-selected`, `-long`,
`-empty`, `-no-access`, `-paused`, `-armed` (fixed fake data and a fixed
"now").

Related: [[clipboard-privacy]], [[settings-structure]], [[devices]]
