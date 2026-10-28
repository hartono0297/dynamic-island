---
tags: [decision, gotcha, lyrics, media, v3.2]
date: 2026-10-28
---

# Lyrics sync (v3.2 Run 4)

Lyrics ran behind or ahead of the song for four reasons, each handled:

- **Position model.** `Timeline` now carries `rate` (GSMTC `PlaybackRate`,
  odd or unreadable = 1.0); `position_at` runs at that speed. `timeline_moved`
  re-syncs when the position is more than `RESYNC_MS` (100) off where the
  last one says playback should be, or the length or rate changed (it was
  250 ms, which showed as a visible lag after a seek or a stutter).
- **Lookahead.** The line is picked at `lyric_position` = position + offset +
  `LOOKAHEAD_MS` (150): a line is on screen when it is sung, not a beat late
  (the notch redraws on a timer, so a line chosen exactly on time shows late).
- **Offset.** `config.lyrics_offset_ms`, ±2 s in 0.1 s steps
  (`OFFSET_RANGE`, `step_offset`), nudged by the −/value/+ buttons left of
  "Lirik" on the Music page (drawn only with lyrics on; the value click
  resets to 0). It is the island's own setting (`merge_owned`), read
  leniently. `Effect::LyricOffset`: `App` saves it and re-syncs the line.
- **Lookup.** YouTube-style decoration ("(Official Video)", "- Topic",
  "VEVO", "- Remastered") is cut by `clean_title`/`clean_artist` before the
  lrclib `/api/get`; on a miss `/api/search` (no length) picks the result
  whose length is closest within `SEARCH_DURATION_MS` (`pick_search`).
  The negative cache extension is now `.none2` so misses recorded by the old,
  weaker lookup are retried once.

The timeline is cleared on a track change so the old position never leaks
into the new song.

**Not verified live:** real YouTube Music / Spotify GSMTC positions. Owner:
play a song with synced lyrics, check the line against the audio, adjust
with ± if a player reports its position consistently early or late.

Related: [[lyric-notch-width]].
