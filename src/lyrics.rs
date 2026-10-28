//! Synced lyrics for the notch: LRC parsing, the line at a playback position,
//! and lookups at LRCLIB on a worker thread, with a disk cache (results and
//! 24 h "not found" markers) behind `App`'s in-memory LRU.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::google::http::{Http, form_encode};
use crate::model::{Event, MediaInfo, Timeline};

/// Tracks remembered in memory (with or without lyrics).
pub const MEMORY_SLOTS: usize = 50;
/// A "not found" answer is trusted this long before asking again.
pub const NEGATIVE_TTL_SECS: i64 = 24 * 3600;
/// A fresh reading closer than this to where playback was predicted is the
/// same playback: the media watcher sends no new timeline for it. Anything
/// farther is sent, so a reading that is more accurate than the prediction
/// replaces it (a bias is never kept; a seek is just a large one).
pub const RESYNC_MS: u64 = 100;
/// A line shows this long after its time, never before: the player's position
/// is where it *plays*, and the sound reaches the ear later (output buffers,
/// Bluetooth 150-300 ms), so a line at exactly its time looks early. 3.2.0
/// went 150 ms *early* instead, which read as "too fast".
pub const OUTPUT_LAG_MS: i64 = 100;
/// "Offset lirik" in ms: (min, max, step). The offset is added to the
/// position: positive shows each line sooner, negative later (the Music
/// page's "-" is "later").
pub const OFFSET_RANGE: (i32, i32, i32) = (-5_000, 5_000, 100);
/// Crossfade between two lyric lines.
pub const FADE: Duration = Duration::from_millis(150);
/// The worker waits this long for a newer query before looking one up, so a
/// track change reported in pieces (title first, timeline later) is looked
/// up once, with the new length.
pub const SETTLE: Duration = Duration::from_secs(1);
/// A lookup that failed on the way (no network, a stalled or refused
/// request) is asked again this long after, up to `RETRIES` times per track.
pub const RETRY_AFTER: Duration = Duration::from_secs(30);
pub const RETRIES: u32 = 3;
/// One request to LRCLIB that has not answered by then has failed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// A length change this large under the same title means the next track's
/// timeline arrived before its title.
const LENGTH_JUMP_MS: u64 = 2_000;
const USER_AGENT: &str = concat!(
    "Vale/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/hartono0297/dynamic-island)"
);
const API: &str = "https://lrclib.net/api/get";
const SEARCH_API: &str = "https://lrclib.net/api/search";
/// Not-found markers use this extension: ones written before the search
/// fallback and the title clean-up (`.none`) are ignored, so a track they
/// gave up on is asked again.
const NONE_EXT: &str = "none2";
/// A search result is the track only when its length is this close.
const SEARCH_DURATION_MS: u64 = 3_000;
/// LRCLIB answers and cached `.lrc` files larger than this are refused
/// unparsed: a song's synced lyrics are a few KB.
pub const MAX_BODY_BYTES: usize = 512 * 1024;
/// `parse_lrc` keeps at most this many time tags of one line (more are
/// skipped), this many characters of its text, and this many lines.
const MAX_TAGS_PER_LINE: usize = 16;
const MAX_LINE_CHARS: usize = 500;
const MAX_LINES: usize = 5_000;

/// One timed line; an empty `text` is an instrumental gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LrcLine {
    pub at_ms: u64,
    pub text: String,
}

pub type Lines = Arc<Vec<LrcLine>>;

/// What the worker found for `key`. `lines: None` is "no synced lyrics";
/// `transient` marks a network failure, which is not remembered.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricsFound {
    pub key: String,
    pub lines: Option<Lines>,
    pub transient: bool,
}

/// A track to look up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub key: String,
    pub artist: String,
    pub title: String,
    pub duration_ms: u64,
    /// The length may still be the previous track's (no timeline reported
    /// since the title changed): a "not found" for it is not remembered.
    pub provisional: bool,
}

/// Parses LRC text: every `[mm:ss.xx]` tag on a line times a copy of its text,
/// `[offset:±ms]` shifts all lines (positive = earlier), metadata tags and
/// lines without a time tag are skipped. The result is sorted by time
/// (stable: equal times keep their order). Untrusted text stays bounded: a
/// line keeps its first `MAX_TAGS_PER_LINE` times and `MAX_LINE_CHARS`
/// characters, and at most `MAX_LINES` lines come out.
pub fn parse_lrc(text: &str) -> Vec<LrcLine> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut offset: i64 = 0;
    let mut lines: Vec<(u64, String)> = Vec::new();
    for raw in text.lines() {
        if lines.len() >= MAX_LINES {
            break;
        }
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(tag_end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            let tag = &rest[1..=tag_end];
            if let Some(ms) = parse_time(tag) {
                if times.len() < MAX_TAGS_PER_LINE {
                    times.push(ms);
                }
            } else if let Some(value) = tag_value(tag, "offset") {
                if let Ok(v) = value.trim().parse::<i64>() {
                    offset = v;
                }
                break;
            } else {
                // Metadata ([ar:…], [ti:…], [length:…]) or junk.
                break;
            }
            rest = rest[tag_end + 2..].trim_start();
        }
        let text = rest.trim();
        let text = match text.char_indices().nth(MAX_LINE_CHARS) {
            Some((cut, _)) => text[..cut].trim_end(),
            None => text,
        };
        let room = MAX_LINES - lines.len();
        lines.extend(times.into_iter().take(room).map(|at_ms| (at_ms, text.to_owned())));
    }
    let mut out: Vec<LrcLine> = lines
        .into_iter()
        .map(|(at, text)| LrcLine {
            at_ms: u64::try_from(i64::try_from(at).unwrap_or(i64::MAX).saturating_sub(offset))
                .unwrap_or(0),
            text,
        })
        .collect();
    out.sort_by_key(|l| l.at_ms);
    out
}

/// `mm:ss`, `mm:ss.f`, `mm:ss.ff`, `mm:ss.fff` or `mm:ss:ff` in milliseconds.
fn parse_time(tag: &str) -> Option<u64> {
    let (min, sec) = tag.split_once(':')?;
    if min.is_empty() || !min.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (whole, frac) = match sec.split_once(['.', ':']) {
        Some((w, f)) => (w, f),
        None => (sec, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || whole.len() > 2 || (!frac.is_empty() && (!digits(frac) || frac.len() > 3))
    {
        return None;
    }
    let secs: u64 = whole.parse().ok()?;
    if secs >= 60 {
        return None;
    }
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac.parse::<u64>().ok()?,
    };
    min.parse::<u64>()
        .ok()?
        .checked_mul(60_000)?
        .checked_add(secs * 1000 + frac_ms)
}

/// The value of a `name:value` tag, the name compared without case.
fn tag_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let (n, v) = tag.split_once(':')?;
    n.trim().eq_ignore_ascii_case(name).then_some(v)
}

/// The line showing at `pos_ms`: the last one whose time has come. `None`
/// before the first line.
pub fn current_line(lines: &[LrcLine], pos_ms: u64) -> Option<(usize, &str)> {
    let i = lines.partition_point(|l| l.at_ms <= pos_ms);
    let i = i.checked_sub(1)?;
    Some((i, lines[i].text.as_str()))
}

/// When the shown line next changes after `pos_ms`; `None` after the last.
pub fn next_change(lines: &[LrcLine], pos_ms: u64) -> Option<u64> {
    let i = lines.partition_point(|l| l.at_ms <= pos_ms);
    lines.get(i).map(|l| l.at_ms)
}

/// Whether the lyrics have any words (not only instrumental gaps).
pub fn has_words(lines: &[LrcLine]) -> bool {
    lines.iter().any(|l| !l.text.is_empty())
}

/// The playback position at `now`: frozen while paused, advancing at the
/// reported rate while playing, never past the end when the end is known.
pub fn position_at(t: &Timeline, playing: bool, now: Instant) -> u64 {
    let pos = if playing {
        let rate = usable_rate(t.rate);
        let elapsed = now.saturating_duration_since(t.updated).as_secs_f64() * 1000.0 * rate;
        t.position_ms.saturating_add(elapsed as u64)
    } else {
        t.position_ms
    };
    if t.end_ms > 0 { pos.min(t.end_ms) } else { pos }
}

/// A playback rate to count with: kept in 0.1..=4, and 1 for one that is
/// not a number or infinite (`clamp` keeps a NaN, which would make the
/// next-line timer fire every millisecond).
fn usable_rate(rate: f32) -> f64 {
    let rate = f64::from(rate);
    if rate.is_finite() { rate.clamp(0.1, 4.0) } else { 1.0 }
}

/// The farthest lyric position this track can reach: `position_at` stops at
/// its end, so with the offset applied (`lyric_position`) a line after this
/// never comes. `None` when the player reports no length.
pub fn lyric_reach(end_ms: u64, offset_ms: i32) -> Option<u64> {
    (end_ms > 0).then(|| lyric_position(end_ms, offset_ms))
}

/// The position the lyric line is chosen at: the playback position plus the
/// user's offset (`lyrics_offset_ms`) minus `OUTPUT_LAG_MS`, never before 0.
/// At offset 0 a line is therefore never chosen before its time.
pub fn lyric_position(raw_ms: u64, offset_ms: i32) -> u64 {
    let shifted = i64::try_from(raw_ms)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(offset_ms))
        .saturating_sub(OUTPUT_LAG_MS);
    u64::try_from(shifted).unwrap_or(0)
}

