//! The screenshot preview (Run 8 of the v3.1 plan, keputusan #6): deciding
//! whether a new clipboard image is a screenshot, finding its file in the
//! Screenshots folder, and the rules for "Hapus". The pure parts (PNG and
//! JPEG headers, the matching, the clipboard rule) are tested on bytes and
//! fakes; `list_folder` reads a folder it is given (tests make one under
//! `target/test-tmp/`); `win` holds the Win32 side the running island alone
//! calls. See docs/memory/screenshot-preview.md.

pub mod win;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A folder file counts from this long before the image reached the
/// clipboard (PrtSc may write first) ...
pub const BEFORE_MS: i64 = 3_000;
/// ... to this long after it (Snipping Tool writes a moment later).
pub const AFTER_MS: i64 = 5_000;
/// The header bytes read from a file for its size: a PNG needs 24, a JPEG's
/// frame header can sit behind a large EXIF block.
const HEADER_BYTES: u64 = 256 * 1024;

/// Two thumbnail hashes further apart than this many bits are different
/// pictures (out of 64; a re-encode or a resample moves a few).
pub const HASH_MAX_DIST: u32 = 10;
/// The looser distance the preview's match and Simpan's "already saved" accept
/// (never a delete): an average hash of a low-contrast picture flips cells
/// near the mean easily, and the clipboard thumbnail (a box sampler) and the
/// file's (WIC Fant) are not decoded alike. Other pictures sit far above it
/// (a random pair is about 32 of 64 bits apart).
pub const LAX_HASH_DIST: u32 = 16;
/// Simpan waits for Snipping Tool's file this long after the capture (longer
/// than `AFTER_MS`: a cold start writes late, and writing a second copy is the
/// worse mistake); the notice then comes up to this + `SETTLE_MS` after the
/// capture when no file exists at all (PrtSc without auto-save).
pub const SAVE_AFTER_MS: i64 = 7_000;
/// A clipboard update of the same picture this soon after the shown or
/// waiting screenshot's is the same capture again (a tool that sets the
/// clipboard twice), not a new screenshot.
pub const SAME_CAPTURE_MS: i64 = 2_000;
/// After Hapus, an update of the deleted capture is taken off the clipboard
/// and out of the history again only this soon after its last update (and
/// with strictly agreeing hashes, `takes_back`): a deliberate retake of an
/// unchanged screen must not be wiped on the lax rule.
pub const TAKE_BACK_MS: i64 = 1_000;

/// The new clipboard image, as far as matching cares: when it arrived
/// (unix ms), its size in pixels and its thumbnail's hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipCandidate {
    pub at_ms: i64,
    /// The earliest file time it may claim: `at_ms - BEFORE_MS`, or its own
    /// arrival when another screenshot came just before (files before it
    /// are that one's).
    pub from_ms: i64,
    /// The latest file time it may claim: `at_ms + AFTER_MS`, or just before
    /// the next image's arrival when one came sooner (`window_end`).
    pub to_ms: i64,
    pub w: u32,
    pub h: u32,
    /// `thumb_hash` of its thumbnail; `None` when it had none.
    pub hash: Option<u64>,
}

/// An image file in the Screenshots folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCandidate {
    pub path: PathBuf,
    /// Its time (the later of created and modified), unix ms.
    pub at_ms: i64,
    pub bytes: u64,
    pub w: u32,
    pub h: u32,
    /// `thumb_hash` of the decoded file; `None` when it could not be read
    /// (still being written, a codec missing).
    pub hash: Option<u64>,
}

/// An 8 × 8 average hash of BGRA pixels (`w` × `h`, alpha ignored): each
/// cell's mean brightness against the mean of all 64. Robust to the small
/// differences between the clipboard's thumbnail and the file's decode.
/// `None` for an empty or malformed buffer.
pub fn thumb_hash(bgra: &[u8], w: u32, h: u32) -> Option<u64> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || bgra.len() != w * h * 4 {
        return None;
    }
    let mut cells = [0f64; 64];
    for (i, cell) in cells.iter_mut().enumerate() {
        let (cx, cy) = (i % 8, i / 8);
        // Each cell covers at least one pixel, even in a tiny image.
        let (x0, x1) = (cx * w / 8, ((cx + 1) * w / 8).max(cx * w / 8 + 1).min(w));
        let (y0, y1) = (cy * h / 8, ((cy + 1) * h / 8).max(cy * h / 8 + 1).min(h));
        let mut sum = 0f64;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = &bgra[(y * w + x) * 4..][..3];
                // Rec. 601 luma from B, G, R.
                sum += 0.114 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.299 * f64::from(p[2]);
            }
        }
        *cell = sum / ((x1 - x0) * (y1 - y0)) as f64;
    }
    let mean = cells.iter().sum::<f64>() / 64.0;
    Some(cells.iter().enumerate().fold(
        0u64,
        |acc, (i, &c)| if c > mean { acc | 1 << i } else { acc },
    ))
}

/// The hashes say the same picture (within `LAX_HASH_DIST`), or one of them
/// is unknown. Lax: good enough to show a file's size and to say "already
/// saved", never enough to delete it (`confirmed`).
fn hash_agrees(clip: Option<u64>, file: Option<u64>) -> bool {
    match (clip, file) {
        (Some(a), Some(b)) => (a ^ b).count_ones() <= LAX_HASH_DIST,
        _ => true,
    }
}

/// How sure the hashes are: 0 both read and within `HASH_MAX_DIST`, 1 both
/// read and only within `LAX_HASH_DIST` (or further), 2 one is unknown.
fn hash_tier(clip: Option<u64>, file: Option<u64>) -> u8 {
    match (clip, file) {
        (Some(a), Some(b)) => u8::from((a ^ b).count_ones() > HASH_MAX_DIST),
        _ => 2,
    }
}

/// Both hashes were read and say the same picture (within `HASH_MAX_DIST`).
/// What a delete needs: an unknown hash on either side is no proof, and the
/// looser distance of the preview and Simpan is not enough to recycle a file.
pub fn confirmed(clip: Option<u64>, file: Option<u64>) -> bool {
    hash_tier(clip, file) == 0
}

/// A matched file the delete worker may still accept (`confirmed`): it is
/// already, or the clipboard's hash is known and the file's was not read yet
/// (the worker hashes it again). A lax-only match (both read, more than
/// `HASH_MAX_DIST` apart) or a clipboard without a hash never will.
pub fn may_confirm(clip: Option<u64>, file: Option<u64>) -> bool {
    match (clip, file) {
        (Some(_), None) => true,
        _ => confirmed(clip, file),
    }
}

