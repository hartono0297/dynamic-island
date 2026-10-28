//! The clipboard history: newest first, capped, with pins, de-duplication,
//! search and the image budgets. `ingest` turns a `ClipSnapshot` (what the
//! Win32 reader saw, or a test's fake) into an entry. Pure.

use std::path::PathBuf;
use std::sync::Arc;

use super::image::{self, Thumb};
use super::kinds::{ClipKind, detect_kind, should_skip};

/// Default and range of "Jumlah item".
pub const DEFAULT_ITEMS: u32 = 50;
pub const MIN_ITEMS: u32 = 10;
pub const MAX_ITEMS: u32 = 200;
pub const ITEMS_STEP: u32 = 10;
/// A text longer than this (in UTF-8 bytes) is not kept at all: a cut copy
/// pasted back would silently lose its end.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Bitmaps larger than this are not read.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// All thumbnails together (memory and file); the oldest unpinned image
/// entries go first when a new one would pass it.
pub const THUMB_BUDGET: usize = 4 * 1024 * 1024;
/// Full-size image data, memory only; the oldest images lose theirs first
/// and then copy back at thumbnail size.
pub const FULL_BUDGET: usize = 48 * 1024 * 1024;

pub fn clamp_items(n: u32) -> u32 {
    n.clamp(MIN_ITEMS, MAX_ITEMS)
}

/// A bitmap entry: the thumbnail (kept and saved) and, while the budget
/// allows, the original DIB (memory only).
#[derive(Clone, Debug, PartialEq)]
pub struct ClipImage {
    pub width: u32,
    pub height: u32,
    pub thumb: Thumb,
    /// `image::fingerprint` of the original, for de-duplication.
    pub fingerprint: u64,
    /// Only bitmap formats were on the clipboard (PrtSc, Snipping Tool).
    pub screenshot: bool,
    pub full: Option<Arc<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipData {
    Text(String),
    Files(Vec<PathBuf>),
    Image(ClipImage),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Unique for this run (and kept in the file).
    pub id: u64,
    pub kind: ClipKind,
    pub data: ClipData,
    pub pinned: bool,
    /// When it was (last) copied, unix seconds.
    pub at: i64,
}

impl Clip {
    /// The text the card, the search and "copy as plain text" use: the
    /// text itself, paths one per line, nothing for an image.
    pub fn plain_text(&self) -> Option<String> {
        match &self.data {
            ClipData::Text(t) => Some(t.clone()),
            ClipData::Files(paths) => Some(
                paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\r\n"),
            ),
            ClipData::Image(_) => None,
        }
    }

    pub fn image(&self) -> Option<&ClipImage> {
        match &self.data {
            ClipData::Image(i) => Some(i),
            _ => None,
        }
    }

    /// Same content as `other` (the id, pin and time aside).
    fn same_content(&self, other: &ClipData) -> bool {
        match (&self.data, other) {
            (ClipData::Text(a), ClipData::Text(b)) => a == b,
            (ClipData::Files(a), ClipData::Files(b)) => a == b,
            (ClipData::Image(a), ClipData::Image(b)) => {
                (a.width, a.height, a.fingerprint) == (b.width, b.height, b.fingerprint)
            }
            _ => false,
        }
    }
}

/// What the reader found on the clipboard, closed again by now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipSnapshot {
    /// Registered formats by name, with the DWORD of an excluded one.
    pub formats: Vec<(String, Option<u32>)>,
    /// Only bitmap formats were present.
    pub only_bitmaps: bool,
    pub text: Option<String>,
    pub files: Vec<PathBuf>,
    /// A packed DIB.
    pub dib: Option<Vec<u8>>,
    /// `GetClipboardSequenceNumber()` when it was read (the screenshot
    /// preview's "Hapus" empties only while it is still this one).
    pub sequence: u32,
}

/// What `ingest` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingested {
    /// Password-manager content: nothing stored, nothing logged.
    Skipped,
    /// Nothing the history keeps (an unknown format, too big, empty).
    Nothing,
    Added {
        id: u64,
        image: bool,
    },
    /// The same content was already there: moved to the front.
    Bumped {
        id: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Newest first.
    items: Vec<Clip>,
    next_id: u64,
    cap: usize,
}