/// The offset one step (`dir` −1 or +1) on, kept in `OFFSET_RANGE`; a
/// hand-edited value off the grid snaps onto it.
pub fn step_offset(offset_ms: i32, dir: i32) -> i32 {
    let (min, max, step) = OFFSET_RANGE;
    let on_grid = offset_ms.div_euclid(step) * step;
    let stepped = if dir > 0 {
        on_grid + step
    } else if offset_ms == on_grid {
        on_grid - step
    } else {
        on_grid
    };
    stepped.clamp(min, max)
}

/// The same moment re-expressed at `now`: the position then, as of now. Used
/// when play/pause flips, so a player that does not refresh its timeline
/// neither runs on while paused nor jumps when it resumes.
pub fn rebase(t: &Timeline, was_playing: bool, now: Instant) -> Timeline {
    Timeline {
        position_ms: position_at(t, was_playing, now),
        end_ms: t.end_ms,
        updated: now,
        rate: t.rate,
    }
}

/// Whether a freshly read timeline differs from the one last sent: one of
/// them missing, another length or rate, or a position more than `RESYNC_MS`
/// away from where the last one says playback should be.
pub fn timeline_moved(
    last: Option<&Timeline>,
    next: Option<&Timeline>,
    playing: bool,
    now: Instant,
) -> bool {
    match (last, next) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            a.end_ms != b.end_ms
                || a.rate != b.rate
                || position_at(a, playing, now).abs_diff(position_at(b, playing, now))
                    > RESYNC_MS
        }
        _ => true,
    }
}

/// Lower case, blanks collapsed to one space, trimmed.
fn normalise(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The cache key of a track: normalised artist and title, and its length in
/// whole seconds (rounded).
pub fn track_key(artist: &str, title: &str, duration_ms: u64) -> String {
    let secs = duration_ms.saturating_add(500) / 1000;
    format!("{}|{}|{secs}", normalise(artist), normalise(title))
}

/// The lookup for what is playing: needs a title, an artist and a known
/// length (without a timeline nothing can be synced anyway).
pub fn track_query(media: Option<&MediaInfo>, timeline: Option<&Timeline>) -> Option<Query> {
    let m = media?;
    let t = timeline?;
    let (artist, title) = (m.artist.trim(), m.title.trim());
    if artist.is_empty() || title.is_empty() || t.end_ms == 0 {
        return None;
    }
    Some(Query {
        key: track_key(artist, title, t.end_ms),
        artist: artist.to_owned(),
        title: title.to_owned(),
        duration_ms: t.end_ms,
        provisional: false,
    })
}

/// FNV-1a, 64 bit, as 16 hex digits: a stable file name for a key.
pub fn key_hash(key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The LRCLIB lookup URL. The album is not sent: players report it
/// unreliably, and the length (±2 s) already pins the recording.
pub fn lrclib_url(q: &Query) -> String {
    lrclib_url_for(&q.title, &q.artist, q.duration_ms)
}

/// A title as a streaming site decorates it, without the decoration:
/// "(Official Video)", "[Lyrics]", "(Audio)", a trailing "- Remastered"
/// and the like. Returns `title` itself when nothing is to cut.
pub fn clean_title(title: &str) -> String {
    const NOISE: &[&str] = &[
        "official", "video", "music video", "lyric", "lyrics", "audio", "hd", "hq", "4k", "mv",
        "visualizer", "visualiser", "remaster", "remastered", "live", "version",
    ];
    let mut out = title.trim().to_owned();
    loop {
        let before = out.clone();
        for (open, close) in [('(', ')'), ('[', ']')] {
            if out.ends_with(close)
                && let Some(start) = out.rfind(open)
            {
                let inner = out[start + 1..out.len() - 1].to_lowercase();
                if NOISE.iter().any(|n| inner.contains(n)) {
                    out = out[..start].trim_end().to_owned();
                }
            }
        }
        if let Some((head, tail)) = out.rsplit_once(" - ")
            && NOISE.iter().any(|n| tail.to_lowercase().contains(n))
        {
            out = head.trim_end().to_owned();
        }
        if out == before {
            break;
        }
    }
    if out.is_empty() { title.trim().to_owned() } else { out }
}

/// An artist without YouTube Music's " - Topic" suffix and a trailing
/// "VEVO".
pub fn clean_artist(artist: &str) -> String {
    let a = artist.trim();
    let a = a.strip_suffix(" - Topic").unwrap_or(a).trim_end();
    let lower = a.to_lowercase();
    let a = if lower.ends_with("vevo") && a.len() > 4 {
        a[..a.len() - 4].trim_end()
    } else {
        a
    };
    if a.is_empty() { artist.trim().to_owned() } else { a.to_owned() }
}

/// The LRCLIB lookup URL for an explicit title and artist.
fn lrclib_url_for(title: &str, artist: &str, duration_ms: u64) -> String {
    let secs = (duration_ms as f64 / 1000.0).clamp(1.0, 3600.0);
    let duration = format!("{}", secs.round() as u64);
    let query = form_encode(&[
        ("track_name", title),
        ("artist_name", artist),
        ("duration", &duration),
    ]);
    format!("{API}?{query}")
}

/// The search fallback's URL: no length (a different version of the song has
/// another one); `pick_search` chooses by it.
pub fn lrclib_search_url(title: &str, artist: &str) -> String {
    let query = form_encode(&[("track_name", title), ("artist_name", artist)]);
    format!("{SEARCH_API}?{query}")
}

/// The synced lyrics of the search result whose length is closest to
/// `duration_ms`, within `SEARCH_DURATION_MS`; `None` when there is none or
/// the body is over `MAX_BODY_BYTES`.
pub fn pick_search(body: &str, duration_ms: u64) -> Option<String> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json.as_array()?
        .iter()
        .filter_map(|item| {
            let lrc = item.get("syncedLyrics")?.as_str()?;
            if parse_lrc(lrc).is_empty() {
                return None;
            }
            let secs = item.get("duration")?.as_f64()?;
            let ms = (secs * 1000.0).round() as u64;
            let gap = ms.abs_diff(duration_ms);
            (gap <= SEARCH_DURATION_MS).then(|| (gap, lrc.to_owned()))
        })
        .min_by_key(|(gap, _)| *gap)
        .map(|(_, lrc)| lrc)
}

/// What an LRCLIB answer means.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// The `syncedLyrics` LRC text.
    Synced(String),
    /// Known not to have synced lyrics (404, instrumental, plain only).
    None,
    /// Server or network trouble: ask again another time.
    Failed,
}

/// What a `/api/get` reply says. A body over `MAX_BODY_BYTES` is not read:
/// it is a failure, asked again another time.
pub fn read_answer(status: u16, body: &str) -> Answer {
    match status {
        200 if body.len() > MAX_BODY_BYTES => Answer::Failed,
        200 => {
            let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
                return Answer::Failed;
            };
            match json.get("syncedLyrics").and_then(|v| v.as_str()) {
                Some(lrc) if !parse_lrc(lrc).is_empty() => Answer::Synced(lrc.to_owned()),
                _ => Answer::None,
            }
        }
        404 => Answer::None,
        _ => Answer::Failed,
    }
}

/// The last `MEMORY_SLOTS` tracks looked up, most recent first, with the
/// Unix time each was learnt. A "not found" expires like the disk marker.
pub struct MemoryCache {
    cap: usize,
    entries: VecDeque<(String, Option<Lines>, i64)>,
}

impl MemoryCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            entries: VecDeque::new(),
        }
    }

    /// `Some(None)`: known to have no synced lyrics (for under
    /// `NEGATIVE_TTL_SECS`). Moves the entry to the front.
    pub fn get(&mut self, key: &str, now_unix: i64) -> Option<Option<Lines>> {
        let i = self.entries.iter().position(|(k, _, _)| k == key)?;
        let entry = self.entries.remove(i)?;
        if entry.1.is_none() && !(0..NEGATIVE_TTL_SECS).contains(&(now_unix - entry.2)) {
            return None;
        }
        let lines = entry.1.clone();
        self.entries.push_front(entry);
        Some(lines)
    }

    pub fn put(&mut self, key: String, lines: Option<Lines>, now_unix: i64) {
        self.entries.retain(|(k, _, _)| *k != key);
        self.entries.push_front((key, lines, now_unix));
        self.entries.truncate(self.cap);
    }

    #[cfg(test)]
    fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|(k, _, _)| k.as_str()).collect()
    }
}

/// What the disk cache knows of a key.
#[derive(Debug, PartialEq)]
pub enum Cached {
    Lrc(String),
    /// A fresh "not found" marker, written at this Unix time.
    Nothing(i64),
    Miss,
}