/// The file that is this screenshot (the lax match, for the preview and for
/// Simpan): from `from_ms` to `to_ms` with the very same size and a thumbnail
/// hash that agrees within `LAX_HASH_DIST` or is unknown. A file whose hash
/// was read and is close (`HASH_MAX_DIST`) beats one only slightly off, which
/// beats one whose hash is unknown; then the closest in time wins (the later
/// one on a tie). A different size, a clearly different picture or a time
/// outside the window is no match.
pub fn pick_match<'a>(
    clip: &ClipCandidate,
    files: &'a [FileCandidate],
) -> Option<&'a FileCandidate> {
    files
        .iter()
        .filter(|f| in_window(clip, f) && hash_agrees(clip.hash, f.hash))
        .min_by_key(|f| {
            (
                hash_tier(clip.hash, f.hash),
                (f.at_ms - clip.at_ms).abs(),
                std::cmp::Reverse(f.at_ms),
            )
        })
}

/// `pick_match` among the files whose hash `confirmed`s the clipboard's:
/// the file Hapus may move to the Recycle Bin.
pub fn pick_confirmed<'a>(
    clip: &ClipCandidate,
    files: &'a [FileCandidate],
) -> Option<&'a FileCandidate> {
    files
        .iter()
        .filter(|f| in_window(clip, f) && confirmed(clip.hash, f.hash))
        .min_by_key(|f| ((f.at_ms - clip.at_ms).abs(), std::cmp::Reverse(f.at_ms)))
}

/// The `to_ms` of an image that arrived at `at_ms`, `arrivals` the images
/// seen since (any order): `AFTER_MS` later, or just before the first image
/// that came after it, whose files those are. So the earlier screenshot's
/// last looks never claim the next one's file.
#[cfg(test)]
pub fn window_end(at_ms: i64, arrivals: &[i64]) -> i64 {
    window_end_after(at_ms, arrivals, AFTER_MS)
}

/// `window_end` with a window of `after_ms` (Simpan's wait is longer).
pub fn window_end_after(at_ms: i64, arrivals: &[i64], after_ms: i64) -> i64 {
    let end = at_ms + after_ms;
    arrivals
        .iter()
        .filter(|&&a| a > at_ms)
        .min()
        .map_or(end, |&next| end.min(next - 1))
}

/// The arrival times of the latest clipboard images, shared with the
/// workers that keep looking for a file (`window_end` while they wait).
#[derive(Clone, Default)]
pub struct Arrivals(std::sync::Arc<std::sync::Mutex<Vec<i64>>>);

impl Arrivals {
    /// How many arrivals are kept: more than ever fall in one window.
    const CAP: usize = 16;

    pub fn push(&self, at_ms: i64) {
        let mut v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        v.push(at_ms);
        if v.len() > Self::CAP {
            v.remove(0);
        }
    }

    /// `window_end` of an image that arrived at `at_ms`, as of now.
    pub fn window_end(&self, at_ms: i64) -> i64 {
        self.window_end_after(at_ms, AFTER_MS)
    }

    /// The same with a window of `after_ms`.
    pub fn window_end_after(&self, at_ms: i64, after_ms: i64) -> i64 {
        window_end_after(
            at_ms,
            &self.0.lock().unwrap_or_else(|e| e.into_inner()),
            after_ms,
        )
    }

    /// The latest arrival before `ms`: the image before a capture whose
    /// window starts at `ms` (`save_decision`'s `earlier`).
    pub fn before(&self, ms: i64) -> Option<i64> {
        let v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        v.iter().copied().filter(|&a| a < ms).max()
    }
}

/// How often a worker that needs the file looks again while it waits.
pub const LOOK_EVERY_MS: i64 = 300;
/// A worker's last look comes this long after the window closes: a file
/// stamped at its end may still be being written.
pub const SETTLE_MS: i64 = 500;

/// Looks (`look`) until it finds something or the clock (`now`, unix ms)
/// passes `deadline` (asked again each time: a newer image can bring it
/// closer), sleeping `LOOK_EVERY_MS` at most between looks. There is always
/// one look at or after the deadline before it gives up.
pub fn wait_for<T>(
    deadline: impl Fn() -> i64,
    now: impl Fn() -> i64,
    mut look: impl FnMut() -> Option<T>,
    mut sleep: impl FnMut(u64),
) -> Option<T> {
    loop {
        if let Some(found) = look() {
            return Some(found);
        }
        let left = deadline() - now();
        if left <= 0 {
            return None;
        }
        sleep(left.min(LOOK_EVERY_MS) as u64);
    }
}

/// What Hapus reports for a matched file: one that is gone already (moved
/// or deleted meanwhile) is no file; otherwise `recycle` runs and gives its
/// failures (`clean::recycle`).
pub fn delete_outcome(exists: bool, recycle: impl FnOnce() -> u32) -> FileOutcome {
    if !exists {
        FileOutcome::NoFile
    } else if recycle() == 0 {
        FileOutcome::Recycled
    } else {
        FileOutcome::Failed
    }
}

/// The `from_ms` of an image arriving at `at_ms` when the image before it
/// arrived at `previous`: its own arrival when that one's files (written up
/// to `AFTER_MS` after it) could fall in its window, else `BEFORE_MS` earlier.
pub fn claim_from(at_ms: i64, previous: Option<i64>) -> i64 {
    let window = at_ms - BEFORE_MS;
    match previous {
        Some(p) if p + AFTER_MS >= window => at_ms,
        _ => window,
    }
}

/// Inside the time window (`from_ms..=to_ms`, which the callers keep within
/// `AFTER_MS`, or `SAVE_AFTER_MS` for Simpan) with the very same size: worth
/// hashing.
fn in_window(clip: &ClipCandidate, f: &FileCandidate) -> bool {
    (clip.from_ms..=clip.to_ms).contains(&f.at_ms) && (f.w, f.h) == (clip.w, clip.h)
}

/// A new clipboard image (`new`, history card `new_entry`) is the shown or
/// waiting screenshot's (`prev`, `prev_entry`) own capture again, not a new
/// screenshot: the same size, arriving at most `SAME_CAPTURE_MS` after it,
/// and the same history card or hashes that agree (an unknown hash is no
/// proof). Snipping Tool can set the clipboard more than once for one
/// capture; as a fresh candidate the second could never claim the file
/// stamped before it arrived.
pub fn same_capture(
    prev: &ClipCandidate,
    prev_entry: Option<u64>,
    new: &ClipCandidate,
    new_entry: Option<u64>,
) -> bool {
    let gap = new.at_ms - prev.at_ms;
    let same_card = prev_entry.is_some() && prev_entry == new_entry;
    let same_pixels = prev.hash.is_some() && new.hash.is_some() && hash_agrees(prev.hash, new.hash);
    (prev.w, prev.h) == (new.w, new.h)
        && (0..=SAME_CAPTURE_MS).contains(&gap)
        && (same_card || same_pixels)
}

