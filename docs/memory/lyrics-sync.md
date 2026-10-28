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

## 3.2.1: still too fast

3.2.0 chose the line at position **+150 ms** (`LOOKAHEAD_MS`) so the crossfade
ended as the word was sung; with the player's position already ahead of what
the ear hears (output buffers, Bluetooth 150-300 ms) that read as "too fast".
Now `lyric_position` = position + offset − `OUTPUT_LAG_MS` (100): at offset 0 a
line is never chosen before its time (test
`at_the_default_offset_a_line_never_shows_before_its_time`). The offset range
is ±5 s (was ±2 s). The next-line timer now waits wall-clock time at the
playback rate (`wait_ms(pos, rate)`) and is not armed while paused (the next
refresh re-arms it). The sign is unchanged: the offset is added to the
position, so **+ shows lines sooner and − later**; if lines still come early,
press − on the Music page.

The timeline is cleared on a track change so the old position never leaks
into the new song.

## 3.2.1 review fixes

- **Timer that never ends.** `position_at` stops at `end_ms`, so a line past
  `lyric_reach(end_ms, offset)` (the end with the offset and lag applied; a
  large negative offset pulls it back) never comes. `wait_ms(pos, rate,
  reach)` returns `None` for it, which also covers "the position hit the
  end": the timer used to fire and re-arm until the next track.
- **NaN rate.** `clamp` keeps a NaN: `wait_ms` gave 1 ms (a 1 kHz timer) and
  `position_at` froze. `usable_rate`: finite → clamp 0.1..4, else 1.0.
- **Lookup timeout.** `Http::with_timeout` (lyrics: `REQUEST_TIMEOUT` 10 s)
  waits on `SendRequestAsync` with a deadline (`Http::send_request`, via
  `when` + `recv_timeout`) and `Cancel`s it; the call fails with
  `ERROR_TIMEOUT` → `Answer::Failed`. The default completion option buffers
  the whole body, so the deadline covers a stalled body too.
- **Retry.** A transient answer (failure, or a provisional "not found") for
  the track last asked clears `asked` and holds the track; `found` returns
  true and `App` arms `TIMER_LYRIC_RETRY` (31, one-shot `RETRY_AFTER` 30 s),
  whose `retry_due` lets the next `want` ask again. At most `RETRIES` (3) per
  track; asking another track resets the count.

**Not verified live:** real YouTube Music / Spotify GSMTC positions. Owner:
play a song with synced lyrics, check the line against the audio, adjust
with ± if a player reports its position consistently early or late.

Related: [[lyric-notch-width]].