/// `<dir>\<hash>.lrc` holds a track's LRC; `<hash>.none2` holds the Unix time
/// a lookup found nothing, trusted for `NEGATIVE_TTL_SECS`.
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `%APPDATA%\Vale\lyrics-cache`.
    pub fn default_dir() -> PathBuf {
        crate::data_dir::dir().join("lyrics-cache")
    }

    fn file(&self, key: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", key_hash(key)))
    }

    /// An `.lrc` over `MAX_BODY_BYTES` (never written by `write_lrc`) is
    /// left unread, as if it were not there.
    pub fn read(&self, key: &str, now_unix: i64) -> Cached {
        let lrc = self.file(key, "lrc");
        let fits = std::fs::metadata(&lrc).is_ok_and(|m| m.len() <= MAX_BODY_BYTES as u64);
        if fits && let Ok(lrc) = std::fs::read_to_string(&lrc) {
            return Cached::Lrc(lrc);
        }
        let marker = self.file(key, NONE_EXT);
        match std::fs::read_to_string(&marker) {
            Ok(stamp) => match stamp.trim().parse::<i64>() {
                Ok(at) if (0..NEGATIVE_TTL_SECS).contains(&(now_unix - at)) => Cached::Nothing(at),
                _ => {
                    let _ = std::fs::remove_file(&marker);
                    Cached::Miss
                }
            },
            Err(_) => Cached::Miss,
        }
    }

    /// Write errors only cost a lookup next time; they are ignored. LRC
    /// that `storable` refuses is not written.
    pub fn write_lrc(&self, key: &str, lrc: &str) {
        if !storable(lrc) {
            return;
        }
        let _ = write_atomic(&self.file(key, "lrc"), lrc);
        let _ = std::fs::remove_file(self.file(key, NONE_EXT));
    }

    pub fn write_nothing(&self, key: &str, now_unix: i64) {
        let _ = write_atomic(&self.file(key, NONE_EXT), &now_unix.to_string());
    }
}

/// LRC fit to cache and show: within `MAX_BODY_BYTES` and with at least one
/// timed line.
fn storable(lrc: &str) -> bool {
    lrc.len() <= MAX_BODY_BYTES && !parse_lrc(lrc).is_empty()
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The island's side of the lyrics: the track asked for, the lines of the
/// current track, and the line on screen. Caches live in the worker.
#[derive(Default)]
pub struct LyricsState {
    /// The key last sent to the worker; not asked again while it is current.
    /// A lookup that failed on the way clears it, so it is asked again once
    /// `retry_due` says the back-off is over.
    asked: Option<String>,
    /// The track whose failed lookup waits for `retry_due`.
    held: Option<String>,
    /// The track retried last and how many retries it has had.
    retries: Option<(String, u32)>,
    /// The key of the playing track as last seen; only its answer is shown.
    wanted: Option<String>,
    /// Artist and title (normalised) and the length last seen with them.
    seen: Option<(String, u64)>,
    /// A length that may belong to another track than the title shown with
    /// it; cleared once a different length is reported.
    stale_end: Option<u64>,
    current: Option<(String, Lines)>,
    shown: Option<Shown>,
}

/// The line on screen, and the one fading out since `since`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    line: Option<usize>,
    prev: Option<Option<usize>>,
    since: Instant,
}

/// A lyric line to draw: the text (empty = instrumental), the one fading
/// out, and the crossfade's progress (1 = settled).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownLine<'a> {
    pub text: &'a str,
    pub prev: Option<&'a str>,
    pub fade: f32,
}

impl LyricsState {
    /// Notes `q` (the playing track) and returns the lookup to send to the
    /// worker when it has not been asked yet and `fetch` allows it. The
    /// worker settles for `SETTLE` before answering, so a title reported
    /// before its timeline is superseded by the query with the right length.
    pub fn want(&mut self, q: Option<&Query>, fetch: bool) -> Option<Query> {
        let q = q?;
        let provisional = self.observe(q);
        self.wanted = Some(q.key.clone());
        if self.current.as_ref().is_some_and(|(k, _)| *k == q.key) {
            return None;
        }
        if fetch
            && self.asked.as_deref() != Some(q.key.as_str())
            && self.held.as_deref() != Some(q.key.as_str())
        {
            if self.retries.as_ref().is_some_and(|(key, _)| *key != q.key) {
                self.retries = None;
            }
            self.asked = Some(q.key.clone());
            return Some(Query {
                provisional,
                ..q.clone()
            });
        }
        None
    }

    /// Whether `q`'s length may still be another track's: the title changed
    /// but the length is the one seen before it (title reported first), or
    /// the length jumped under the same title (timeline reported first).
    /// Either way it holds until a different length is reported.
    fn observe(&mut self, q: &Query) -> bool {
        let identity = q.key.rsplit_once('|').map_or(q.key.as_str(), |(id, _)| id);
        let end = q.duration_ms;
        match &self.seen {
            Some((prev, prev_end)) if prev != identity => {
                self.stale_end = (*prev_end == end).then_some(end);
            }
            Some((_, prev_end)) => match self.stale_end {
                Some(stale) if stale != end => self.stale_end = None,
                Some(_) => {}
                None if prev_end.abs_diff(end) > LENGTH_JUMP_MS => self.stale_end = Some(end),
                None => {}
            },
            None => {}
        }
        self.seen = Some((identity.to_owned(), end));
        self.stale_end.is_some()
    }

    /// A worker answer; it becomes the current lyrics only if it is for the
    /// track playing now. A failure on the way (`transient`) for the track
    /// last asked lets it be asked again, up to `RETRIES` times: true when
    /// the caller should call `retry_due` after `RETRY_AFTER`.
    pub fn found(&mut self, f: LyricsFound) -> bool {
        if f.transient {
            if self.asked.as_deref() != Some(f.key.as_str()) {
                return false;
            }
            let tries = match &self.retries {
                Some((key, n)) if *key == f.key => *n,
                _ => 0,
            };
            if tries >= RETRIES {
                return false;
            }
            self.asked = None;
            self.held = Some(f.key.clone());
            self.retries = Some((f.key, tries + 1));
            return true;
        }
        if let Some(lines) = f.lines
            && self.wanted.as_deref() == Some(f.key.as_str())
        {
            self.set_current(f.key, lines);
        }
        false
    }

    /// The back-off after a failed lookup is over: the track may be asked
    /// again (the next `want` while it plays).
    pub fn retry_due(&mut self) {
        self.held = None;
    }

    fn set_current(&mut self, key: String, lines: Lines) {
        self.current = Some((key, lines));
        self.shown = None;
    }

    /// Synced lyrics with words exist for `q`.
    pub fn ready(&self, q: Option<&Query>) -> bool {
        match (q, &self.current) {
            (Some(q), Some((key, lines))) => *key == q.key && has_words(lines),
            _ => false,
        }
    }

    fn lines(&self) -> &[LrcLine] {
        self.current.as_ref().map_or(&[], |(_, l)| l.as_slice())
    }

    /// Moves the shown line to the one at `pos_ms`. True when the text on
    /// screen changes; it then crossfades from the old one, except for the
    /// first line shown. A line repeating the same text changes nothing.
    pub fn advance(&mut self, pos_ms: u64, now: Instant) -> bool {
        let line = current_line(self.lines(), pos_ms).map(|(i, _)| i);
        let Some(shown) = self.shown else {
            self.shown = Some(Shown {
                line,
                prev: None,
                since: now,
            });
            return true;
        };
        if shown.line == line {
            return false;
        }
        if self.text(shown.line) == self.text(line) {
            self.shown = Some(Shown { line, ..shown });
            return false;
        }
        self.shown = Some(Shown {
            line,
            prev: Some(shown.line),
            since: now,
        });
        true
    }

    fn text(&self, line: Option<usize>) -> &str {
        line.and_then(|i| self.lines().get(i))
            .map_or("", |l| l.text.as_str())
    }

    /// The lyric view went away: the next one starts without a crossfade.
    pub fn hide(&mut self) {
        self.shown = None;
    }

    /// A crossfade is still running at `now`.
    pub fn fading(&self, now: Instant) -> bool {
        self.shown
            .is_some_and(|s| s.prev.is_some() && now.saturating_duration_since(s.since) < FADE)
    }

    /// What to draw at `now`; `None` before `advance` placed a line.
    pub fn shown(&self, now: Instant) -> Option<ShownLine<'_>> {
        let s = self.shown?;
        let fade = match s.prev {
            Some(_) => {
                (now.saturating_duration_since(s.since).as_secs_f32() / FADE.as_secs_f32()).min(1.0)
            }
            None => 1.0,
        };
        Some(ShownLine {
            text: self.text(s.line),
            prev: s.prev.filter(|_| fade < 1.0).map(|p| self.text(p)),
            fade,
        })
    }

    /// Milliseconds of wall clock from `pos_ms` until the line next changes
    /// at playback `rate` (a faster song gets there sooner), plus 1 so the
    /// change has happened when a timer fires. `None` after the last line,
    /// and when the next line lies past `reach_ms` (`lyric_reach`): the
    /// position stops at the track's end, so a timer for it would fire and
    /// re-arm for ever.
    pub fn wait_ms(&self, pos_ms: u64, rate: f32, reach_ms: Option<u64>) -> Option<u64> {
        let rate = usable_rate(rate);
        next_change(self.lines(), pos_ms)
            .filter(|&at| reach_ms.is_none_or(|reach| at <= reach))
            .map(|at| (((at - pos_ms) as f64 / rate).ceil() as u64).saturating_add(1))
    }
}