/// After Hapus, whether an update the deleted capture absorbed
/// (`same_capture` held) is surely the deleted picture put back, so it may
/// go again (clipboard emptied, its card removed): both hashes read and
/// within `HASH_MAX_DIST` (`confirmed`), the same size, at most
/// `TAKE_BACK_MS` after `prev`. The card cannot prove it (Hapus removed that
/// one). Otherwise the update only stays quiet: no preview, nothing wiped.
pub fn takes_back(prev: &ClipCandidate, new: &ClipCandidate) -> bool {
    (prev.w, prev.h) == (new.w, new.h)
        && (0..=TAKE_BACK_MS).contains(&(new.at_ms - prev.at_ms))
        && confirmed(prev.hash, new.hash)
}

/// The candidate of a re-arrival of the same capture (`same_capture`): its
/// own arrival and end, but the files from where the first one's window began.
pub fn merge_arrival(prev: &ClipCandidate, new: ClipCandidate) -> ClipCandidate {
    ClipCandidate {
        from_ms: prev.from_ms.min(new.from_ms),
        hash: new.hash.or(prev.hash),
        ..new
    }
}

/// What Simpan does for a screenshot with no matched file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Windows wrote a file for this screenshot: nothing is written.
    AlreadySaved(PathBuf),
    /// No lax match, only the one unclaimed same-size file in the window,
    /// its hash far off: a guess, taken only when the wait is over
    /// (`wait_for_save`). This screenshot's own file may still be on its
    /// way and is the better candidate.
    Lone(PathBuf),
    /// No file appeared: Simpan writes its own PNG.
    Write,
}

/// Simpan's decision over the files listed (and hashed) in `clip`'s window:
/// the lax match (`pick_match`: size, time, a hash that agrees within
/// `LAX_HASH_DIST` or cannot be read) among the files no other screenshot
/// has `taken`: `AlreadySaved`. With none, the one unclaimed file of the
/// very same size in the window when it is the only one, whatever its hash
/// says (an average hash of a low-contrast picture can read far off, and a
/// second copy is the worse mistake): only `Lone`, a guess the wait takes at
/// its end (`wait_for_save`). That file must also lie past the Simpan window
/// of the image that arrived before this capture (`earlier`, its arrival):
/// `claim_from` keeps only that one's `AFTER_MS` files out, and a late
/// (cold) file of it, unclaimed and unanswered, is not this one's. A file
/// found is never copied again; only when none is, Simpan writes one.
/// Deletes never use this (`pick_confirmed`).
pub fn save_decision(
    clip: &ClipCandidate,
    files: &[FileCandidate],
    taken: &[PathBuf],
    earlier: Option<i64>,
) -> SaveOutcome {
    let free: Vec<FileCandidate> = files
        .iter()
        .filter(|f| !taken.contains(&f.path))
        .cloned()
        .collect();
    if let Some(f) = pick_match(clip, &free) {
        return SaveOutcome::AlreadySaved(f.path.clone());
    }
    let mut sized = free.iter().filter(|f| in_window(clip, f));
    match (sized.next(), sized.next()) {
        (Some(f), None) if earlier.is_none_or(|e| f.at_ms > e + SAVE_AFTER_MS) => {
            SaveOutcome::Lone(f.path.clone())
        }
        _ => SaveOutcome::Write,
    }
}

/// Simpan's wait (`wait_for`) over `look` (`save_decision` on the folder as
/// of now): the first `AlreadySaved` ends it. A `Lone` guess never does,
/// as this screenshot's own file may turn up on a later look; the guess of
/// the last look (the one at or after the deadline) is kept when nothing
/// better came. `None`: Simpan writes.
pub fn wait_for_save(
    deadline: impl Fn() -> i64,
    now: impl Fn() -> i64,
    mut look: impl FnMut() -> SaveOutcome,
    sleep: impl FnMut(u64),
) -> Option<PathBuf> {
    let mut lone = None;
    let found = wait_for(
        deadline,
        now,
        || match look() {
            SaveOutcome::AlreadySaved(path) => Some(path),
            SaveOutcome::Lone(path) => {
                lone = Some(path);
                None
            }
            SaveOutcome::Write => {
                lone = None;
                None
            }
        },
        sleep,
    );
    found.or(lone)
}

/// "Hapus" empties the clipboard only while it still holds this
/// screenshot: nothing was copied since its sequence number was recorded.
pub fn should_empty(current: u32, recorded: u32) -> bool {
    current == recorded
}

/// A PNG's width and height from its IHDR chunk; `None` for anything else
/// or a buffer too short.
pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if b.get(..8)? != SIGNATURE || b.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let (w, h) = (be(16)?, be(20)?);
    (w > 0 && h > 0).then_some((w, h))
}

/// A JPEG's width and height from its first frame header (SOF0..SOF15 but
/// DHT, JPG and DAC); `None` for anything else or a header cut short.
pub fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    loop {
        // Fill bytes before a marker.
        while *b.get(i)? == 0xFF && *b.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *b.get(i)? != 0xFF {
            return None;
        }
        let marker = *b.get(i + 1)?;
        // Markers without a length.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
        if len < 2 {
            return None;
        }
        let sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if sof {
            let h = u16::from_be_bytes([*b.get(i + 5)?, *b.get(i + 6)?]);
            let w = u16::from_be_bytes([*b.get(i + 7)?, *b.get(i + 8)?]);
            return (w > 0 && h > 0).then_some((u32::from(w), u32::from(h)));
        }
        // Start of scan or end of image before any frame header.
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        i += 2 + len;
    }
}

/// An image's size from its first bytes: PNG, else JPEG.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    png_size(b).or_else(|| jpeg_size(b))
}

/// `.png`, `.jpg` or `.jpeg`, any case.
fn is_image_name(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["png", "jpg", "jpeg"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

/// Unix ms of `t`; 0 before 1970.
pub fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64)
}

/// The image files directly in `dir` whose time falls in `from_ms..=to_ms`,
/// with their size read from the header. Never a reparse point (symlink,
/// junction, cloud placeholder), never a folder below. One listing; the
/// caller repeats it if it wants to.
pub fn list_folder(dir: &Path, from_ms: i64, to_ms: i64) -> Vec<FileCandidate> {
    use std::io::Read;
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_image_name(&path) {
            continue;
        }
        // Not followed: the entry's own metadata.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.file_attributes() & REPARSE_POINT != 0 {
            continue;
        }
        let modified = meta.modified().map(unix_ms).unwrap_or(0);
        let created = meta.created().map(unix_ms).unwrap_or(0);
        let at_ms = modified.max(created);
        if !(from_ms..=to_ms).contains(&at_ms) {
            continue;
        }
        let mut head = Vec::new();
        let read =
            std::fs::File::open(&path).and_then(|f| f.take(HEADER_BYTES).read_to_end(&mut head));
        let Some((w, h)) = read.ok().and_then(|_| image_size(&head)) else {
            continue;
        };
        out.push(FileCandidate {
            path,
            at_ms,
            bytes: meta.len(),
            w,
            h,
            hash: None,
        });
    }
    out
}