impl History {
    pub fn new(cap: u32) -> Self {
        History {
            items: Vec::new(),
            next_id: 1,
            cap: clamp_items(cap) as usize,
        }
    }

    /// Restored entries (newest first), ids renumbered.
    pub fn from_items(cap: u32, items: Vec<Clip>) -> Self {
        let mut h = History::new(cap);
        for mut clip in items {
            clip.id = h.take_id();
            h.items.push(clip);
        }
        h.enforce();
        h
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn items(&self) -> &[Clip] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Clip> {
        self.items.iter().find(|c| c.id == id)
    }

    pub fn set_cap(&mut self, cap: u32) {
        self.cap = clamp_items(cap) as usize;
        self.enforce();
    }

    /// Adds `data` at the front, or moves an entry with the same content
    /// there (its pin kept), then applies the cap and the image budgets.
    pub fn push(&mut self, kind: ClipKind, data: ClipData, at: i64) -> Ingested {
        if let Some(i) = self.items.iter().position(|c| c.same_content(&data)) {
            let mut clip = self.items.remove(i);
            clip.at = at;
            // A newer copy of an image may bring its full size back.
            if let (ClipData::Image(old), ClipData::Image(new)) = (&mut clip.data, data)
                && old.full.is_none()
            {
                old.full = new.full;
            }
            let id = clip.id;
            self.items.insert(0, clip);
            self.enforce();
            return Ingested::Bumped { id };
        }
        let image = matches!(data, ClipData::Image(_));
        let id = self.take_id();
        self.items.insert(
            0,
            Clip {
                id,
                kind,
                data,
                pinned: false,
                at,
            },
        );
        self.enforce();
        Ingested::Added { id, image }
    }

    /// The island copied entry `id` back: it moves to the front, no
    /// duplicate is made.
    pub fn bump(&mut self, id: u64, at: i64) -> bool {
        let Some(i) = self.items.iter().position(|c| c.id == id) else {
            return false;
        };
        let mut clip = self.items.remove(i);
        clip.at = at;
        self.items.insert(0, clip);
        true
    }

    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> bool {
        let Some(c) = self.items.iter_mut().find(|c| c.id == id) else {
            return false;
        };
        let changed = c.pinned != pinned;
        c.pinned = pinned;
        if changed {
            self.enforce();
        }
        changed
    }

    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|c| c.id != id);
        self.items.len() != before
    }

    /// Everything not pinned goes (the clear button, a locked screen).
    /// True when something went.
    pub fn clear_unpinned(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|c| c.pinned);
        self.items.len() != before
    }

    /// The cap (oldest unpinned first, never a pin and never the newest
    /// entry), then the thumbnail budget (oldest unpinned images go), then
    /// the full-size budget (the oldest images lose their original).
    fn enforce(&mut self) {
        while self.items.len() > self.cap {
            let Some(i) = self.oldest_unpinned(|_| true) else {
                break;
            };
            self.items.remove(i);
        }
        while self.thumb_bytes() > THUMB_BUDGET {
            let Some(i) = self.oldest_unpinned(|c| c.image().is_some()) else {
                break;
            };
            self.items.remove(i);
        }
        let mut full = 0usize;
        for clip in &mut self.items {
            if let ClipData::Image(img) = &mut clip.data
                && let Some(bytes) = &img.full
            {
                if full + bytes.len() > FULL_BUDGET {
                    img.full = None;
                } else {
                    full += bytes.len();
                }
            }
        }
    }

    /// The last unpinned entry matching `f`, never the newest one.
    fn oldest_unpinned(&self, f: impl Fn(&Clip) -> bool) -> Option<usize> {
        (1..self.items.len())
            .rev()
            .find(|&i| !self.items[i].pinned && f(&self.items[i]))
    }

    pub fn thumb_bytes(&self) -> usize {
        self.items
            .iter()
            .filter_map(Clip::image)
            .map(|i| i.thumb.bytes())
            .sum()
    }

    /// The entries matching `query` (case-insensitive, over the text, the
    /// paths and the kind's label from `label`), newest first. An empty
    /// query matches all.
    pub fn search<'a>(&'a self, query: &str, label: impl Fn(ClipKind) -> &'a str) -> Vec<&'a Clip> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.items.iter().collect();
        }
        self.items
            .iter()
            .filter(|c| {
                label(c.kind).to_lowercase().contains(&q)
                    || c.plain_text()
                        .is_some_and(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }
}

/// The history's side of a clipboard change: password-manager content is
/// skipped, then files, else text, else a bitmap becomes an entry.
pub fn ingest(history: &mut History, snap: ClipSnapshot, at: i64) -> Ingested {
    if should_skip(&snap.formats) {
        return Ingested::Skipped;
    }
    if !snap.files.is_empty() {
        return history.push(ClipKind::Files, ClipData::Files(snap.files), at);
    }
    if let Some(text) = snap.text.filter(|t| !t.trim().is_empty()) {
        if text.len() > MAX_TEXT_BYTES {
            return Ingested::Nothing;
        }
        let kind = detect_kind(&text);
        return history.push(kind, ClipData::Text(text), at);
    }
    match make_image(snap.dib, snap.only_bitmaps) {
        Some(img) => history.push(ClipKind::Image, ClipData::Image(img), at),
        None => Ingested::Nothing,
    }
}

/// The bitmap entry for `dib`, if it is one the history reads.
fn make_image(dib: Option<Vec<u8>>, only_bitmaps: bool) -> Option<ClipImage> {
    let dib = dib.filter(|d| d.len() <= MAX_IMAGE_BYTES)?;
    let info = image::dib_info(&dib)?;
    Some(ClipImage {
        width: info.w,
        height: info.h,
        thumb: image::thumbnail(&dib, &info),
        fingerprint: image::fingerprint(&dib),
        screenshot: only_bitmaps,
        full: (dib.len() <= FULL_BUDGET).then(|| Arc::new(dib)),
    })
}

/// With the history off and the screenshot preview on: the image `ingest`
/// would have made of `snap`, by the same rules (password-manager content
/// skipped, files and text first), without any entry.
pub fn image_only(snap: ClipSnapshot) -> Option<ClipImage> {
    if should_skip(&snap.formats)
        || !snap.files.is_empty()
        || snap.text.as_ref().is_some_and(|t| !t.trim().is_empty())
    {
        return None;
    }
    make_image(snap.dib, snap.only_bitmaps)
}

/// A one-line preview for the "Disalin" notice: whitespace runs become one
/// space, at most `max` characters (the renderer trims to the width).
pub fn preview(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &one[..i]),
        None => one,
    }
}

