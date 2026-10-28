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

/// The new clipboard image, as far as matching cares: when it arrived
/// (unix ms) and its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipCandidate {
    pub at_ms: i64,
    pub w: u32,
    pub h: u32,
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
}

/// The file that is this screenshot: inside `BEFORE_MS` .. `AFTER_MS` of
/// the clipboard's arrival with the very same size, the closest in time
/// winning (the later one on a tie). A different size or a time outside the
/// window is no match: never a guess.
pub fn pick_match<'a>(clip: &ClipCandidate, files: &'a [FileCandidate]) -> Option<&'a FileCandidate> {
    let (from, to) = (clip.at_ms - BEFORE_MS, clip.at_ms + AFTER_MS);
    files
        .iter()
        .filter(|f| (from..=to).contains(&f.at_ms) && (f.w, f.h) == (clip.w, clip.h))
        .min_by_key(|f| ((f.at_ms - clip.at_ms).abs(), std::cmp::Reverse(f.at_ms)))
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
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg"].iter().any(|x| e.eq_ignore_ascii_case(x)))
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
        let read = std::fs::File::open(&path)
            .and_then(|f| f.take(HEADER_BYTES).read_to_end(&mut head));
        let Some((w, h)) = read.ok().and_then(|_| image_size(&head)) else {
            continue;
        };
        out.push(FileCandidate {
            path,
            at_ms,
            bytes: meta.len(),
            w,
            h,
        });
    }
    out
}

/// One folder check for `clip`: the listing of its window, then the match.
pub fn find_in(dir: &Path, clip: &ClipCandidate) -> Option<FileCandidate> {
    let files = list_folder(dir, clip.at_ms - BEFORE_MS, clip.at_ms + AFTER_MS);
    pick_match(clip, &files).cloned()
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
        }
    }

    const AT: i64 = 1_800_000_000_000;
    const CLIP: ClipCandidate = ClipCandidate {
        at_ms: AT,
        w: 1280,
        h: 720,
    };

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
        assert_eq!(pick_match(&CLIP, &tie).unwrap().path, PathBuf::from("next.png"));
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
            w: 1280,
            h: 720,
        };
        assert!(find_in(&dir, &clip).is_some());
        let other = ClipCandidate { w: 800, ..clip };
        assert_eq!(find_in(&dir, &other), None);
        assert!(list_folder(&dir.join("missing"), 0, i64::MAX).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