/// One folder check for `clip`: the listing of its window, the hash of
/// each file of its size (`hash_of` decodes one; only those are decoded),
/// then the match. A file in `taken` belongs to another screenshot.
pub fn find_in(
    dir: &Path,
    clip: &ClipCandidate,
    taken: &[PathBuf],
    hash_of: impl Fn(&Path) -> Option<u64>,
) -> Option<FileCandidate> {
    pick_match(clip, &hashed_files(dir, clip, taken, hash_of)).cloned()
}

/// `find_in` for a delete: only a file whose hash was read and agrees closely
/// (`pick_confirmed`).
pub fn find_confirmed_in(
    dir: &Path,
    clip: &ClipCandidate,
    taken: &[PathBuf],
    hash_of: impl Fn(&Path) -> Option<u64>,
) -> Option<FileCandidate> {
    pick_confirmed(clip, &hashed_files(dir, clip, taken, hash_of)).cloned()
}

/// The files in `clip`'s window, of its size and not `taken`, hashed.
pub fn hashed_files(
    dir: &Path,
    clip: &ClipCandidate,
    taken: &[PathBuf],
    hash_of: impl Fn(&Path) -> Option<u64>,
) -> Vec<FileCandidate> {
    let mut files = list_folder(dir, clip.from_ms, clip.to_ms);
    files.retain(|f| in_window(clip, f) && !taken.contains(&f.path));
    for f in &mut files {
        f.hash = hash_of(&f.path);
    }
    files
}

/// How the folder is named in "Disimpan · …": its parent and itself
/// ("Pictures\Screenshots"), from the real path (OneDrive's included).
pub fn folder_label(dir: &Path) -> String {
    let parts: Vec<String> = dir
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [] => dir.to_string_lossy().into_owned(),
        [one] => one.clone(),
        [.., parent, name] => format!("{parent}\\{name}"),
    }
}

