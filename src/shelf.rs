//! The file shelf: a temporary place to leave files on the island and take
//! them out again later. It keeps paths only, never copies, and only in
//! memory: a restart empties it. The list and the card data are pure; the
//! OLE drop target and drag-out are in `shelf_ole`.

use std::path::{Path, PathBuf};

use crate::ui::icons::Icon;

/// Files on the shelf at most (three rows of four cards).
pub const MAX_ITEMS: usize = 12;
/// A card's name, in characters, before it is cut short.
pub const NAME_MAX: usize = 14;

/// What a card's icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShelfKind {
    Folder,
    Image,
    /// Sound and video.
    Media,
    Code,
    Archive,
    /// Everything else.
    Document,
}

/// The icon kind for a path: a folder, or by extension.
pub fn kind_of(path: &Path, is_dir: bool) -> ShelfKind {
    if is_dir {
        return ShelfKind::Folder;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "tif" | "tiff" | "ico"
        | "heic" | "avif" => ShelfKind::Image,
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "mp4" | "mkv" | "mov" | "avi"
        | "webm" | "wmv" => ShelfKind::Media,
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "cpp" | "h" | "cs" | "java" | "go"
        | "html" | "css" | "json" | "xml" | "yaml" | "yml" | "toml" | "ps1" | "sh" | "bat"
        | "sql" => ShelfKind::Code,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab" => ShelfKind::Archive,
        _ => ShelfKind::Document,
    }
}

/// A card's icon.
pub fn icon(kind: ShelfKind) -> Icon {
    match kind {
        ShelfKind::Folder => Icon::Folder,
        ShelfKind::Image => Icon::Image,
        ShelfKind::Media => Icon::Music,
        ShelfKind::Code => Icon::Code,
        ShelfKind::Archive => Icon::Package,
        ShelfKind::Document => Icon::Note,
    }
}

/// What the file system says about a shelf path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Missing,
    File,
    Folder,
}

/// The real file system's answer (for `Shelf::cards`).
pub fn probe(path: &Path) -> Probe {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => Probe::Folder,
        Ok(_) => Probe::File,
        Err(_) => Probe::Missing,
    }
}

/// One card as it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfCard {
    /// The file's name, cut to `NAME_MAX` characters.
    pub name: String,
    pub kind: ShelfKind,
    /// The path is gone: the card is dimmed and cannot be dragged.
    pub stale: bool,
}

/// The name a card shows for `path`: its last part, or the whole path for
/// a drive root ("C:\").
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `name` cut to `max` characters with an ellipsis, keeping a short
/// extension: "kemang-denah-lantai.pdf" -> "kemang-de….pdf".
pub fn short_name(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return name.to_owned();
    }
    let ext_len = name
        .rfind('.')
        .map(|i| name[i..].chars().count())
        .filter(|&n| n > 1 && n <= 6 && n + 2 < max)
        .unwrap_or(0);
    let head = max - ext_len - 1;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - ext_len..]);
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shelf {
    items: Vec<PathBuf>,
}

impl Shelf {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Path> {
        self.items.get(i).map(PathBuf::as_path)
    }

    /// Puts `paths` on the shelf, newest last: a path already there (the
    /// same name in any letter case, as Windows sees it) and anything past
    /// `MAX_ITEMS` is left out. Returns how many were added.
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut added = 0;
        for path in paths {
            if self.items.len() >= MAX_ITEMS {
                break;
            }
            let key = key_of(&path);
            if key.is_empty() || self.items.iter().any(|p| key_of(p) == key) {
                continue;
            }
            self.items.push(path);
            added += 1;
        }
        added
    }

    pub fn remove(&mut self, i: usize) -> bool {
        if i < self.items.len() {
            self.items.remove(i);
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) -> bool {
        let had = !self.items.is_empty();
        self.items.clear();
        had
    }

    /// The cards, with `probe` telling what each path is now. A missing
    /// path is stale; its kind then comes from its extension alone.
    pub fn cards(&self, probe: impl Fn(&Path) -> Probe) -> Vec<ShelfCard> {
        self.items
            .iter()
            .map(|p| {
                let found = probe(p);
                ShelfCard {
                    name: short_name(&display_name(p), NAME_MAX),
                    kind: kind_of(p, found == Probe::Folder),
                    stale: found == Probe::Missing,
                }
            })
            .collect()
    }
}