#[cfg(test)]
pub mod tests {
    use super::super::image::tests::dib24;
    use super::*;

    pub fn text(s: &str) -> ClipSnapshot {
        ClipSnapshot {
            text: Some(s.into()),
            ..ClipSnapshot::default()
        }
    }

    fn texts(h: &History) -> Vec<String> {
        h.items().iter().filter_map(Clip::plain_text).collect()
    }

    #[test]
    fn newest_first_with_a_cap_of_fifty() {
        let mut h = History::new(DEFAULT_ITEMS);
        for i in 0..60 {
            ingest(&mut h, text(&format!("item {i}")), i);
        }
        assert_eq!(h.len(), 50);
        assert_eq!(texts(&h)[0], "item 59");
        assert_eq!(texts(&h)[49], "item 10");
    }

    #[test]
    fn pins_survive_sixty_inserts() {
        let mut h = History::new(50);
        ingest(&mut h, text("keep me"), 0);
        let id = h.items()[0].id;
        assert!(h.set_pinned(id, true));
        for i in 0..60 {
            ingest(&mut h, text(&format!("n{i}")), i + 1);
        }
        assert_eq!(h.len(), 50);
        assert!(h.get(id).is_some_and(|c| c.pinned));
        // The pin keeps its place by age; 49 newest unpinned remain.
        assert_eq!(texts(&h)[0], "n59");
        assert_eq!(texts(&h).last().unwrap(), "keep me");
        assert_eq!(texts(&h)[48], "n11");
    }