/// Sends lookups to the lyrics thread.
pub struct LyricsWorker {
    queries: Sender<Query>,
}

impl LyricsWorker {
    /// False once the thread has gone.
    pub fn ask(&self, q: Query) -> bool {
        self.queries.send(q).is_ok()
    }
}

/// Starts the lyrics thread: it answers each query from its memory LRU, the
/// disk cache or LRCLIB with an `Event::Lyrics`, then `wake`s the island. A
/// query is looked up only once no newer one came for `SETTLE`; older ones
/// are dropped unanswered. It blocks between queries.
pub fn spawn_worker(
    tx: Sender<Event>,
    wake: impl Fn() + Send + 'static,
    cache: DiskCache,
) -> LyricsWorker {
    let (queries, rx) = mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut http = None;
        let mut memory = MemoryCache::new(MEMORY_SLOTS);
        while let Some(q) = settled(&rx, SETTLE) {
            let found = lookup(&q, &cache, &mut memory, &mut http);
            if tx.send(Event::Lyrics(found)).is_err() {
                return;
            }
            wake();
        }
    });
    LyricsWorker { queries }
}

/// Blocks for a query, then waits until none newer came for `settle`;
/// returns the newest.
fn settled(rx: &Receiver<Query>, settle: Duration) -> Option<Query> {
    let mut q = rx.recv().ok()?;
    while let Ok(newer) = rx.recv_timeout(settle) {
        q = newer;
    }
    Some(q)
}

fn found(q: &Query, lines: Option<Lines>, transient: bool) -> LyricsFound {
    LyricsFound {
        key: q.key.clone(),
        lines,
        transient,
    }
}

fn lookup(
    q: &Query,
    cache: &DiskCache,
    memory: &mut MemoryCache,
    http: &mut Option<Http>,
) -> LyricsFound {
    let now = chrono::Local::now().timestamp();
    if let Some(known) = remembered(q, cache, memory, now) {
        return known;
    }
    if http.is_none() {
        // A request that stalls fails (and is retried later) instead of
        // holding the thread and every lookup behind it.
        *http = Http::new().ok().map(|h| h.with_timeout(REQUEST_TIMEOUT));
    }
    let Some(client) = http.as_ref() else {
        return found(q, None, true);
    };
    let get = |url: &str| {
        client.get_with_headers_capped(url, &[("User-Agent", USER_AGENT)], MAX_BODY_BYTES)
    };
    let ask = |url: &str| match get(url) {
        Ok((status, body)) => read_answer(status, &body),
        Err(_) => Answer::Failed,
    };
    // The track as the player names it first; then as a streaming site
    // decorates it ("(Official Video)", " - Topic"); then a search, picking
    // the result whose length is closest (a video's length differs from the
    // album's by seconds, which `/api/get` does not forgive).
    let mut answer = ask(&lrclib_url(q));
    let (title, artist) = (clean_title(&q.title), clean_artist(&q.artist));
    if answer == Answer::None && (title != q.title || artist != q.artist) {
        answer = ask(&lrclib_url_for(&title, &artist, q.duration_ms));
    }
    if answer == Answer::None {
        answer = match get(&lrclib_search_url(&title, &artist)) {
            Ok((200, body)) => pick_search(&body, q.duration_ms).map_or(Answer::None, Answer::Synced),
            Ok((404, _)) => Answer::None,
            _ => Answer::Failed,
        };
    }
    keep_answer(q, answer, cache, memory, now)
}

/// The answer from memory, else from disk (then remembered in memory).
fn remembered(
    q: &Query,
    cache: &DiskCache,
    memory: &mut MemoryCache,
    now: i64,
) -> Option<LyricsFound> {
    if let Some(lines) = memory.get(&q.key, now) {
        return Some(found(q, lines, false));
    }
    let (lines, at) = match cache.read(&q.key, now) {
        Cached::Lrc(lrc) => (Some(Arc::new(parse_lrc(&lrc))), now),
        Cached::Nothing(at) => (None, at),
        Cached::Miss => return None,
    };
    memory.put(q.key.clone(), lines.clone(), at);
    Some(found(q, lines, false))
}

