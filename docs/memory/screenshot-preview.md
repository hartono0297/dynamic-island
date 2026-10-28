---
tags: [decision, screenshot, clipboard, privacy, power]
date: 2026-10-28
---

# Screenshot preview in the notch

Run 8 of the v3.1 plan (keputusan #6). Code: `src/screenshot/` (pure:
matching, PNG/JPEG headers, folder listing; Win32: `win`) and
`src/app_shot.rs` (the island's side). The Pintasan Screenshot tile stays
`ms-screenclip:`.

- **Detection.** A new clipboard image from the history's listener
  ([[clipboard-history]], `clip_image_arrived`) becomes a candidate. With
  only bitmap formats on the clipboard (PrtSc, some Snipping Tool paths) the
  preview shows at once; any other image shows only once its file is found
  in the Screenshots folder (Win+Shift+S with auto-save, PrtSc to Snipping
  Tool). A browser's "Copy image" never matches a file, so it never shows.
  The folder comes from `FOLDERID_Screenshots` (follows OneDrive), never a
  guessed path.
- **Matching** (`screenshot::pick_match`, pure, tested): a file counts from
  3 s before to 5 s after the clipboard image, with the very same width and
  height, and an 8 × 8 average hash (`thumb_hash`) within 10 bits of the
  clipboard thumbnail's. The file side is decoded by WIC at the clipboard
  thumbnail's size (`thumb_size`) so both hashes see alike pixels. A file
  whose hash could not be read (still being written) is allowed on time and
  size but loses to a hash-confirmed one; then the closest in time wins.
- **No polling.** One short-lived worker lists the folder when an image
  arrives; `TIMER_SHOT_RETRY` (23, one-shot) looks twice more (1.0 s, 1.8 s
  later). Then nothing runs until the next image ([[bata-idle-power-budget]]).
- **One card.** The screenshot is the history's image card; the file is
  never ingested, so file and clipboard never make two cards. PrtSc twice on
  an unchanged screen bumps the card and previews it again.
- **The notice.** `NoticeKind::Screenshot` on `ShapeKind::NoticeShot`
  (440 × 156, radius 26): the 230 × 136 thumbnail (drawn whole), Bata happy
  or the Camera badge, "1280 × 720 · 245 KB" (the file's size once found,
  else the bitmap's) and "sudah disalin", then Salin lagi / Simpan / Hapus.
  It shows `show_secs` (8 by default) from when it becomes current, never
  expires under the pointer, and gets 2 s more after the pointer leaves. A
  newer screenshot replaces it (and a "Disalin"); never over a fullscreen
  app. "Disimpan · Pictures\Screenshots" (happy) and "Dihapus · clipboard
  dikosongkan" (Bata at rest) follow the buttons.
- **Clicks** use the press-release rule ([[media-click-guards]]) through
  `shot_regions`; the thumbnail opens the file in its default app
  (`ShellExecuteW`), the island never takes the foreground.
- **Nothing is deleted without Hapus.** Only `Hit::ShotDelete` gives
  `ShotAction::Delete`. "Jika diabaikan: Hapus" (opt-in; Simpan is the
  default) gives `ShotAction::DeleteIgnored` only when the preview ran its
  full time with no visit, no open and no other click, and `App` deletes
  then only if the setting is still Hapus (`delete_when_ignored`). Expiry,
  Body click, a newer shot or a game always keep the file.
- **Hapus** moves the file to the Recycle Bin through the cleaner's
  `IFileOperation` recycle on an STA worker (no file matched yet: one last
  look, same rules), removes the history card, and empties the clipboard
  only while `GetClipboardSequenceNumber` is still the one read with the
  image (checked again with the clipboard open, `should_empty`).
- **Settings → Clipboard**, a second card under an eyebrow "Screenshot":
  Preview di notch (on), Jika diabaikan (Simpan | Hapus, Simpan), Lama tampil
  (4–30 s, step 2, default 8). The section now scrolls
  ([[settings-appearance-scroll]]). Old config.json without `screenshot`
  loads the defaults. The clipboard listener runs while the history **or**
  the preview is on; with the history off only images are looked at and
  nothing is kept.
- **Tests and demo** never touch the real clipboard, folder or files: the
  matching runs on fakes, the folder test on `target/test-tmp/`, the demo
  (`notch:screenshot`, `-saved`, `-deleted`) draws a fixed picture with
  fixed figures ([[clipboard-privacy]], [[screenshot-capture-script]]).

## Verify pass (2026-10-28)

- **Never another screenshot's file.** `ClipCandidate::from_ms`
  (`claim_from`): when an image arrived shortly before, files before this
  one's arrival belong to that one. `Shots::taken` (last 16) lists files
  already matched; they are never matched again. Before this, a retake within
  a few seconds could match the earlier file, and Hapus recycled it.
- Simpan without a file (PrtSc with no auto-save) says "Belum ada file · tetap
  di clipboard", not "Disimpan".

- Hapus never removes a pinned history card (`card_to_remove`).
- A new image gives the shown screenshot one last file look before it takes
  over the retries (one retry slot only).

Related: [[bata-robot-expressions]], [[system-page]]

Listed in [[index]].

## v3.2 Run 3: the preview always shows

Why it could stay silent in 3.1.0 (read from the code; the Windows side was
not seen live):

1. `only_bitmaps` was an allow-list: every registered format name had to be
   exactly "PNG". Any extra format a screenshot tool registers turned the
   instant preview off, and the preview then waited for a FILE; Win+Shift+S
   often writes none.
2. A screen clip's overlay is a fullscreen window: `SHQueryUserNotificationState`
   says BUSY until it closes, and the island's own poll is up to a second
   behind. `shot_from_image` dropped the image when the cached flag said
   fullscreen, with no retry (and the reducer drops screenshot notices too
   while fullscreen).
3. The file looks stopped at about 2.8 s, short of the 5 s match window.

Now:

- `picture_only` (`clipboard/win.rs`) is a deny-list: a bitmap and none of
  text, files, a metafile, or HTML / rich text / Office / shell-list formats
  (`NOT_A_SCREENSHOT`). A browser's "Copy image" carries HTML Format, so it is
  still no screenshot.
- `fullscreen_now` asks Windows again before trusting the cached flag;
  `try_show_waiting` waits `SHOW_RETRY_MS` (1.2 s) up to `SHOW_TRIES` (3)
  times on `TIMER_SHOT_SHOW` (25), and gives up only for a game or a
  presentation. A flagged picture without a file is never dropped.
- `RETRY_MS` = 0.6, 0.9, 1.5, 2.5 s after the first look: the looks cover the
  whole match window.
- Simpan with no file writes one: `spawn_save` encodes the clipboard DIB at
  full size as a PNG through WIC on a worker, in the Screenshots folder (made
  if missing), `Screenshot YYYY-MM-DD HHMMSS.png`, " (2)"... never over
  another file (`save_name`). The path joins `taken`, so no later screenshot
  matches it. Failure says "Gagal menyimpan". Without the full picture (a
  history restored after a restart) it still says nothing was written.
- Must be verified live: the exact clipboard formats and owner of Win+Shift+S,
  PrtSc, Win+PrtSc and `ms-screenclip:`; whether Win+PrtSc touches the
  clipboard at all (the only trigger is `WM_CLIPBOARDUPDATE`; there is no
  folder watcher); whether Pictures is OneDrive-redirected (its files can look
  like reparse points, which `list_folder` skips).