    #[test]
    fn pins_over_the_cap_stay_and_the_newest_copy_too() {
        let mut h = History::new(10);
        for i in 0..12 {
            ingest(&mut h, text(&format!("p{i}")), i);
            let id = h.items()[0].id;
            h.set_pinned(id, true);
        }
        assert_eq!(h.len(), 12, "pins are never pushed out");
        ingest(&mut h, text("fresh"), 20);
        assert_eq!(h.len(), 13);
        assert_eq!(texts(&h)[0], "fresh");
        ingest(&mut h, text("fresher"), 21);
        // The older unpinned one goes, the newest stays.
        assert_eq!(h.len(), 13);
        assert_eq!(texts(&h)[0], "fresher");
        assert!(!texts(&h).contains(&"fresh".to_string()));
        // A lower cap applies at once.
        let mut h = History::new(50);
        for i in 0..30 {
            ingest(&mut h, text(&format!("x{i}")), i);
        }
        h.set_cap(10);
        assert_eq!(h.len(), 10);
    }

    #[test]
    fn the_cap_is_clamped() {
        assert_eq!(clamp_items(0), MIN_ITEMS);
        assert_eq!(clamp_items(5000), MAX_ITEMS);
        assert_eq!(clamp_items(50), 50);
    }

    #[test]
    fn same_text_moves_to_the_front_without_a_duplicate() {
        let mut h = History::new(50);
        ingest(&mut h, text("a"), 1);
        ingest(&mut h, text("b"), 2);
        let a = h.items()[1].id;
        h.set_pinned(a, true);
        assert_eq!(ingest(&mut h, text("a"), 3), Ingested::Bumped { id: a });
        assert_eq!(texts(&h), ["a", "b"]);
        assert!(h.items()[0].pinned, "the pin stays");
        assert_eq!(h.items()[0].at, 3);
    }

    #[test]
    fn an_island_copy_bumps_the_entry() {
        let mut h = History::new(50);
        ingest(&mut h, text("a"), 1);
        ingest(&mut h, text("b"), 2);
        let a = h.items()[1].id;
        assert!(h.bump(a, 5));
        assert_eq!(texts(&h), ["a", "b"]);
        assert!(!h.bump(999, 5));
    }

    #[test]
    fn delete_and_clear_unpinned() {
        let mut h = History::new(50);
        for s in ["a", "b", "c"] {
            ingest(&mut h, text(s), 0);
        }
        let b = h.items()[1].id;
        assert!(h.delete(b));
        assert!(!h.delete(b));
        assert_eq!(texts(&h), ["c", "a"]);
        let a = h.items()[1].id;
        h.set_pinned(a, true);
        // The lock (and the clear button) keeps pinned entries.
        assert!(h.clear_unpinned());
        assert_eq!(texts(&h), ["a"]);
        assert!(!h.clear_unpinned());
    }

    #[test]
    fn search_is_case_insensitive_over_text_paths_and_kind() {
        let mut h = History::new(50);
        ingest(&mut h, text("Denah lantai 2"), 1);
        ingest(&mut h, text("#E2683C"), 2);
        let files = ClipSnapshot {
            files: vec![PathBuf::from(r"D:\Proyek\Laporan.pdf")],
            ..ClipSnapshot::default()
        };
        ingest(&mut h, files, 3);
        let label = |k: ClipKind| match k {
            ClipKind::Color => "Warna",
            ClipKind::Files => "File",
            _ => "Teks",
        };
        let found =
            |q: &str| -> Vec<ClipKind> { h.search(q, label).iter().map(|c| c.kind).collect() };
        assert_eq!(found("DENAH"), [ClipKind::Text]);
        assert_eq!(found("laporan"), [ClipKind::Files]);
        assert_eq!(found("warna"), [ClipKind::Color]);
        assert_eq!(found("e268"), [ClipKind::Color]);
        assert_eq!(found("  ").len(), 3);
        assert!(found("tidak ada").is_empty());
    }

