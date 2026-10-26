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
/// Positions closer than this to the predicted one are the same playback,
/// not a seek: the media watcher sends no new timeline for them.
pub const SEEK_TOLERANCE_MS: u64 = 750;
/// Crossfade between two lyric lines.
pub const FADE: Duration = Duration::from_millis(150);
/// The worker waits this long for a newer query before looking one up, so a
/// track change reported in pieces (title first, timeline later) is looked
/// up once, with the new length.
pub const SETTLE: Duration = Duration::from_secs(1);
/// A length change this large under the same title means the next track's
/// timeline arrived before its title.
const LENGTH_JUMP_MS: u64 = 2_000;
const USER_AGENT: &str = concat!(
    "DynamicIsland/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/hartono0297/dynamic-island)"
);
const API: &str = "https://lrclib.net/api/get";

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
/// (stable: equal times keep their order).
pub fn parse_lrc(text: &str) -> Vec<LrcLine> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut offset: i64 = 0;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(tag_end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            let tag = &rest[1..=tag_end];
            if let Some(ms) = parse_time(tag) {
                times.push(ms);
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
        lines.extend(times.into_iter().map(|at_ms| (at_ms, text.to_owned())));
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

/// The playback position at `now`: frozen while paused, advancing in real
/// time while playing, never past the end when the end is known.
pub fn position_at(t: &Timeline, playing: bool, now: Instant) -> u64 {
    let pos = if playing {
        let elapsed = now.saturating_duration_since(t.updated).as_millis();
        t.position_ms
            .saturating_add(u64::try_from(elapsed).unwrap_or(u64::MAX))
    } else {
        t.position_ms
    };
    if t.end_ms > 0 { pos.min(t.end_ms) } else { pos }
}

/// The same moment re-expressed at `now`: the position then, as of now. Used
/// when play/pause flips, so a player that does not refresh its timeline
/// neither runs on while paused nor jumps when it resumes.
pub fn rebase(t: &Timeline, was_playing: bool, now: Instant) -> Timeline {
    Timeline {
        position_ms: position_at(t, was_playing, now),
        end_ms: t.end_ms,
        updated: now,
    }
}

/// Whether a freshly read timeline differs from the one last sent: one of
/// them missing, another length, or a position more than `SEEK_TOLERANCE_MS`
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
                || position_at(a, playing, now).abs_diff(position_at(b, playing, now))
                    > SEEK_TOLERANCE_MS
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
    let secs = (q.duration_ms as f64 / 1000.0).clamp(1.0, 3600.0);
    let duration = format!("{}", secs.round() as u64);
    let query = form_encode(&[
        ("track_name", &q.title),
        ("artist_name", &q.artist),
        ("duration", &duration),
    ]);
    format!("{API}?{query}")
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

pub fn read_answer(status: u16, body: &str) -> Answer {
    match status {
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

/// `<dir>\<hash>.lrc` holds a track's LRC; `<hash>.none` holds the Unix time
/// a lookup found nothing, trusted for `NEGATIVE_TTL_SECS`.
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `%APPDATA%\DynamicIsland\lyrics-cache`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("DynamicIsland")
            .join("lyrics-cache")
    }

    fn file(&self, key: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", key_hash(key)))
    }

    pub fn read(&self, key: &str, now_unix: i64) -> Cached {
        if let Ok(lrc) = std::fs::read_to_string(self.file(key, "lrc")) {
            return Cached::Lrc(lrc);
        }
        let marker = self.file(key, "none");
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

    /// Write errors only cost a lookup next time; they are ignored.
    pub fn write_lrc(&self, key: &str, lrc: &str) {
        let _ = write_atomic(&self.file(key, "lrc"), lrc);
        let _ = std::fs::remove_file(self.file(key, "none"));
    }

    pub fn write_nothing(&self, key: &str, now_unix: i64) {
        let _ = write_atomic(&self.file(key, "none"), &now_unix.to_string());
    }
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
    /// The key last sent to the worker; not asked again while it is current,
    /// so a failed lookup is not retried until another track was asked.
    asked: Option<String>,
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
        if fetch && self.asked.as_deref() != Some(q.key.as_str()) {
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
    /// track playing now.
    pub fn found(&mut self, f: LyricsFound) {
        if let Some(lines) = f.lines
            && self.wanted.as_deref() == Some(f.key.as_str())
        {
            self.set_current(f.key, lines);
        }
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

    /// Milliseconds from `pos_ms` until the line next changes, plus 1 so the
    /// change has happened when a timer fires; `None` after the last line.
    pub fn wait_ms(&self, pos_ms: u64) -> Option<u64> {
        next_change(self.lines(), pos_ms).map(|at| at - pos_ms + 1)
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
        *http = Http::new().ok();
    }
    let Some(client) = http.as_ref() else {
        return found(q, None, true);
    };
    let answer = match client.get_with_headers(&lrclib_url(q), &[("User-Agent", USER_AGENT)]) {
        Ok((status, body)) => read_answer(status, &body),
        Err(_) => Answer::Failed,
    };
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
        // The player re-reported where playback is: not a move.
        let same = tl(15_200, 200_000, now);
        assert!(!timeline_moved(Some(&last), Some(&same), true, now));
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
            Some(&tl(10_300, 200_000, now)),
            false,
            now
        ));
        assert!(timeline_moved(None, Some(&last), true, now));
        assert!(timeline_moved(Some(&last), None, true, now));
        assert!(!timeline_moved(None, None, true, now));
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
        assert!(USER_AGENT.starts_with("DynamicIsland/"));
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
            let dir = std::env::temp_dir().join(format!(
                "dynamic-island-lyrics-{name}-{}",
                std::process::id()
            ));
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
        assert_eq!(cache.read("gone", now), Cached::Nothing(now));
        assert_eq!(
            cache.read("gone", now + NEGATIVE_TTL_SECS - 1),
            Cached::Nothing(now)
        );
        // Expired: a miss, and the marker is removed.
        assert_eq!(cache.read("gone", now + NEGATIVE_TTL_SECS), Cached::Miss);
        let marker = tmp
            .0
            .join("lyrics-cache")
            .join(format!("{}.none", key_hash("gone")));
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
    fn a_failure_is_retried_only_after_another_track() {
        let mut st = LyricsState::default();
        let a = query("a");
        assert!(st.want(Some(&a), true).is_some());
        st.found(found(&a.key, None, true));
        assert_eq!(st.want(Some(&a), true), None);
        let b = query_len("b", 245_000);
        assert!(st.want(Some(&b), true).is_some());
        st.found(found(&b.key, Some(song()), false));
        assert_eq!(st.want(Some(&a), true), Some(a));
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
        assert_eq!(st.wait_ms(0), Some(1_001));
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
        assert_eq!(st.wait_ms(200), Some(801));
        assert_eq!(st.wait_ms(9_000), None, "after the last line");
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
        assert_eq!(st.wait_ms(2_500), Some(501));
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
            .get_with_headers(&lrclib_url(&q), &[("User-Agent", USER_AGENT)])
            .unwrap();
        println!("status {status}, {} bytes", body.len());
        let Answer::Synced(lrc) = read_answer(status, &body) else {
            panic!("no synced lyrics: {status} {body}");
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