/// Caches what LRCLIB said. A "not found" for a provisional query is not
/// kept anywhere: its length may have been the previous track's.
fn keep_answer(
    q: &Query,
    answer: Answer,
    cache: &DiskCache,
    memory: &mut MemoryCache,
    now: i64,
) -> LyricsFound {
    match answer {
        // Never anything else: `read_answer` and `pick_search` check it.
        Answer::Synced(lrc) if !storable(&lrc) => found(q, None, true),
        Answer::Synced(lrc) => {
            cache.write_lrc(&q.key, &lrc);
            let lines = Some(Arc::new(parse_lrc(&lrc)));
            memory.put(q.key.clone(), lines.clone(), now);
            found(q, lines, false)
        }
        Answer::None if q.provisional => found(q, None, true),
        Answer::None => {
            cache.write_nothing(&q.key, now);
            memory.put(q.key.clone(), None, now);
            found(q, None, false)
        }
        Answer::Failed => found(q, None, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn l(at_ms: u64, text: &str) -> LrcLine {
        LrcLine {
            at_ms,
            text: text.into(),
        }
    }

    #[test]
    fn parses_plain_timed_lines() {
        let lrc = "[00:12.34] Hello there\n[01:02.5]Second\r\n[02:03.456] Third  \n[03:04] Fourth";
        assert_eq!(
            parse_lrc(lrc),
            vec![
                l(12_340, "Hello there"),
                l(62_500, "Second"),
                l(123_456, "Third"),
                l(184_000, "Fourth"),
            ]
        );
    }

    #[test]
    fn several_timestamps_on_one_line_repeat_it() {
        let lrc = "[00:30.00][00:10.00] Chorus\n[00:20.00] Verse";
        assert_eq!(
            parse_lrc(lrc),
            vec![l(10_000, "Chorus"), l(20_000, "Verse"), l(30_000, "Chorus")]
        );
    }

    #[test]
    fn offset_tag_shifts_every_line() {
        // Positive: lyrics come earlier; never before 0.
        let lrc = "[offset:+500]\n[00:00.20] a\n[00:10.00] b";
        assert_eq!(parse_lrc(lrc), vec![l(0, "a"), l(9_500, "b")]);
        let lrc = "[00:10.00] b\n[offset:-250]";
        assert_eq!(parse_lrc(lrc), vec![l(10_250, "b")]);
        // An unreadable offset is ignored.
        assert_eq!(
            parse_lrc("[offset:soon]\n[00:01.00] x"),
            vec![l(1_000, "x")]
        );
    }

    #[test]
    fn metadata_tags_are_ignored() {
        let lrc = "\u{feff}[ar:Demo]\n[ti:Jalan Pulang]\n[length: 05:55]\n[by:someone]\n[00:01.00] Lampu kota menyala";
        assert_eq!(parse_lrc(lrc), vec![l(1_000, "Lampu kota menyala")]);
    }

    #[test]
    fn blank_timed_lines_are_instrumental_gaps() {
        let lrc = "[00:01.00] Words\n[00:05.00]\n[00:09.00]   \n\n[00:12.00] More";
        let lines = parse_lrc(lrc);
        assert_eq!(
            lines,
            vec![
                l(1_000, "Words"),
                l(5_000, ""),
                l(9_000, ""),
                l(12_000, "More")
            ]
        );
        assert!(has_words(&lines));
        assert!(!has_words(&parse_lrc("[00:01.00]\n[00:02.00]")));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let lrc = "no tag here\n[xx:yy] bad\n[00:61.00] bad seconds\n[1:02.3456] too precise\n[00:03.00 unclosed\n[] empty\n[00:04.00] good";
        assert_eq!(parse_lrc(lrc), vec![l(4_000, "good")]);
        assert!(parse_lrc("").is_empty());
    }

    #[test]
    fn output_is_sorted_and_stable() {
        let lrc = "[00:09.00] c\n[00:01.00] a\n[00:05.00] b1\n[00:05.00] b2";
        assert_eq!(
            parse_lrc(lrc),
            vec![l(1_000, "a"), l(5_000, "b1"), l(5_000, "b2"), l(9_000, "c")]
        );
    }

    #[test]
    fn extreme_values_never_overflow() {
        // Minutes too many for u64, minutes whose total overflows by the
        // seconds, and offsets at both ends of i64.
        let lrc = "[99999999999999999999:00.00] a\n[307445734561825:59.999] b\n[00:01.00] c";
        assert_eq!(parse_lrc(lrc), vec![l(1_000, "c")]);
        let big = "[307445734561825:00.00] far\n[00:02.00] near";
        // Past i64::MAX ms (292 million years) a time is clamped there.
        assert_eq!(parse_lrc(big)[1].at_ms, i64::MAX as u64);
        let earlier = "[offset:9223372036854775807]\n[00:02.00] x\n[307445734561825:00.00] y";
        assert!(parse_lrc(earlier).iter().all(|l| l.at_ms == 0));
        let later = "[offset:-9223372036854775808]\n[00:02.00] x\n[307445734561825:00.00] y";
        assert!(parse_lrc(later).iter().all(|l| l.at_ms == i64::MAX as u64));
    }

    #[test]
    fn long_minutes_and_colon_fractions() {
        assert_eq!(parse_lrc("[123:04:50] x"), vec![l(7_384_500, "x")]);
    }

    #[test]
    fn parsing_untrusted_lrc_stays_bounded() {
        // 100k tags on one line: only the first few become lines, the rest
        // are skipped without a copy each, and it stays quick.
        let flood = format!("{} x", "[00:01.00]".repeat(100_000));
        let start = Instant::now();
        let lines = parse_lrc(&flood);
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!(lines.len(), MAX_TAGS_PER_LINE);
        assert!(lines.iter().all(|line| line.text == "x"));
        // A long text is cut by characters, not bytes.
        let long = format!("[00:01.00] {}", "é".repeat(2_000));
        assert_eq!(parse_lrc(&long)[0].text.chars().count(), MAX_LINE_CHARS);
        // At most `MAX_LINES` lines, however they come.
        let many: String = (0..MAX_LINES + 100)
            .map(|i| format!("[00:{:02}.00] l{i}\n", i % 60))
            .collect();
        assert_eq!(parse_lrc(&many).len(), MAX_LINES);
        let tagged = format!("{} x\n", "[00:01.00]".repeat(MAX_TAGS_PER_LINE)).repeat(MAX_LINES);
        assert_eq!(parse_lrc(&tagged).len(), MAX_LINES);
    }

    #[test]
    fn oversized_bodies_are_refused_unparsed() {
        let pad = "x".repeat(MAX_BODY_BYTES);
        let big = format!(r#"{{"syncedLyrics":"[00:01.00] hi","pad":"{pad}"}}"#);
        assert_eq!(read_answer(200, &big), Answer::Failed);
        let list = format!(r#"[{{"duration":180.0,"syncedLyrics":"[00:01.00] hi","pad":"{pad}"}}]"#);
        assert_eq!(pick_search(&list, 180_000), None);
        assert!(storable("[00:01.00] hi"));
        assert!(!storable(&format!("[00:01.00] {pad}")));
        assert!(!storable("no tags"));
    }

    fn song() -> Vec<LrcLine> {
        vec![
            l(1_000, "one"),
            l(5_000, ""),
            l(8_000, "three"),
            l(8_000, "three b"),
        ]
    }

    #[test]
    fn current_line_follows_position() {
        let s = song();
        assert_eq!(current_line(&s, 0), None, "before the first line");
        assert_eq!(current_line(&s, 999), None);
        assert_eq!(current_line(&s, 1_000), Some((0, "one")));
        assert_eq!(current_line(&s, 4_999), Some((0, "one")));
        assert_eq!(current_line(&s, 5_000), Some((1, "")));
        // Equal times: the later line wins.
        assert_eq!(current_line(&s, 8_000), Some((3, "three b")));
        assert_eq!(current_line(&s, 999_999), Some((3, "three b")));
        assert_eq!(current_line(&[], 5), None);
    }

    #[test]
    fn current_line_after_seeking_backwards() {
        let s = song();
        assert_eq!(current_line(&s, 9_000), Some((3, "three b")));
        assert_eq!(current_line(&s, 2_000), Some((0, "one")));
        assert_eq!(next_change(&s, 2_000), Some(5_000));
        assert_eq!(current_line(&s, 500), None);
        assert_eq!(next_change(&s, 500), Some(1_000));
    }

    #[test]
    fn next_change_is_the_next_distinct_time() {
        let s = song();
        assert_eq!(next_change(&s, 0), Some(1_000));
        assert_eq!(next_change(&s, 1_000), Some(5_000));
        assert_eq!(next_change(&s, 5_500), Some(8_000));
        assert_eq!(next_change(&s, 8_000), None, "after the last line");
        assert_eq!(next_change(&[], 0), None);
    }

    fn tl(position_ms: u64, end_ms: u64, updated: Instant) -> Timeline {
        Timeline {
            position_ms,
            end_ms,
            updated,
            rate: 1.0,
        }
    }

    #[test]
    fn position_interpolates_while_playing_only() {
        let t0 = Instant::now();
        let t = tl(10_000, 200_000, t0);
        let later = t0 + Duration::from_millis(2_500);
        assert_eq!(position_at(&t, true, later), 12_500);
        assert_eq!(position_at(&t, false, later), 10_000);
        // Never past the end; an unknown end does not clamp.
        let far = t0 + Duration::from_secs(1_000);
        assert_eq!(position_at(&t, true, far), 200_000);
        assert_eq!(position_at(&tl(10_000, 0, t0), true, far), 1_010_000);
        // A `now` before `updated` (clock skew) does not go backwards.
        assert_eq!(position_at(&tl(10_000, 0, later), true, t0), 10_000);
    }

    #[test]
    fn rebase_freezes_a_pause_and_resumes_from_it() {
        let t0 = Instant::now();
        let t = tl(10_000, 200_000, t0);
        let paused_at = t0 + Duration::from_secs(3);
        let paused = rebase(&t, true, paused_at);
        assert_eq!(paused, tl(13_000, 200_000, paused_at));
        // Paused for a minute: no time passes.
        let resumed_at = paused_at + Duration::from_secs(60);
        let resumed = rebase(&paused, false, resumed_at);
        assert_eq!(resumed, tl(13_000, 200_000, resumed_at));
        assert_eq!(
            position_at(&resumed, true, resumed_at + Duration::from_secs(2)),
            15_000
        );
    }

    #[test]
    fn timeline_moved_ignores_steady_playback_but_sees_seeks() {
        let t0 = Instant::now();
        let now = t0 + Duration::from_secs(5);
        let last = tl(10_000, 200_000, t0);
        // The player re-reported where playback is, within the tolerance:
        // not a move. Past it (a stutter, a drift) is one.
        let same = tl(15_080, 200_000, now);
        assert!(!timeline_moved(Some(&last), Some(&same), true, now));
        assert!(timeline_moved(
            Some(&last),
            Some(&tl(15_200, 200_000, now)),
            true,
            now
        ));
        // Another playback rate is a move on its own.
        let faster = Timeline {
            rate: 1.25,
            ..tl(15_000, 200_000, now)
        };
        assert!(timeline_moved(Some(&last), Some(&faster), true, now));
        // A seek back, a seek forward, a new length.
        assert!(timeline_moved(
            Some(&last),
            Some(&tl(2_000, 200_000, now)),
            true,
            now
        ));
        assert!(timeline_moved(
            Some(&last),
            Some(&tl(60_000, 200_000, now)),
            true,
            now
        ));
        assert!(timeline_moved(
            Some(&last),
            Some(&tl(15_000, 180_000, now)),
            true,
            now
        ));
        // Paused: positions compare as they are.
        assert!(timeline_moved(Some(&last), Some(&same), false, now));
        assert!(!timeline_moved(
            Some(&last),
            Some(&tl(10_060, 200_000, now)),
            false,
            now
        ));
        assert!(timeline_moved(None, Some(&last), true, now));
        assert!(timeline_moved(Some(&last), None, true, now));
        assert!(!timeline_moved(None, None, true, now));
    }

    #[test]
    fn the_lyric_position_adds_offset_minus_the_output_lag_and_never_goes_below_zero() {
        let lag = OUTPUT_LAG_MS as u64;
        assert_eq!(lyric_position(10_000, 0), 10_000 - lag);
        assert_eq!(lyric_position(10_000, -300), 10_000 - 300 - lag);
        assert_eq!(lyric_position(10_000, 500), 10_000 + 500 - lag);
        assert_eq!(lyric_position(0, -5_000), 0);
        assert_eq!(lyric_position(50, 0), 0);
        assert_eq!(lyric_position(u64::MAX, 5_000), i64::MAX as u64 - lag);
    }

    #[test]
    fn at_the_default_offset_a_line_never_shows_before_its_time() {
        let lines = vec![l(1_000, "one"), l(5_000, "two"), l(9_000, "three")];
        // Walking the player's position in small steps: the line chosen is
        // never one whose time has not come, and each shows `OUTPUT_LAG_MS`
        // after its time.
        let mut raw = 0;
        let mut first_seen = [None; 3];
        while raw < 12_000 {
            let chosen = current_line(&lines, lyric_position(raw, 0)).map(|(i, _)| i);
            if let Some(i) = chosen {
                assert!(raw >= lines[i].at_ms, "line {i} early at {raw}");
                first_seen[i].get_or_insert(raw);
            }
            raw += 7;
        }
        for (i, seen) in first_seen.iter().enumerate() {
            let seen = seen.expect("every line shows");
            let due = lines[i].at_ms + OUTPUT_LAG_MS as u64;
            assert!((due..due + 7).contains(&seen), "line {i}: {seen} vs {due}");
        }
    }

    #[test]
    fn the_timer_waits_wall_clock_time_at_the_playback_rate() {
        let mut st = LyricsState::default();
        let a = query("a");
        st.want(Some(&a), true);
        st.found(found(&a.key, Some(vec![l(1_000, "a"), l(5_000, "b")]), false));
        assert_eq!(st.wait_ms(2_000, 1.0, None), Some(3_001));
        assert_eq!(st.wait_ms(2_000, 2.0, None), Some(1_501), "twice as fast");
        assert_eq!(st.wait_ms(2_000, 0.5, None), Some(6_001), "half speed");
        assert_eq!(st.wait_ms(6_000, 1.0, None), None, "after the last line");
        // An odd rate never makes it spin or sleep for ever: a rate that is
        // not a number (or infinite) counts as 1 (a NaN used to give 1 ms).
        assert!(st.wait_ms(2_000, 0.0, None).is_some_and(|ms| ms <= 30_002));
        assert_eq!(st.wait_ms(2_000, f32::NAN, None), Some(3_001));
        assert_eq!(st.wait_ms(2_000, f32::INFINITY, None), Some(3_001));
        assert_eq!(st.wait_ms(2_000, f32::NEG_INFINITY, None), Some(3_001));
    }

    #[test]
    fn a_rate_that_is_not_a_number_plays_at_normal_speed() {
        let t0 = Instant::now();
        let mut t = tl(1_000, 0, t0);
        t.rate = f32::NAN;
        let later = t0 + Duration::from_millis(500);
        assert_eq!(position_at(&t, true, later), 1_500, "it used to freeze");
        t.rate = f32::INFINITY;
        assert_eq!(position_at(&t, true, later), 1_500);
    }

    #[test]
    fn the_timer_is_not_armed_for_a_line_the_track_end_cuts_off() {
        let mut st = LyricsState::default();
        let a = query("a");
        st.want(Some(&a), true);
        // The last line sits past the reported end (a long outro tag, or a
        // length the player rounds down).
        st.found(found(
            &a.key,
            Some(vec![l(1_000, "a"), l(5_000, "b"), l(9_500, "c")]),
            false,
        ));
        let reach = lyric_reach(9_000, 0);
        assert_eq!(reach, Some(8_900), "the end minus the output lag");
        assert_eq!(st.wait_ms(2_000, 1.0, reach), Some(3_001), "b is reachable");
        assert_eq!(st.wait_ms(6_000, 1.0, reach), None, "c never comes");
        // Stopped at the end: position_at holds end_ms, nothing is armed
        // (it used to re-arm every few hundred ms until the next track).
        let t0 = Instant::now();
        let t = tl(8_990, 9_000, t0);
        let pos = lyric_position(position_at(&t, true, t0 + Duration::from_secs(5)), 0);
        assert_eq!(pos, 8_900);
        assert_eq!(st.wait_ms(pos, 1.0, reach), None);
        // A large negative offset moves the reach back with the lines.
        let reach = lyric_reach(9_000, -5_000);
        assert_eq!(reach, Some(3_900));
        assert_eq!(st.wait_ms(2_000, 1.0, reach), None, "b now lies past it");
        assert_eq!(st.wait_ms(500, 1.0, reach), Some(501));
        // No reported length: nothing to cut off.
        assert_eq!(lyric_reach(0, 0), None);
        assert_eq!(st.wait_ms(6_000, 1.0, None), Some(3_501));
    }

    #[test]
    fn the_offset_steps_by_a_tenth_within_its_range_and_snaps_onto_the_grid() {
        assert_eq!(step_offset(0, 1), 100);
        assert_eq!(step_offset(0, -1), -100);
        assert_eq!(step_offset(-100, 1), 0);
        assert_eq!(step_offset(5_000, 1), 5_000);
        assert_eq!(step_offset(-5_000, -1), -5_000);
        // Hand-edited, off the grid: the next step lands on it.
        assert_eq!(step_offset(130, 1), 200);
        assert_eq!(step_offset(130, -1), 100);
        assert_eq!(step_offset(-130, 1), -100);
        assert_eq!(step_offset(-130, -1), -200);
    }

    #[test]
    fn streaming_decoration_is_cut_from_titles_and_artists() {
        assert_eq!(clean_title("Blinding Lights (Official Video)"), "Blinding Lights");
        assert_eq!(clean_title("Song [Lyrics] (Audio)"), "Song");
        assert_eq!(clean_title("Yesterday - Remastered 2009"), "Yesterday");
        // A real part of the name stays.
        assert_eq!(clean_title("Song (feat. Someone)"), "Song (feat. Someone)");
        assert_eq!(clean_title("Plain Title"), "Plain Title");
        // Never empties the title.
        assert_eq!(clean_title("(Official Video)"), "(Official Video)");
        assert_eq!(clean_artist("The Weeknd - Topic"), "The Weeknd");
        assert_eq!(clean_artist("ArianaGrandeVEVO"), "ArianaGrande");
        assert_eq!(clean_artist("Queen"), "Queen");
        assert_eq!(clean_artist("VEVO"), "VEVO");
    }

    #[test]
    fn the_search_fallback_picks_the_closest_length_with_synced_lyrics() {
        let body = r#"[
            {"duration": 200.0, "syncedLyrics": null},
            {"duration": 190.0, "syncedLyrics": "[00:01.00] far"},
            {"duration": 181.0, "syncedLyrics": "[00:01.00] near"},
            {"duration": 181.5, "syncedLyrics": "not a lrc"}
        ]"#;
        assert_eq!(pick_search(body, 180_000).as_deref(), Some("[00:01.00] near"));
        // Nothing within the tolerance, a non-list, junk: none.
        assert_eq!(pick_search(body, 400_000), None);
        assert_eq!(pick_search("{}", 180_000), None);
        assert_eq!(pick_search("junk", 180_000), None);
        let url = lrclib_search_url("Hello World", "A&B");
        assert!(url.starts_with(SEARCH_API) && !url.contains("duration"), "{url}");
    }

    #[test]
    fn track_key_is_normalised() {
        assert_eq!(
            track_key("  Queen ", "Bohemian   RHAPSODY", 354_499),
            "queen|bohemian rhapsody|354"
        );
        assert_eq!(
            track_key("queen", "bohemian rhapsody", 354_500),
            "queen|bohemian rhapsody|355"
        );
        assert_ne!(key_hash("a|b|1"), key_hash("a|b|2"));
        assert_eq!(key_hash("a|b|1").len(), 16);
        assert_eq!(key_hash(""), "cbf29ce484222325");
    }

    fn media(title: &str, artist: &str) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: artist.into(),
            playing: true,
            art: None,
            source_app: String::new(),
        }
    }

    #[test]
    fn query_needs_title_artist_and_length() {
        let t = tl(0, 215_000, Instant::now());
        let q = track_query(Some(&media("Song", "Band")), Some(&t)).unwrap();
        assert_eq!(
            q,
            Query {
                key: "band|song|215".into(),
                artist: "Band".into(),
                title: "Song".into(),
                duration_ms: 215_000,
                provisional: false,
            }
        );
        assert_eq!(track_query(Some(&media("Song", " ")), Some(&t)), None);
        assert_eq!(track_query(Some(&media("", "Band")), Some(&t)), None);
        assert_eq!(track_query(Some(&media("Song", "Band")), None), None);
        let no_end = tl(0, 0, Instant::now());
        assert_eq!(
            track_query(Some(&media("Song", "Band")), Some(&no_end)),
            None
        );
        assert_eq!(track_query(None, Some(&t)), None);
    }

    #[test]
    fn url_carries_title_artist_and_seconds() {
        let q = Query {
            key: String::new(),
            artist: "Sigur Rós".into(),
            title: "Hoppípolla & more".into(),
            duration_ms: 268_400,
            provisional: false,
        };
        assert_eq!(
            lrclib_url(&q),
            "https://lrclib.net/api/get?track_name=Hopp%C3%ADpolla%20%26%20more&artist_name=Sigur%20R%C3%B3s&duration=268"
        );
        let short = Query {
            duration_ms: 200,
            ..q
        };
        assert!(lrclib_url(&short).ends_with("&duration=1"));
        assert!(USER_AGENT.starts_with("Vale/"));
    }

    #[test]
    fn answers_map_to_found_none_or_failed() {
        let ok = r#"{"id":1,"trackName":"t","artistName":"a","albumName":"x","duration":200,"instrumental":false,"plainLyrics":"hi","syncedLyrics":"[00:01.00] hi"}"#;
        assert_eq!(read_answer(200, ok), Answer::Synced("[00:01.00] hi".into()));
        let plain_only = r#"{"id":1,"plainLyrics":"hi","syncedLyrics":null}"#;
        assert_eq!(read_answer(200, plain_only), Answer::None);
        let instrumental = r#"{"id":1,"instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#;
        assert_eq!(read_answer(200, instrumental), Answer::None);
        assert_eq!(
            read_answer(200, r#"{"syncedLyrics":"no tags"}"#),
            Answer::None
        );
        assert_eq!(read_answer(404, r#"{"code":404}"#), Answer::None);
        assert_eq!(read_answer(200, "<html>"), Answer::Failed);
        assert_eq!(read_answer(500, ""), Answer::Failed);
        assert_eq!(read_answer(429, ""), Answer::Failed);
    }

    #[test]
    fn memory_cache_keeps_the_most_recent() {
        let now = 1_800_000_000;
        let mut c = MemoryCache::new(3);
        let lines = Arc::new(vec![l(1, "x")]);
        c.put("a".into(), Some(lines.clone()), now);
        c.put("b".into(), None, now);
        c.put("c".into(), None, now);
        assert_eq!(c.get("a", now), Some(Some(lines.clone())));
        assert_eq!(c.get("b", now), Some(None), "a known miss");
        assert_eq!(c.get("zzz", now), None, "never asked");
        // "c" is now the least recently used.
        c.put("d".into(), None, now);
        assert_eq!(c.keys(), ["d", "b", "a"]);
        c.put("a".into(), None, now);
        assert_eq!(c.keys(), ["a", "d", "b"], "re-put replaces");
        assert_eq!(c.get("a", now), Some(None));
        assert_eq!(MemoryCache::new(MEMORY_SLOTS).cap, 50);
    }

    #[test]
    fn memory_negatives_expire_but_lyrics_do_not() {
        let now = 1_800_000_000;
        let mut c = MemoryCache::new(5);
        c.put("none".into(), None, now);
        c.put("lyrics".into(), Some(Arc::new(vec![l(1, "x")])), now);
        let day = now + NEGATIVE_TTL_SECS;
        assert_eq!(c.get("none", day - 1), Some(None));
        assert_eq!(c.get("none", day), None, "expired");
        assert_eq!(c.keys(), ["lyrics"], "and dropped");
        assert!(matches!(c.get("lyrics", day * 2), Some(Some(_))));
        // A stamp from the future (clock moved back) is not trusted.
        c.put("skew".into(), None, now + 100);
        assert_eq!(c.get("skew", now), None);
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("vale-lyrics-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn disk_cache_round_trip_and_negative_ttl() {
        let tmp = TempDir::new("disk");
        let cache = DiskCache::new(tmp.0.join("lyrics-cache"));
        let now = 1_800_000_000;
        assert_eq!(cache.read("k", now), Cached::Miss, "missing dir is a miss");
        cache.write_lrc("k", "[00:01.00] hi");
        assert_eq!(cache.read("k", now), Cached::Lrc("[00:01.00] hi".into()));
        let file = tmp
            .0
            .join("lyrics-cache")
            .join(format!("{}.lrc", key_hash("k")));
        assert!(file.exists());

        cache.write_nothing("gone", now);
        let marker = tmp
            .0
            .join("lyrics-cache")
            .join(format!("{}.{NONE_EXT}", key_hash("gone")));
        assert!(marker.exists(), "the marker is where the test looks");
        assert_eq!(cache.read("gone", now), Cached::Nothing(now));
        assert_eq!(
            cache.read("gone", now + NEGATIVE_TTL_SECS - 1),
            Cached::Nothing(now)
        );
        assert!(marker.exists());
        // Expired: a miss, and the marker is removed.
        assert_eq!(cache.read("gone", now + NEGATIVE_TTL_SECS), Cached::Miss);
        assert!(!marker.exists());
        // A marker from the future (clock moved back) is not trusted.
        cache.write_nothing("skew", now + 100);
        assert_eq!(cache.read("skew", now), Cached::Miss);
        // Found lyrics replace an earlier "not found".
        cache.write_nothing("later", now);
        cache.write_lrc("later", "[00:02.00] yes");
        assert_eq!(
            cache.read("later", now),
            Cached::Lrc("[00:02.00] yes".into())
        );
    }

    #[test]
    fn oversized_lrc_is_neither_written_nor_read() {
        let tmp = TempDir::new("oversized");
        let cache = DiskCache::new(tmp.0.clone());
        let now = 1_800_000_000;
        let huge = format!("[00:01.00] {}", "x".repeat(MAX_BODY_BYTES));
        let q = query("big");
        let file = tmp.0.join(format!("{}.lrc", key_hash(&q.key)));
        let mut memory = MemoryCache::new(5);
        let f = keep_answer(&q, Answer::Synced(huge.clone()), &cache, &mut memory, now);
        assert!(f.transient && f.lines.is_none());
        cache.write_lrc(&q.key, &huge);
        assert!(!file.exists(), "never written");
        // One put there by hand is skipped.
        std::fs::create_dir_all(&tmp.0).unwrap();
        std::fs::write(&file, &huge).unwrap();
        assert_eq!(cache.read(&q.key, now), Cached::Miss);
        std::fs::write(&file, "[00:01.00] hi").unwrap();
        assert_eq!(cache.read(&q.key, now), Cached::Lrc("[00:01.00] hi".into()));
    }

    #[test]
    fn worker_answers_from_disk_cache() {
        let tmp = TempDir::new("worker");
        let cache = DiskCache::new(tmp.0.clone());
        cache.write_lrc("band|song|200", "[00:01.00] one\n[00:02.00] two");
        let (tx, rx) = mpsc::channel();
        let worker = spawn_worker(tx, || {}, DiskCache::new(tmp.0.clone()));
        assert!(worker.ask(Query {
            key: "band|song|200".into(),
            artist: "Band".into(),
            title: "Song".into(),
            duration_ms: 200_000,
            provisional: false,
        }));
        let event = rx.recv_timeout(SETTLE + Duration::from_secs(5)).unwrap();
        assert_eq!(
            event,
            Event::Lyrics(LyricsFound {
                key: "band|song|200".into(),
                lines: Some(Arc::new(vec![l(1_000, "one"), l(2_000, "two")])),
                transient: false,
            })
        );
    }

    #[test]
    fn queries_settle_to_the_newest() {
        let (tx, rx) = mpsc::channel();
        for key in ["a", "b", "c"] {
            tx.send(query(key)).unwrap();
        }
        // One more arrives while it settles: it wins too.
        let late = tx.clone();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            late.send(query("d")).unwrap();
        });
        let t0 = Instant::now();
        assert_eq!(
            settled(&rx, Duration::from_millis(150)).unwrap().key,
            query("d").key
        );
        assert!(
            t0.elapsed() >= Duration::from_millis(150),
            "waited to settle"
        );
        sender.join().unwrap();
        drop(tx);
        assert!(settled(&rx, Duration::from_millis(10)).is_none());
        assert_eq!(SETTLE, Duration::from_secs(1));
    }

    #[test]
    fn worker_remembers_and_skips_provisional_misses() {
        let tmp = TempDir::new("keep");
        let cache = DiskCache::new(tmp.0.clone());
        let mut memory = MemoryCache::new(MEMORY_SLOTS);
        let now = 1_800_000_000;
        let shaky = Query {
            provisional: true,
            ..query("b")
        };
        // A provisional "not found" is kept nowhere and counts as transient.
        let f = keep_answer(&shaky, Answer::None, &cache, &mut memory, now);
        assert!(f.transient && f.lines.is_none());
        assert_eq!(cache.read(&shaky.key, now), Cached::Miss);
        assert_eq!(remembered(&shaky, &cache, &mut memory, now), None);
        // A settled one is kept on disk and in memory.
        let c = query("c");
        let f = keep_answer(&c, Answer::None, &cache, &mut memory, now);
        assert!(!f.transient);
        assert_eq!(cache.read(&c.key, now), Cached::Nothing(now));
        assert_eq!(memory.get(&c.key, now), Some(None));
        // Lyrics found for a provisional query are fine to keep.
        let f = keep_answer(
            &shaky,
            Answer::Synced("[00:01.00] x".into()),
            &cache,
            &mut memory,
            now,
        );
        assert_eq!(f.lines, Some(Arc::new(vec![l(1_000, "x")])));
        // From disk into memory, keeping the marker's age.
        let mut fresh = MemoryCache::new(MEMORY_SLOTS);
        let later = now + 10;
        let f = remembered(&c, &cache, &mut fresh, later).unwrap();
        assert!(f.lines.is_none() && !f.transient);
        assert_eq!(fresh.get(&c.key, now + NEGATIVE_TTL_SECS), None, "same TTL");
        // Transient failures are never kept.
        let d = query("d");
        let f = keep_answer(&d, Answer::Failed, &cache, &mut memory, now);
        assert!(f.transient);
        assert_eq!(memory.get(&d.key, now), None);
    }

    /// A track keyed like `track_key`: `key` is artist|title, length 200 s.
    fn query(key: &str) -> Query {
        query_len(key, 200_000)
    }

    fn query_len(id: &str, duration_ms: u64) -> Query {
        Query {
            key: format!("{id}|{}", duration_ms / 1000),
            artist: "Band".into(),
            title: id.into(),
            duration_ms,
            provisional: false,
        }
    }

    fn found(key: &str, lines: Option<Vec<LrcLine>>, transient: bool) -> LyricsFound {
        LyricsFound {
            key: key.into(),
            lines: lines.map(Arc::new),
            transient,
        }
    }

    #[test]
    fn state_asks_each_track_once() {
        let mut st = LyricsState::default();
        let a = query("a");
        assert_eq!(st.want(Some(&a), false), None, "paused: no network");
        assert_eq!(st.want(Some(&a), true), Some(a.clone()));
        assert_eq!(st.want(Some(&a), true), None, "asked already");
        assert!(!st.ready(Some(&a)));
        st.found(found(&a.key, Some(song()), false));
        assert!(st.ready(Some(&a)));
        assert_eq!(st.want(Some(&a), true), None, "current already");
        // Another track, then back: each is asked again (the worker's
        // memory answers the second time).
        let b = query_len("b", 245_000);
        assert_eq!(st.want(Some(&b), true), Some(b.clone()));
        st.found(found(&b.key, None, false));
        assert!(!st.ready(Some(&b)));
        assert_eq!(st.want(Some(&b), true), None);
        assert_eq!(
            st.want(Some(&a), true),
            None,
            "its lyrics are still current"
        );
        assert!(st.ready(Some(&a)));
        assert!(!st.ready(None));
        assert_eq!(st.want(None, true), None);
    }

    #[test]
    fn a_failure_is_retried_after_a_back_off_a_few_times() {
        let mut st = LyricsState::default();
        let a = query("a");
        assert!(st.want(Some(&a), true).is_some());
        for _ in 0..RETRIES {
            // Failed on the way: the caller arms the back-off.
            assert!(st.found(found(&a.key, None, true)));
            assert_eq!(st.want(Some(&a), true), None, "not before the back-off");
            st.retry_due();
            assert_eq!(st.want(Some(&a), false), None, "paused: still no network");
            assert_eq!(st.want(Some(&a), true), Some(a.clone()), "asked again");
        }
        // Out of retries: kept until another track was asked.
        assert!(!st.found(found(&a.key, None, true)));
        assert_eq!(st.want(Some(&a), true), None);
        let b = query_len("b", 245_000);
        assert!(st.want(Some(&b), true).is_some());
        st.found(found(&b.key, Some(song()), false));
        assert_eq!(st.want(Some(&a), true), Some(a.clone()));
        // And then it has its retries again.
        assert!(st.found(found(&a.key, None, true)));
        // A late failure for a track no longer asked arms nothing.
        assert!(!st.found(found(&b.key, None, true)));
        // A real "not found" is no failure.
        let c = query("c");
        st.retry_due();
        assert!(st.want(Some(&c), true).is_some());
        assert!(!st.found(found(&c.key, None, false)));
        assert_eq!(st.want(Some(&c), true), None);
    }

    #[test]
    fn only_the_playing_tracks_answer_is_taken() {
        let mut st = LyricsState::default();
        let (a, b) = (query("a"), query("b"));
        st.want(Some(&a), true);
        st.want(Some(&b), true);
        // A late answer for "a" does not replace anything.
        st.found(found(&a.key, Some(song()), false));
        assert!(!st.ready(Some(&a)) && !st.ready(Some(&b)));
        st.found(found(&b.key, Some(vec![l(0, "bee")]), false));
        assert!(st.ready(Some(&b)));
        st.found(found(&a.key, Some(song()), false));
        assert!(st.ready(Some(&b)), "b stays");
        // Wanted while paused (not asked): its answer is still taken.
        let c = query("c");
        assert_eq!(st.want(Some(&c), false), None);
        st.found(found(&c.key, Some(song()), false));
        assert!(st.ready(Some(&c)));
    }

    #[test]
    fn title_before_timeline_waits_for_the_new_length() {
        let mut st = LyricsState::default();
        let a = query_len("a", 200_000);
        st.want(Some(&a), true);
        // "b" shows up with "a"'s length: provisional.
        let early = query_len("b", 200_000);
        let asked = st.want(Some(&early), true).unwrap();
        assert!(asked.provisional);
        // Its own timeline arrives: asked again, settled.
        let b = query_len("b", 245_000);
        let asked = st.want(Some(&b), true).unwrap();
        assert!(!asked.provisional);
        // A seek or a pause (same length) changes nothing.
        assert_eq!(st.want(Some(&b), true), None);
        assert!(!st.observe(&b));
        // The provisional answer is not shown for the settled track.
        st.found(found(&early.key, Some(song()), false));
        assert!(!st.ready(Some(&b)));
    }

    #[test]
    fn timeline_before_title_is_provisional_until_a_new_length() {
        let mut st = LyricsState::default();
        let a = query_len("a", 200_000);
        assert!(!st.want(Some(&a), true).unwrap().provisional);
        // "b"'s length arrives under "a"'s title.
        let mixed = query_len("a", 245_000);
        assert!(st.want(Some(&mixed), true).unwrap().provisional);
        // Then "b"'s title, with the same length: still provisional, so a
        // "not found" is not remembered, but lyrics found are shown.
        let b = query_len("b", 245_000);
        let asked = st.want(Some(&b), true).unwrap();
        assert!(asked.provisional);
        st.found(found(&b.key, Some(song()), false));
        assert!(st.ready(Some(&b)));
        // The next track reported whole is settled again.
        let c = query_len("c", 180_000);
        assert!(!st.want(Some(&c), true).unwrap().provisional);
    }

    #[test]
    fn title_and_timeline_together_are_settled() {
        let mut st = LyricsState::default();
        let a = query_len("a", 200_000);
        assert!(!st.want(Some(&a), true).unwrap().provisional, "first track");
        let b = query_len("b", 245_000);
        assert!(!st.want(Some(&b), true).unwrap().provisional);
        // A small length correction under the same title is not a jump.
        let b2 = query_len("b", 246_000);
        assert!(!st.want(Some(&b2), true).unwrap().provisional);
    }

    #[test]
    fn lyrics_without_words_are_not_ready() {
        let mut st = LyricsState::default();
        let a = query("a");
        st.want(Some(&a), true);
        st.found(found(&a.key, Some(vec![l(1_000, ""), l(2_000, "")]), false));
        assert!(!st.ready(Some(&a)));
    }

    fn playing_state() -> LyricsState {
        let mut st = LyricsState::default();
        let a = query("a");
        st.want(Some(&a), true);
        st.found(found(&a.key, Some(song()), false));
        st
    }

    #[test]
    fn advance_crossfades_between_lines() {
        let t0 = Instant::now();
        let mut st = playing_state();
        assert_eq!(st.shown(t0), None);
        // Before the first line: the rest sign, shown at once.
        assert!(st.advance(0, t0));
        assert_eq!(
            st.shown(t0),
            Some(ShownLine {
                text: "",
                prev: None,
                fade: 1.0
            })
        );
        assert!(!st.fading(t0));
        assert_eq!(st.wait_ms(0, 1.0, None), Some(1_001));
        assert!(!st.advance(500, t0), "same line");
        let t1 = t0 + Duration::from_millis(1_000);
        assert!(st.advance(1_000, t1));
        assert!(st.fading(t1));
        let mid = st.shown(t1 + FADE / 2).unwrap();
        assert_eq!((mid.text, mid.prev), ("one", Some("")));
        assert!((mid.fade - 0.5).abs() < 0.01);
        let done = st.shown(t1 + FADE).unwrap();
        assert_eq!((done.text, done.prev, done.fade), ("one", None, 1.0));
        assert!(!st.fading(t1 + FADE));
        // Seeking backwards fades back.
        let t2 = t1 + Duration::from_secs(1);
        assert!(st.advance(200, t2));
        assert_eq!(st.shown(t2).unwrap().prev, Some("one"));
        assert_eq!(st.wait_ms(200, 1.0, None), Some(801));
        assert_eq!(st.wait_ms(9_000, 1.0, None), None, "after the last line");
    }

    #[test]
    fn same_text_moves_without_a_fade_and_hide_resets() {
        let t0 = Instant::now();
        let mut st = LyricsState::default();
        let a = query("a");
        st.want(Some(&a), true);
        let lines = vec![l(1_000, "la"), l(2_000, "la"), l(3_000, "")];
        st.found(found(&a.key, Some(lines), false));
        st.advance(1_500, t0);
        assert!(!st.advance(2_500, t0), "repeated text: nothing to redraw");
        assert_eq!(st.wait_ms(2_500, 1.0, None), Some(501));
        assert!(st.advance(3_000, t0));
        st.hide();
        assert_eq!(st.shown(t0), None);
        assert!(st.advance(3_000, t0));
        assert_eq!(st.shown(t0).unwrap().prev, None, "no fade after hide");
    }

    #[test]
    fn new_lyrics_reset_the_shown_line() {
        let t0 = Instant::now();
        let mut st = playing_state();
        st.advance(9_000, t0);
        let b = query("b");
        st.want(Some(&b), true);
        st.found(found(&b.key, Some(vec![l(0, "bee")]), false));
        assert_eq!(st.shown(t0), None);
        st.advance(0, t0);
        assert_eq!(st.shown(t0).unwrap().text, "bee");
    }

    /// One real request (run by hand: `cargo test lrclib_live -- --ignored`).
    #[test]
    #[ignore]
    fn lrclib_live() {
        let q = Query {
            key: track_key("Queen", "Bohemian Rhapsody", 354_000),
            artist: "Queen".into(),
            title: "Bohemian Rhapsody".into(),
            duration_ms: 354_000,
            provisional: false,
        };
        let http = Http::new().unwrap();
        let (status, body) = http
            .get_with_headers_capped(&lrclib_url(&q), &[("User-Agent", USER_AGENT)], MAX_BODY_BYTES)
            .unwrap();
        println!("status {status}, {} bytes", body.len());
        let Answer::Synced(lrc) = read_answer(status, &body) else {
            panic!("no synced lyrics: status {status}");
        };
        let lines = parse_lrc(&lrc);
        // Timing only: the lyrics themselves are not printed.
        println!(
            "{} lines; first at {:?} ms",
            lines.len(),
            lines.first().map(|l| l.at_ms)
        );
        assert!(lines.len() > 20 && has_words(&lines));
        assert!(lines.windows(2).all(|w| w[0].at_ms <= w[1].at_ms));
        assert!(current_line(&lines, 60_000).is_some());
    }
}