    #[test]
    fn files_win_over_text_and_huge_text_is_dropped() {
        let mut h = History::new(50);
        let both = ClipSnapshot {
            text: Some("x".into()),
            files: vec![PathBuf::from(r"C:\a.txt")],
            ..ClipSnapshot::default()
        };
        assert!(matches!(
            ingest(&mut h, both, 0),
            Ingested::Added { image: false, .. }
        ));
        assert_eq!(h.items()[0].kind, ClipKind::Files);
        let huge = "a".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(ingest(&mut h, text(&huge), 1), Ingested::Nothing);
        assert_eq!(ingest(&mut h, text("   "), 1), Ingested::Nothing);
        assert_eq!(
            ingest(&mut h, ClipSnapshot::default(), 1),
            Ingested::Nothing
        );
    }

    #[test]
    fn skipped_content_stores_nothing() {
        let mut h = History::new(50);
        let secret = ClipSnapshot {
            formats: vec![("CanIncludeInClipboardHistory".into(), Some(0))],
            text: Some("hunter2".into()),
            ..ClipSnapshot::default()
        };
        assert_eq!(ingest(&mut h, secret, 0), Ingested::Skipped);
        assert!(h.is_empty());
    }

    fn image(w: u32, h: u32) -> ClipSnapshot {
        ClipSnapshot {
            dib: Some(dib24(w, h)),
            only_bitmaps: true,
            ..ClipSnapshot::default()
        }
    }

    #[test]
    fn images_get_a_thumbnail_and_dedupe() {
        let mut h = History::new(50);
        let added = ingest(&mut h, image(1280, 720), 0);
        assert!(matches!(added, Ingested::Added { image: true, .. }));
        let img = h.items()[0].image().unwrap();
        assert_eq!((img.width, img.height), (1280, 720));
        assert!(img.thumb.w <= image::THUMB_MAX_W && img.thumb.h <= image::THUMB_MAX_H);
        assert!(img.screenshot && img.full.is_some());
        assert!(matches!(
            ingest(&mut h, image(1280, 720), 1),
            Ingested::Bumped { .. }
        ));
        assert_eq!(h.len(), 1);
        let bad = ClipSnapshot {
            dib: Some(b"not a bitmap".to_vec()),
            ..ClipSnapshot::default()
        };
        assert_eq!(ingest(&mut h, bad, 2), Ingested::Nothing);
    }

    #[test]
    fn image_only_follows_the_ingest_rules() {
        let img = image_only(image(1280, 720)).unwrap();
        assert_eq!((img.width, img.height, img.screenshot), (1280, 720, true));
        // Password-manager content and text or files first: no image.
        let secret = ClipSnapshot {
            formats: vec![("ExcludeClipboardContentFromMonitorProcessing".into(), None)],
            ..image(10, 10)
        };
        assert_eq!(image_only(secret), None);
        let with_text = ClipSnapshot {
            text: Some("x".into()),
            ..image(10, 10)
        };
        assert_eq!(image_only(with_text), None);
        assert_eq!(image_only(ClipSnapshot::default()), None);
    }

    #[test]
    fn thumbnails_stay_inside_their_budget() {
        let mut h = History::new(MAX_ITEMS);
        // ~75 KB thumbnails: 60 of them pass 4 MB.
        for i in 0..60 {
            ingest(&mut h, image(368 + i, 256), i64::from(i));
        }
        assert!(h.thumb_bytes() <= THUMB_BUDGET);
        assert!(h.len() < 60 && h.len() > 40);
        // The newest survive.
        assert_eq!(h.items()[0].image().unwrap().width, 427);
    }

    #[test]
    fn previews_are_one_short_line() {
        assert_eq!(preview("a\n  b\tc", 10), "a b c");
        assert_eq!(preview("abcdefghij", 4), "abcd…");
    }
}