/// How two paths are compared: Windows ignores letter case.
fn key_of(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn paths_are_added_once_in_any_letter_case_and_up_to_the_cap() {
        let mut s = Shelf::default();
        assert_eq!(s.add([p(r"C:\a\Denah.pdf"), p(r"c:\A\denah.PDF"), p(r"C:\b")]), 2);
        // A trailing slash is the same folder.
        assert_eq!(s.add([p(r"C:\b\")]), 0);
        assert_eq!(s.len(), 2);
        assert_eq!(s.add([p("")]), 0, "an empty path is nothing");
        let many: Vec<PathBuf> = (0..20).map(|i| p(&format!(r"C:\f\{i}.txt"))).collect();
        assert_eq!(s.add(many), MAX_ITEMS - 2);
        assert_eq!(s.len(), MAX_ITEMS);
        assert_eq!(s.add([p(r"C:\one-more.txt")]), 0, "full");
        // The first ones stay, in order.
        assert_eq!(s.get(0), Some(Path::new(r"C:\a\Denah.pdf")));
    }

    #[test]
    fn remove_and_clear() {
        let mut s = Shelf::default();
        assert!(!s.clear());
        s.add([p(r"C:\a.txt"), p(r"C:\b.txt"), p(r"C:\c.txt")]);
        assert!(s.remove(1));
        assert!(!s.remove(5));
        assert_eq!(s.get(1), Some(Path::new(r"C:\c.txt")));
        assert!(s.clear());
        assert!(s.is_empty());
    }

    #[test]
    fn cards_mark_a_missing_path_stale_through_the_injected_probe() {
        let mut s = Shelf::default();
        s.add([p(r"C:\x\denah.pdf"), p(r"C:\x\foto.PNG"), p(r"C:\x\Proyek"), p(r"C:\x\hilang.zip")]);
        let probe = |path: &Path| match path.file_name().unwrap().to_str().unwrap() {
            "Proyek" => Probe::Folder,
            "hilang.zip" => Probe::Missing,
            _ => Probe::File,
        };
        let cards = s.cards(probe);
        let got: Vec<(&str, ShelfKind, bool)> = cards
            .iter()
            .map(|c| (c.name.as_str(), c.kind, c.stale))
            .collect();
        assert_eq!(
            got,
            [
                ("denah.pdf", ShelfKind::Document, false),
                ("foto.PNG", ShelfKind::Image, false),
                ("Proyek", ShelfKind::Folder, false),
                ("hilang.zip", ShelfKind::Archive, true),
            ]
        );
    }

    #[test]
    fn the_icon_follows_the_extension_in_any_case() {
        let kind = |name: &str| kind_of(Path::new(name), false);
        assert_eq!(kind("a.JPG"), ShelfKind::Image);
        assert_eq!(kind("a.mp3"), ShelfKind::Media);
        assert_eq!(kind("a.rs"), ShelfKind::Code);
        assert_eq!(kind("a.7z"), ShelfKind::Archive);
        assert_eq!(kind("a.xlsx"), ShelfKind::Document);
        assert_eq!(kind("noext"), ShelfKind::Document);
        assert_eq!(kind_of(Path::new("a.jpg"), true), ShelfKind::Folder);
    }

    #[test]
    fn names_are_cut_short_keeping_a_short_extension() {
        assert_eq!(short_name("denah.pdf", 14), "denah.pdf");
        let cut = short_name("kemang-denah-lantai-dua.pdf", 14);
        assert_eq!(cut, "kemang-de….pdf");
        assert_eq!(cut.chars().count(), 14);
        // A long extension is not kept; no extension at all.
        let odd = short_name("arsip.sangatpanjangekstensi", 14);
        assert_eq!(odd.chars().count(), 14);
        assert!(odd.ends_with('…'));
        assert_eq!(short_name("abcdefghijklmnopqrstuvwxyz", 14), "abcdefghijklm…");
        // Multi-byte characters count as one.
        assert_eq!(short_name("ééééééééééééééééé", 14).chars().count(), 14);
        // A drive root has no file name: its path shows.
        assert_eq!(display_name(Path::new(r"C:\")), r"C:\");
    }
}
