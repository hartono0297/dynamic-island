---
tags: [decision, volume, media, audio, v3.2.1]
date: 2026-10-29
---

# Volume by scroll (v3.2.1)

While a song plays, the wheel over the music turns the volume, **2 % a notch**,
on the **master** (default output) level, not the playing app's own session:
the app behind a media session cannot be told apart reliably (browser tabs
share one id), and `audio::Target::Master` is what the Mixer already sets.

- **Where:** the compact notch (`View::MediaCompact` and `MediaLyric`) anywhere
  on it, and the Music page **only over the cover** (`HitRegions::media_art`).
  Everywhere else, including the rest of the Music page, the wheel still turns
  pages, so page-by-page scrolling is never stuck on Music. In Hover open mode
  the panel opens within ~120 ms of the pointer arriving, so the compact notch
  is reachable mainly in Click mode or in that dwell; the cover is the usual
  place. Decision function: `app::volume_wheel_applies` (pure, tested).
- **Only while it plays:** the caller passes `m.playing`, not "a song is
  there", so over a paused song the wheel turns pages (3.2.1 review fix; it
  used to pass `media().is_some()`).
- **Setting:** `config.scroll_volume` (default on, hand-edited like
  `gestures`; no Settings row).
- **Mechanics:** `audio::wheel_points` adds wheel deltas (120 a notch) into
  percent points and keeps the remainder, so a touchpad's small deltas count;
  `audio::move_points` moves a level inside 0..100. The first turn reads the
  master level through the Mixer's `Scan` (`volume_pending` waits for the
  reply in `on_audio`); the level is then kept while the plate shows
  (`TIMER_VOLUME` 28, one-shot 1.2 s, then it is forgotten so an outside
  change is read again).
- **Feedback:** a dark plate (`render_panel::paint_volume`, layout
  `panel_layout::volume_hud_layout`): speaker, bar in the accent colour and the
  percentage. Over the cover it is translucent (0.72); on the compact notch it
  is opaque (0.97) so the title under it does not show through. Demo scenes
  `media-volume` (notch) and `music-volume` (page), shots `notch-volume` and
  `music-volume`.

**Not verified live:** the real master volume changing on a PC, a touchpad's
smooth deltas, and how the plate feels on the real compact island.

Related: [[mixer]], [[gestures]].
