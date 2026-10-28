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