/// What became of the file when "Hapus" ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOutcome {
    /// It is in the Recycle Bin now.
    Recycled,
    /// No file was matched, even by the last check: nothing on disk changed.
    NoFile,
    /// It is still there (in use, refused, or kept at the shell's warning).
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend(13u32.to_be_bytes());
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b.extend([0u8; 4]);
        b
    }

    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        // An APP1 (EXIF) segment first.
        b.extend([0xFF, 0xE1, 0x00, 0x06, b'E', b'x', b'i', b'f']);
        b.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b.extend([0xFF, 0xD9]);
        b
    }

    fn file(name: &str, at_ms: i64, w: u32, h: u32) -> FileCandidate {
        FileCandidate {
            path: PathBuf::from(name),
            at_ms,
            bytes: 1000,
            w,
            h,
            hash: None,
        }
    }

    fn hashed(f: FileCandidate, hash: u64) -> FileCandidate {
        FileCandidate {
            hash: Some(hash),
            ..f
        }
    }

    const AT: i64 = 1_800_000_000_000;
    const CLIP: ClipCandidate = ClipCandidate {
        at_ms: AT,
        from_ms: AT - BEFORE_MS,
        to_ms: AT + AFTER_MS,
        w: 1280,
        h: 720,
        hash: None,
    };
    const PIC: u64 = 0x00FF_00FF_F0F0_0F0F;

    #[test]
    fn a_delete_needs_both_hashes_read_and_agreeing() {
        assert!(confirmed(Some(PIC), Some(PIC ^ 0b11)));
        assert!(!confirmed(Some(PIC), Some(!PIC)), "another picture");
        // Unknown on either side: fine to show the size, never to delete.
        assert!(!confirmed(None, Some(PIC)));
        assert!(!confirmed(Some(PIC), None));
        assert!(!confirmed(None, None));
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        let writing = file("writing.png", AT + 100, 1280, 720);
        let same = hashed(file("same.png", AT + 3_000, 1280, 720), PIC);
        // The lax match takes a file still being written; the strict one never.
        assert!(pick_match(&clip, std::slice::from_ref(&writing)).is_some());
        assert_eq!(pick_confirmed(&clip, std::slice::from_ref(&writing)), None);
        assert_eq!(
            pick_confirmed(&clip, &[writing, same]).unwrap().path,
            PathBuf::from("same.png")
        );
        // Without a clipboard hash nothing is ever confirmed.
        let hashless = [hashed(file("x.png", AT + 100, 1280, 720), PIC)];
        assert_eq!(pick_confirmed(&CLIP, &hashless), None);
        // The window and the size still hold.
        let late = [hashed(file("late.png", AT + AFTER_MS + 1, 1280, 720), PIC)];
        assert_eq!(pick_confirmed(&clip, &late), None);
    }

    #[test]
    fn the_last_look_of_a_screenshot_stops_where_the_next_one_arrives() {
        // No image since: the whole window.
        assert_eq!(window_end(AT, &[]), AT + AFTER_MS);
        assert_eq!(window_end(AT, &[AT - 9_000, AT]), AT + AFTER_MS);
        // The next image 2 s later: files from then on are its own.
        assert_eq!(window_end(AT, &[AT, AT + 2_000, AT + 4_000]), AT + 1_999);
        // One after the window changes nothing.
        assert_eq!(window_end(AT, &[AT + 7_000]), AT + AFTER_MS);
        let arrivals = Arrivals::default();
        arrivals.push(AT);
        assert_eq!(arrivals.window_end(AT), AT + AFTER_MS);
        arrivals.push(AT + 2_000);
        assert_eq!(arrivals.window_end(AT), AT + 1_999);
        // The previous screenshot's file written after the next arrived is
        // not matched to it any more.
        let prev = ClipCandidate {
            to_ms: window_end(AT, &[AT + 2_000]),
            ..CLIP
        };
        let next_file = [file("next.png", AT + 2_500, 1280, 720)];
        assert_eq!(pick_match(&prev, &next_file), None);
        assert!(pick_match(&CLIP, &next_file).is_some());
    }

    #[test]
    fn a_worker_keeps_looking_until_the_window_closes() {
        use std::cell::Cell;
        let clock = Cell::new(0i64);
        let looks = Cell::new(0u32);
        // The file turns up at 1.2 s: found then, no more waiting.
        let found = wait_for(
            || 5_000,
            || clock.get(),
            || {
                looks.set(looks.get() + 1);
                (clock.get() >= 1_200).then_some("file")
            },
            |ms| clock.set(clock.get() + ms as i64),
        );
        assert_eq!(found, Some("file"));
        assert!(clock.get() < 1_200 + LOOK_EVERY_MS, "{}", clock.get());
        // Never there: it gives up only after a look at the deadline.
        clock.set(0);
        looks.set(0);
        let last = Cell::new(-1i64);
        let none: Option<()> = wait_for(
            || 1_000,
            || clock.get(),
            || {
                looks.set(looks.get() + 1);
                last.set(clock.get());
                None
            },
            |ms| clock.set(clock.get() + ms as i64),
        );
        assert_eq!(none, None);
        assert_eq!(last.get(), 1_000, "the last look is at the deadline");
        assert!(looks.get() >= 4, "{} looks", looks.get());
        // Past the deadline already: one look, no sleep.
        clock.set(9_000);
        let slept = Cell::new(false);
        let _: Option<()> = wait_for(|| 1_000, || clock.get(), || None, |_| slept.set(true));
        assert!(!slept.get());
    }

    #[test]
    fn hapus_on_a_file_that_is_gone_reports_no_file() {
        let ran = std::cell::Cell::new(false);
        let gone = delete_outcome(false, || {
            ran.set(true);
            0
        });
        assert_eq!(gone, FileOutcome::NoFile);
        assert!(!ran.get(), "nothing to recycle");
        assert_eq!(delete_outcome(true, || 0), FileOutcome::Recycled);
        assert_eq!(delete_outcome(true, || 1), FileOutcome::Failed);
    }

    /// BGRA pixels: a white left half, a black right half, or the reverse.
    fn halves(w: u32, h: u32, white_left: bool) -> Vec<u8> {
        let mut b = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let white = (x < w / 2) == white_left;
                let v = if white { 250 } else { 5 };
                b.extend([v, v, v, 255]);
            }
        }
        b
    }

    #[test]
    fn thumb_hash_keeps_the_picture_across_sizes() {
        let a = thumb_hash(&halves(184, 104, true), 184, 104).unwrap();
        // The same picture at another size (the file's decode): the same bits.
        let b = thumb_hash(&halves(160, 90, true), 160, 90).unwrap();
        assert!((a ^ b).count_ones() <= HASH_MAX_DIST);
        // The mirror image is far apart.
        let c = thumb_hash(&halves(184, 104, false), 184, 104).unwrap();
        assert!((a ^ c).count_ones() > HASH_MAX_DIST);
        // Tiny images still hash; bad buffers do not.
        assert!(thumb_hash(&halves(2, 2, true), 2, 2).is_some());
        assert_eq!(thumb_hash(&[0; 7], 2, 2), None);
        assert_eq!(thumb_hash(&[], 0, 0), None);
    }

    #[test]
    fn matching_by_hash() {
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        // A different picture of the same size, in time: no match.
        let other = [hashed(file("other.png", AT + 100, 1280, 720), !PIC)];
        assert_eq!(pick_match(&clip, &other), None);
        // A few bits off (a re-encode): still the same picture.
        let close = [hashed(file("close.png", AT + 100, 1280, 720), PIC ^ 0b111)];
        assert!(pick_match(&clip, &close).is_some());
        // A confirmed hash beats a closer file whose hash is unknown.
        let files = [
            file("unknown.png", AT + 50, 1280, 720),
            hashed(file("same.png", AT + 2_000, 1280, 720), PIC),
        ];
        assert_eq!(
            pick_match(&clip, &files).unwrap().path,
            PathBuf::from("same.png")
        );
        // An unknown hash on either side falls back to time and size.
        assert!(pick_match(&clip, &files[..1]).is_some());
        assert!(pick_match(&CLIP, &files[1..]).is_some());
        // The hash never rescues a file outside the window or of another size.
        let off = [
            hashed(file("late.png", AT + AFTER_MS + 1, 1280, 720), PIC),
            hashed(file("small.png", AT + 10, 1279, 720), PIC),
        ];
        assert_eq!(pick_match(&clip, &off), None);
    }

    /// A clip of the shot arriving at `at` with `hash`, its files from `from`.
    fn clip_at(at: i64, from: i64, hash: Option<u64>) -> ClipCandidate {
        ClipCandidate {
            at_ms: at,
            from_ms: from,
            to_ms: at + AFTER_MS,
            hash,
            ..CLIP
        }
    }

    #[test]
    fn two_arrivals_of_one_capture_still_find_the_file_between_them() {
        // Snipping Tool sets the clipboard, writes its file, and sets the
        // clipboard again 600 ms after the first update.
        let first = clip_at(AT, claim_from(AT, None), Some(PIC));
        let file = hashed(file("Screenshot 235523.png", AT + 200, 1280, 720), PIC);
        let second_at = AT + 600;
        let fresh = clip_at(second_at, claim_from(second_at, Some(AT)), Some(PIC));
        // As a fresh candidate (the old behaviour) the second never claims it:
        // the file is stamped before its arrival.
        assert_eq!(pick_match(&fresh, std::slice::from_ref(&file)), None);
        assert_eq!(
            save_decision(&fresh, std::slice::from_ref(&file), &[], None),
            SaveOutcome::Write,
            "this is the duplicate Simpan wrote"
        );
        // Recognised as the same capture, it keeps the first one's window.
        assert!(same_capture(&first, Some(4), &fresh, Some(4)));
        let merged = merge_arrival(&first, fresh);
        assert_eq!(merged.at_ms, second_at);
        assert_eq!(merged.from_ms, first.from_ms);
        assert!(pick_match(&merged, std::slice::from_ref(&file)).is_some());
        assert_eq!(
            save_decision(&merged, std::slice::from_ref(&file), &[], None),
            SaveOutcome::AlreadySaved(PathBuf::from("Screenshot 235523.png"))
        );
        // The window now runs from the second arrival.
        assert_eq!(merged.to_ms, second_at + AFTER_MS);
    }

    #[test]
    fn the_same_capture_is_told_from_a_new_screenshot() {
        let first = clip_at(AT, AT - BEFORE_MS, Some(PIC));
        let again = |gap: i64, hash: Option<u64>| clip_at(AT + gap, AT + gap, hash);
        // Same card, or the same pixels, soon after.
        assert!(same_capture(&first, Some(1), &again(100, None), Some(1)));
        assert!(same_capture(&first, None, &again(800, Some(PIC)), None));
        assert!(same_capture(
            &first,
            None,
            &again(800, Some(PIC ^ 0xFFFF)),
            None
        ));
        // Another picture, a long gap, another size: a new screenshot.
        assert!(!same_capture(&first, None, &again(800, Some(!PIC)), None));
        assert!(!same_capture(
            &first,
            Some(1),
            &again(SAME_CAPTURE_MS + 1, Some(PIC)),
            Some(1)
        ));
        let bigger = ClipCandidate {
            w: 1920,
            ..again(100, Some(PIC))
        };
        assert!(!same_capture(&first, Some(1), &bigger, Some(1)));
        // An unknown hash and another card is no proof; never backwards.
        assert!(!same_capture(&first, Some(1), &again(100, None), Some(2)));
        assert!(!same_capture(
            &first,
            Some(1),
            &again(-5, Some(PIC)),
            Some(1)
        ));
    }

    #[test]
    fn simpan_takes_a_same_size_file_in_the_window_without_a_perfect_hash() {
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        // The file's hash could not be read (still locked / a decode failure).
        let unread = [file("a.png", AT + 2_100, 1280, 720)];
        assert_eq!(
            save_decision(&clip, &unread, &[], None),
            SaveOutcome::AlreadySaved(PathBuf::from("a.png"))
        );
        // Slightly off: more than the strict distance, within the lax one.
        let off = PIC ^ 0b1111_1111_1111;
        assert!((PIC ^ off).count_ones() > HASH_MAX_DIST);
        assert!((PIC ^ off).count_ones() <= LAX_HASH_DIST);
        let slightly = [hashed(file("b.png", AT + 900, 1280, 720), off)];
        assert!(matches!(
            save_decision(&clip, &slightly, &[], None),
            SaveOutcome::AlreadySaved(_)
        ));
        // No clipboard hash at all: size and time are enough.
        assert!(matches!(
            save_decision(&CLIP, &unread, &[], None),
            SaveOutcome::AlreadySaved(_)
        ));
    }

    #[test]
    fn simpan_writes_only_when_no_file_appeared() {
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        assert_eq!(save_decision(&clip, &[], &[], None), SaveOutcome::Write);
        // Another size, outside the window, or another screenshot's file.
        let files = [
            file("small.png", AT + 100, 640, 480),
            file("late.png", AT + AFTER_MS + 1, 1280, 720),
            file("theirs.png", AT + 100, 1280, 720),
        ];
        assert_eq!(
            save_decision(&clip, &files, &[PathBuf::from("theirs.png")], None),
            SaveOutcome::Write
        );
    }

    #[test]
    fn simpan_waits_longer_for_a_late_file_but_not_into_the_next_screenshot() {
        let at = AT;
        let end = window_end_after(at, &[at], SAVE_AFTER_MS);
        assert_eq!(end, at + SAVE_AFTER_MS);
        const { assert!(SAVE_AFTER_MS > AFTER_MS) };
        let clip = ClipCandidate {
            hash: Some(PIC),
            to_ms: end,
            ..CLIP
        };
        // 6 s after the clipboard: past the preview's window, inside Simpan's.
        let late = [hashed(file("late.png", AT + 6_000, 1280, 720), PIC)];
        assert!(matches!(
            save_decision(&clip, &late, &[], None),
            SaveOutcome::AlreadySaved(_)
        ));
        let preview = ClipCandidate {
            to_ms: AT + AFTER_MS,
            ..clip
        };
        assert_eq!(pick_match(&preview, &late), None);
        // A newer image ends it just before its arrival, as before.
        assert_eq!(
            window_end_after(at, &[at, at + 3_000], SAVE_AFTER_MS),
            at + 2_999
        );
        let arrivals = Arrivals::default();
        arrivals.push(at);
        assert_eq!(arrivals.window_end_after(at, SAVE_AFTER_MS), end);
        assert_eq!(arrivals.window_end(at), at + AFTER_MS);
    }

    #[test]
    fn a_clearly_different_picture_is_not_taken_when_a_better_candidate_exists() {
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        let other = hashed(file("other.png", AT + 100, 1280, 720), !PIC);
        let mine = hashed(file("mine.png", AT + 4_000, 1280, 720), PIC ^ 0b11);
        let files = [other.clone(), mine];
        assert_eq!(
            pick_match(&clip, &files).unwrap().path,
            PathBuf::from("mine.png")
        );
        assert_eq!(
            save_decision(&clip, &files, &[], None),
            SaveOutcome::AlreadySaved(PathBuf::from("mine.png"))
        );
        // Alone, the one unclaimed same-size file in the window is Simpan's
        // guess for the end of its wait (a far-off hash of a low-contrast
        // picture must never make a second copy), never the preview's or a
        // delete's.
        assert_eq!(
            save_decision(&clip, std::slice::from_ref(&other), &[], None),
            SaveOutcome::Lone(PathBuf::from("other.png"))
        );
        assert_eq!(pick_match(&clip, std::slice::from_ref(&other)), None);
        assert_eq!(pick_confirmed(&clip, std::slice::from_ref(&other)), None);
        // Two of them, or the lone one another screenshot's: nothing to say
        // which is this one's, so Simpan writes.
        let other2 = hashed(file("other2.png", AT + 900, 1280, 720), !PIC ^ 0xF);
        assert_eq!(
            save_decision(&clip, &[other.clone(), other2], &[], None),
            SaveOutcome::Write
        );
        assert_eq!(
            save_decision(
                &clip,
                std::slice::from_ref(&other),
                &[PathBuf::from("other.png")],
                None
            ),
            SaveOutcome::Write
        );
        // A slightly-off file loses to an exact one, which loses to nothing
        // but time: tiers are hash first.
        let near = hashed(file("near.png", AT + 100, 1280, 720), PIC ^ 0xFFF);
        let exact = hashed(file("exact.png", AT + 3_000, 1280, 720), PIC);
        assert_eq!(
            pick_match(&clip, &[near, exact]).unwrap().path,
            PathBuf::from("exact.png")
        );
    }

    #[test]
    fn a_far_hash_lone_file_never_ends_the_wait_before_the_right_one_comes() {
        use std::cell::Cell;
        let clip = ClipCandidate {
            hash: Some(PIC),
            to_ms: AT + SAVE_AFTER_MS,
            ..CLIP
        };
        // An unrelated same-size picture is in the window from the start;
        // this capture's own file is stamped at +1.5 s, seen from 1.8 s on.
        let stray = hashed(file("stray.png", AT - 1_000, 1280, 720), !PIC);
        let mine = hashed(file("mine.png", AT + 1_500, 1280, 720), PIC);
        let clock = Cell::new(AT);
        let looks = Cell::new(0u32);
        let folder = || {
            let mut files = vec![stray.clone()];
            if clock.get() >= AT + 1_800 {
                files.push(mine.clone());
            }
            files
        };
        let found = wait_for_save(
            || clip.to_ms + SETTLE_MS,
            || clock.get(),
            || {
                looks.set(looks.get() + 1);
                save_decision(&clip, &folder(), &[], None)
            },
            |ms| clock.set(clock.get() + ms as i64),
        );
        // The bug: the first look took the stray file and the wait ended.
        assert_eq!(found, Some(PathBuf::from("mine.png")));
        assert!(looks.get() > 1, "{} looks", looks.get());
        assert!(clock.get() < AT + 1_800 + LOOK_EVERY_MS, "{}", clock.get());
        // Never a better one: the guess of the last look (at the deadline).
        clock.set(AT);
        let last = Cell::new(0i64);
        let guess = wait_for_save(
            || clip.to_ms + SETTLE_MS,
            || clock.get(),
            || {
                last.set(clock.get());
                save_decision(&clip, std::slice::from_ref(&stray), &[], None)
            },
            |ms| clock.set(clock.get() + ms as i64),
        );
        assert_eq!(guess, Some(PathBuf::from("stray.png")));
        assert_eq!(last.get(), clip.to_ms + SETTLE_MS);
        // A guess that is gone by the last look is none: Simpan writes.
        clock.set(AT);
        let gone = wait_for_save(
            || AT + 1_000,
            || clock.get(),
            || {
                if clock.get() < AT + 1_000 {
                    SaveOutcome::Lone(PathBuf::from("stray.png"))
                } else {
                    SaveOutcome::Write
                }
            },
            |ms| clock.set(clock.get() + ms as i64),
        );
        assert_eq!(gone, None);
    }

    #[test]
    fn a_late_file_of_the_image_before_is_no_lone_guess() {
        // Screenshot P at p, never answered; Snipping Tool stamped its file
        // cold at p + 6 s (past P's preview window, so unclaimed). A plain
        // PrtSc A without auto-save arrives 8.5 s after P: its window starts
        // 3 s back and covers P's file.
        let p = AT - 8_500;
        let from = claim_from(AT, Some(p));
        assert_eq!(from, AT - BEFORE_MS);
        let clip = ClipCandidate {
            hash: Some(PIC),
            from_ms: from,
            to_ms: AT + SAVE_AFTER_MS,
            ..CLIP
        };
        let theirs = hashed(file("p.png", p + 6_000, 1280, 720), !PIC);
        let arrivals = Arrivals::default();
        arrivals.push(p);
        arrivals.push(AT);
        assert_eq!(arrivals.before(clip.from_ms), Some(p));
        assert_eq!(
            save_decision(
                &clip,
                std::slice::from_ref(&theirs),
                &[],
                arrivals.before(clip.from_ms)
            ),
            SaveOutcome::Write,
            "Simpan writes A, never 'already saved' for P's picture"
        );
        // Past P's Simpan window, the lone file is A's guess again; a lax
        // match is never held back by it.
        let lone = hashed(file("a.png", p + SAVE_AFTER_MS + 1, 1280, 720), !PIC);
        assert_eq!(
            save_decision(&clip, std::slice::from_ref(&lone), &[], Some(p)),
            SaveOutcome::Lone(PathBuf::from("a.png"))
        );
        let near = hashed(file("a.png", p + 6_000, 1280, 720), PIC);
        assert_eq!(
            save_decision(&clip, std::slice::from_ref(&near), &[], Some(p)),
            SaveOutcome::AlreadySaved(PathBuf::from("a.png"))
        );
        // A merged update's own first arrival is no earlier image.
        let merged = merge_arrival(
            &clip,
            ClipCandidate {
                at_ms: AT + 600,
                from_ms: AT + 600,
                ..clip
            },
        );
        arrivals.push(AT + 600);
        assert_eq!(arrivals.before(merged.from_ms), Some(p));
    }

    #[test]
    fn a_delete_still_refuses_what_simpan_would_accept() {
        let clip = ClipCandidate {
            hash: Some(PIC),
            ..CLIP
        };
        let off = PIC ^ 0b1111_1111_1111;
        let slightly = [hashed(file("b.png", AT + 900, 1280, 720), off)];
        let unread = [file("a.png", AT + 900, 1280, 720)];
        for files in [&slightly[..], &unread[..]] {
            assert!(pick_match(&clip, files).is_some(), "the lax match takes it");
            assert_eq!(pick_confirmed(&clip, files), None, "a delete never");
        }
        assert!(!confirmed(Some(PIC), Some(off)));
        assert!(confirmed(Some(PIC), Some(PIC ^ 0b11_1111_1111)));
    }

    #[test]
    fn an_ignored_preview_deletes_only_what_the_strict_worker_could_accept() {
        // Close, or the file's hash not read yet (the worker reads it again).
        assert!(may_confirm(Some(PIC), Some(PIC ^ 0b11_1111_1111)));
        assert!(may_confirm(Some(PIC), None));
        // Only the lax band, or no clipboard hash: the worker always refuses.
        assert!(!may_confirm(Some(PIC), Some(PIC ^ 0b1111_1111_1111)));
        assert!(!may_confirm(None, Some(PIC)));
        assert!(!may_confirm(None, None));
    }

    #[test]
    fn png_ihdr_valid_truncated_and_not_png() {
        assert_eq!(png_size(&png(1280, 720)), Some((1280, 720)));
        assert_eq!(png_size(&png(1280, 720)[..23]), None, "cut short");
        assert_eq!(png_size(&png(0, 720)), None);
        assert_eq!(png_size(b"GIF89a not a png at all......"), None);
        let mut wrong_chunk = png(10, 10);
        wrong_chunk[12..16].copy_from_slice(b"IDAT");
        assert_eq!(png_size(&wrong_chunk), None);
        assert_eq!(png_size(&[]), None);
    }

    #[test]
    fn jpeg_frame_header_after_exif() {
        assert_eq!(jpeg_size(&jpeg(1920, 1080)), Some((1920, 1080)));
        assert_eq!(image_size(&jpeg(640, 480)), Some((640, 480)));
        assert_eq!(jpeg_size(&jpeg(1920, 1080)[..12]), None);
        assert_eq!(jpeg_size(&png(4, 4)), None);
        assert_eq!(image_size(&png(4, 4)), Some((4, 4)));
    }

    #[test]
    fn matching_window_edges() {
        // Exactly at both edges matches; a millisecond past does not.
        let early = [file("a.png", AT - BEFORE_MS, 1280, 720)];
        assert!(pick_match(&CLIP, &early).is_some());
        let late = [file("b.png", AT + AFTER_MS, 1280, 720)];
        assert!(pick_match(&CLIP, &late).is_some());
        let too_early = [file("c.png", AT - BEFORE_MS - 1, 1280, 720)];
        assert_eq!(pick_match(&CLIP, &too_early), None);
        let too_late = [file("d.png", AT + AFTER_MS + 1, 1280, 720)];
        assert_eq!(pick_match(&CLIP, &too_late), None);
    }

    #[test]
    fn a_file_of_an_earlier_screenshot_is_never_this_ones() {
        // B is taken 2 s after A of the same window: A's file, written
        // before B arrived, must not be taken for B's (Hapus would recycle
        // it).
        let b = ClipCandidate {
            from_ms: AT,
            ..CLIP
        };
        let a_file = file("a.png", AT - 1_500, 1280, 720);
        assert_eq!(pick_match(&b, std::slice::from_ref(&a_file)), None);
        let b_file = file("b.png", AT + 700, 1280, 720);
        assert_eq!(
            pick_match(&b, &[a_file.clone(), b_file]).unwrap().path,
            PathBuf::from("b.png")
        );
        // With no screenshot just before, a file written first still counts
        // (PrtSc saves before the clipboard has it).
        assert!(pick_match(&CLIP, &[a_file]).is_some());
        // Where a screenshot's files may start: before it, unless the files
        // of the one before could reach that far.
        assert_eq!(claim_from(AT, None), AT - BEFORE_MS);
        assert_eq!(claim_from(AT, Some(AT - 2_000)), AT);
        assert_eq!(claim_from(AT, Some(AT - BEFORE_MS - AFTER_MS)), AT);
        assert_eq!(
            claim_from(AT, Some(AT - BEFORE_MS - AFTER_MS - 1)),
            AT - BEFORE_MS
        );
    }

    #[test]
    fn a_file_matched_by_another_screenshot_is_not_matched_again() {
        let dir = std::env::temp_dir().join(format!("vale-shot-taken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Screenshot 1.png");
        std::fs::write(&path, png(1280, 720)).unwrap();
        let now = unix_ms(SystemTime::now());
        let clip = ClipCandidate {
            at_ms: now,
            from_ms: now - BEFORE_MS,
            to_ms: now + AFTER_MS,
            ..CLIP
        };
        assert!(find_in(&dir, &clip, &[], |_| Some(PIC)).is_some());
        assert_eq!(find_in(&dir, &clip, &[path], |_| Some(PIC)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_different_size_is_never_a_match() {
        let files = [
            file("a.png", AT + 400, 1280, 721),
            file("b.png", AT + 400, 720, 1280),
        ];
        assert_eq!(pick_match(&CLIP, &files), None);
    }

    #[test]
    fn the_closest_in_time_wins() {
        let files = [
            file("far.png", AT + 4_000, 1280, 720),
            file("near.png", AT + 600, 1280, 720),
            file("other-size.png", AT + 10, 640, 480),
            file("before.png", AT - 2_000, 1280, 720),
        ];
        let m = pick_match(&CLIP, &files).unwrap();
        assert_eq!(m.path, PathBuf::from("near.png"));
        // A tie goes to the later file.
        let tie = [
            file("prev.png", AT - 500, 1280, 720),
            file("next.png", AT + 500, 1280, 720),
        ];
        assert_eq!(
            pick_match(&CLIP, &tie).unwrap().path,
            PathBuf::from("next.png")
        );
    }

    #[test]
    fn an_empty_folder_matches_nothing() {
        assert_eq!(pick_match(&CLIP, &[]), None);
    }

    #[test]
    fn empty_only_while_the_clipboard_is_unchanged() {
        assert!(should_empty(41, 41));
        assert!(!should_empty(42, 41), "something was copied since");
        assert!(!should_empty(0, 41));
    }

    #[test]
    fn folder_labels_name_the_parent_too() {
        assert_eq!(
            folder_label(Path::new(r"C:\Users\vale\Pictures\Screenshots")),
            r"Pictures\Screenshots"
        );
        assert_eq!(
            folder_label(Path::new(r"C:\Users\vale\OneDrive\Gambar\Tangkapan Layar")),
            r"Gambar\Tangkapan Layar"
        );
        assert_eq!(folder_label(Path::new(r"D:\Shots")), "Shots");
    }

    /// A folder of its own under target/test-tmp/, emptied first.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_listing_keeps_new_images_only() {
        let dir = temp_dir("shot-list");
        std::fs::write(dir.join("Screenshot 1.png"), png(1280, 720)).unwrap();
        std::fs::write(dir.join("Photo.JPG"), jpeg(1280, 720)).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not an image").unwrap();
        std::fs::write(dir.join("broken.png"), b"\x89PNG").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("deep.png"), png(1280, 720)).unwrap();
        let now = unix_ms(SystemTime::now());
        // No text file, no broken header, nothing in a folder below.
        let mut names: Vec<String> = list_folder(&dir, now - 60_000, now + 60_000)
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["Photo.JPG", "Screenshot 1.png"]);
        let listed = list_folder(&dir, now - 60_000, now + 60_000);
        let shot = listed
            .iter()
            .find(|f| f.path.ends_with("Screenshot 1.png"))
            .unwrap();
        assert_eq!((shot.w, shot.h), (1280, 720));
        assert_eq!(shot.bytes, png(1280, 720).len() as u64);
        // Outside the window: nothing.
        assert!(list_folder(&dir, now - 600_000, now - 300_000).is_empty());
        // The match: one clipboard image, one file of its size.
        let clip = ClipCandidate {
            at_ms: now,
            from_ms: now - BEFORE_MS,
            to_ms: now + AFTER_MS,
            w: 1280,
            h: 720,
            hash: Some(PIC),
        };
        // The hasher sees only the files of the clipboard's size.
        let seen = std::cell::RefCell::new(Vec::new());
        let same = |p: &Path| {
            seen.borrow_mut().push(p.to_path_buf());
            Some(PIC)
        };
        assert!(find_in(&dir, &clip, &[], same).is_some());
        assert_eq!(seen.borrow().len(), 2);
        // Another picture: nothing, though time and size agree.
        assert_eq!(find_in(&dir, &clip, &[], |_| Some(!PIC)), None);
        let other = ClipCandidate { w: 800, ..clip };
        assert_eq!(find_in(&dir, &other, &[], |_| Some(PIC)), None);
        assert!(list_folder(&dir.join("missing"), 0, i64::MAX).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// "Screenshot 2026-10-28 143012.png", the way Windows names them, with " (2)",
/// " (3)" ... before the extension while `exists` says a name is taken: a
/// saved screenshot never overwrites another file.
pub fn save_name(now: chrono::NaiveDateTime, exists: impl Fn(&str) -> bool) -> String {
    let stem = format!("Screenshot {}", now.format("%Y-%m-%d %H%M%S"));
    let first = format!("{stem}.png");
    if !exists(&first) {
        return first;
    }
    (2..1000)
        .map(|n| format!("{stem} ({n}).png"))
        .find(|name| !exists(name))
        .unwrap_or_else(|| format!("{stem} (1000).png"))
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use chrono::NaiveDate;

    fn at() -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 28)
            .and_then(|d| d.and_hms_opt(14, 30, 12))
            .expect("valid time")
    }

    #[test]
    fn the_name_follows_windows_and_never_overwrites() {
        assert_eq!(
            save_name(at(), |_| false),
            "Screenshot 2026-10-28 143012.png"
        );
        let taken = |n: &str| n == "Screenshot 2026-10-28 143012.png";
        assert_eq!(
            save_name(at(), taken),
            "Screenshot 2026-10-28 143012 (2).png"
        );
        let two = |n: &str| {
            n == "Screenshot 2026-10-28 143012.png" || n == "Screenshot 2026-10-28 143012 (2).png"
        };
        assert_eq!(save_name(at(), two), "Screenshot 2026-10-28 143012 (3).png");
    }
}
