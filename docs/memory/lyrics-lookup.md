---
tags: [decision, gotcha, lyrics, lrclib]
date: 2026-10-31
---

# Lyrics lookup: retry, steps, consensus pick

Builds on [[lyrics-sync]] (which describes the first search fallback and the
`.none2` marker; both are superseded here).

## LRCLIB is often busy

Measured live 2026-10-31: `/api/get` and `/api/search` both answer **503**
`{"name":"ServerOverloaded"}` with `retry-after: 1` fairly often, and the same
request 1-2 s later works. The old `lookup` read a 503 as `Failed` and only
ran the cleaned-title step and the search on a definitive "none", so one 503
skipped every fallback.

- `Http::get_with_headers_capped_retry` returns `(status, body,
  Option<Retry-After>)` (`get_with_headers_capped` calls it).
- Every request goes through one helper: a 429 or any 5xx waits
  `retry_delay` (the header's whole seconds, clamped to 1..=3 s; missing,
  unparsable or an HTTP date = 1 s) and asks the same URL again, at most 2
  retries. A network error or timeout is asked once more. After that the URL
  is **transient**.
- `LOOKUP_BUDGET` is 30 s from the start of the network part, checked before
  every request and every sleep; past it nothing more is sent and what is left
  counts as transient.

## Steps (`lookup_chain`)

Each runs until one yields an accepted synced candidate; a failed or empty
step never stops the next. The chain is written against a `Net` trait (get,
sleep, now) so the tests script it with a fake clock.

1. `/api/search?track_name&artist_name` with the cleaned names.
2. `/api/get` with the raw title, artist and length, then with the cleaned
   names if they differ.
3. `/api/search?q=<artist> <title>` (free text, cleaned names), then the same
   with the title's trailing "(...)" / "[...]" group cut. Covers: "Red
   (Taylor Swift)" by Against The Current is found only by `q=Against The
   Current Red`. A URL already asked is skipped.

## Outcome rules

- An accepted candidate: `Answer::Synced` (cached by `keep_answer` as before).
- Nothing found and **every** request answered definitively (200, 404, other
  4xx such as 400): `Answer::None`, the 24 h negative marker (a provisional
  query still does not write it).
- Nothing found and any request transient (429/5xx after retries, an error, an
  unreadable or over-size 200, or the budget ran out): `Answer::Failed`, the
  existing retry timer, never a marker. A transient step must not be
  remembered as "no lyrics for a day".

## "Artist - Title" split

Plain YouTube videos look like title `Avril Lavigne - When You're Gone
(Official Video)`, artist `AvrilLavigneVEVO`. `split_artist_title` splits the
cleaned title on its first " - " when the first part equals the cleaned artist
(case-insensitive, letters and digits only: "Avril Lavigne" == "AvrilLavigne")
or the raw artist ends with "VEVO" or " - Topic". The pair is then the
"cleaned names" of steps 1, 2 (second get) and 3. Titles such as "Red (Taylor
Swift)" and "Some Song - Live" are not split.

## Best synced version: consensus

LRCLIB entries are user uploads and their timing varies; copies of one
timing agree with each other. `pick_best` keeps the candidates within
`SEARCH_DURATION_MS` (3 s) of the playing length (a missing length counts as
0 off; at most 20), and with several it matches lines by normalised text (the
k-th occurrence with the k-th, first 300 lines, at least 3 shared lines per
pair), takes the median shift per pair, and scores each candidate by the
median of its absolute shifts to the others. Lowest wins; ties go to the
closest length, then the first. A shifted upload loses even when its length is
the closest. One candidate is returned as is. Applied per step's candidate set.

## none3

`NONE_EXT` is now `none3`: markers written by the older, weaker lookups
(`.none`, `.none2`) are ignored, so those tracks are asked again once. `.lrc`
files stay valid.
