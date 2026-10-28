//! Where the CLI page starts and the folder menu's rows: favourite and
//! recently used folders (kept in `%APPDATA%\Vale`), whether to
//! follow the last active File Explorer window when the page opens, and the
//! menu built from those lists. Pure: no COM and no Win32, so all of it is
//! tested without a window. The Explorer windows themselves are read by
//! `sources::explorer`.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cli::write_atomic;

/// Recently used folders kept (and shown) at most.
pub const RECENT_CAP: usize = 5;
/// Favourites kept at most: more would not fit any menu.
pub const FAVORITES_CAP: usize = 20;

/// Where the CLI page starts when it opens (`config.cli_start`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CliStart {
    /// The folder of the last active File Explorer window, when there is one.
    #[default]
    Explorer,
    /// The folder `cd`, a menu pick or the last command used (`cli-cwd.txt`).
    Last,
}

impl CliStart {
    pub const ALL: [CliStart; 2] = [CliStart::Explorer, CliStart::Last];

    /// Its name in config.json.
    pub fn name(self) -> &'static str {
        match self {
            CliStart::Explorer => "Explorer",
            CliStart::Last => "Last",
        }
    }

    /// Case-insensitive.
    pub fn from_name(name: &str) -> Option<CliStart> {
        CliStart::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }
}

impl Serialize for CliStart {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// An unknown name (a typo in config.json) is the default rather than an
/// invalid config.
impl<'de> Deserialize<'de> for CliStart {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Ok(CliStart::from_name(&name).unwrap_or_default())
    }
}

/// `%APPDATA%\Vale\cli-favorites.txt`: one folder per line.
pub fn favorites_path() -> PathBuf {
    crate::config::path().with_file_name("cli-favorites.txt")
}

/// `%APPDATA%\Vale\cli-recent.txt`: `<unix secs>\t<folder>` per
/// line, newest first.
pub fn recent_path() -> PathBuf {
    crate::config::path().with_file_name("cli-recent.txt")
}

/// `path` as it is kept: blanks around it trimmed, a trailing `\` or `/`
/// dropped except on a drive root (`C:\`). `None` when blank.
pub fn normalize(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    let mut text = text.trim();
    while text.len() > 1 && (text.ends_with('\\') || text.ends_with('/')) {
        let shorter = &text[..text.len() - 1];
        // `C:\` stays a root; `C:` alone would mean "the drive's current folder".
        if shorter.len() == 2 && shorter.ends_with(':') {
            break;
        }
        text = shorter;
    }
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// The same folder on Windows: compared normalised and without case.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    match (normalize(a), normalize(b)) {
        (Some(a), Some(b)) => {
            a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
        }
        _ => false,
    }
}

/// Which list a `FolderList` is: they differ in order, cap and file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// In the order they were starred, at most `FAVORITES_CAP`.
    Favorites,
    /// Newest first, at most `RECENT_CAP`, each with when it was used.
    Recent,
}

/// One folder of a list; `used` is when (unix seconds), for recent ones.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub used: Option<i64>,
}

/// Favourite or recently used folders, each once (case-insensitively).
#[derive(Clone, Debug, PartialEq)]
pub struct FolderList {
    kind: ListKind,
    entries: Vec<Entry>,
}

impl FolderList {
    pub fn new(kind: ListKind) -> Self {
        FolderList {
            kind,
            entries: Vec::new(),
        }
    }

    fn cap(&self) -> usize {
        match self.kind {
            ListKind::Favorites => FAVORITES_CAP,
            ListKind::Recent => RECENT_CAP,
        }
    }

    /// Bytes that are not UTF-8 become U+FFFD (as `History::load`); blank
    /// lines and repeats are skipped. A recent line without a timestamp
    /// (written by hand) still loads, without a time. A missing file is an
    /// empty list.
    pub fn load(kind: ListKind, path: &Path) -> Self {
        let mut list = FolderList::new(kind);
        let Ok(bytes) = std::fs::read(path) else {
            return list;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let (used, folder) = match line.split_once('\t') {
                Some((stamp, rest)) if stamp.trim().parse::<i64>().is_ok() => {
                    (stamp.trim().parse().ok(), rest)
                }
                _ => (None, line),
            };
            let Some(folder) = normalize(Path::new(folder)) else {
                continue;
            };
            if list.entries.len() < list.cap() && !list.contains(&folder) {
                let used = if kind == ListKind::Recent { used } else { None };
                list.entries.push(Entry { path: folder, used });
            }
        }
        list
    }

    /// Written atomically, like `cli-cwd.txt`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = String::new();
        for e in &self.entries {
            if let Some(at) = e.used {
                text.push_str(&format!("{at}\t"));
            }
            text.push_str(&e.path.to_string_lossy());
            text.push('\n');
        }
        write_atomic(path, &text)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.entries.iter().any(|e| same_folder(&e.path, path))
    }

    /// Recent: `path` was used at `now`; it moves (or comes) to the front
    /// and the oldest beyond the cap goes.
    pub fn push_recent(&mut self, path: &Path, now: i64) {
        let Some(path) = normalize(path) else { return };
        self.entries.retain(|e| !same_folder(&e.path, &path));
        self.entries.insert(
            0,
            Entry {
                path,
                used: Some(now),
            },
        );
        self.entries.truncate(self.cap());
    }

    /// Favourites: star (`on`) or unstar `path`. False when nothing changed
    /// (already so, a blank path, or the list is full).
    pub fn set_favorite(&mut self, path: &Path, on: bool) -> bool {
        let Some(path) = normalize(path) else {
            return false;
        };
        let had = self.contains(&path);
        if on {
            if had || self.entries.len() >= self.cap() {
                return false;
            }
            self.entries.push(Entry { path, used: None });
        } else {
            if !had {
                return false;
            }
            self.entries.retain(|e| !same_folder(&e.path, &path));
        }
        true
    }
}

/// The folder to switch to as the CLI page opens, or `None` to stay: never
/// with "Folder terakhir", while a command runs, or once the user picked a
/// folder this visit (`pinned`: a manual `cd` or a menu pick wins until the
/// panel closes); not without an Explorer folder, nor when the newest one
/// (`explorer[0]`, the last active window) is where the CLI already is.
pub fn choose_on_open(
    mode: CliStart,
    running: bool,
    pinned: bool,
    explorer: &[PathBuf],
    current: &Path,
) -> Option<PathBuf> {
    if mode == CliStart::Last || running || pinned {
        return None;
    }
    let first = explorer.first()?;
    (!same_folder(first, current)).then(|| first.clone())
}

/// The menu's sections, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// "Jendela Explorer terbuka": the windows, the last active first.
    Explorer,
    /// "Favorit": starred folders; right-click one to unstar it.
    Favorites,
    /// "Terakhir dipakai di CLI".
    Recent,
}

/// How long ago a recent folder was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ago {
    /// Under a minute (or a clock that went back).
    JustNow,
    Minutes(i64),
    /// Earlier today.
    Hours(i64),
    Yesterday,
    On(NaiveDate),
}

/// The small text at a row's right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowNote {
    None,
    /// "aktif terakhir": the first Explorer window.
    LastActive,
    /// "klik kanan = lepas": on the first favourite.
    Unpin,
    Ago(Ago),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuRow {
    pub path: PathBuf,
    pub note: RowNote,
    /// Starred: its star shows filled.
    pub favorite: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuSection {
    pub section: Section,
    pub rows: Vec<MenuRow>,
}

/// `then` relative to `now`, both local.
pub fn ago(then: NaiveDateTime, now: NaiveDateTime) -> Ago {
    let secs = now.signed_duration_since(then).num_seconds();
    if secs < 60 {
        Ago::JustNow
    } else if secs < 3600 {
        Ago::Minutes(secs / 60)
    } else if then.date() == now.date() {
        Ago::Hours(secs / 3600)
    } else if now.date().pred_opt() == Some(then.date()) {
        Ago::Yesterday
    } else {
        Ago::On(then.date())
    }
}

/// Unix seconds as local time.
pub fn local_time(unix: i64) -> Option<NaiveDateTime> {
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map(|t| t.naive_local())
}

/// The menu: the Explorer windows (the first one marked "aktif terakhir"),
/// the favourites (the first saying how to unstar), then the recent ones
/// that are not favourites, each with how long ago. Favourites and recent
/// folders that `exists` says are gone are left out (they stay in their
/// files). Sections without rows are left out; with none at all the menu
/// shows `MenuItem::Empty`.
pub fn menu_rows(
    explorer: &[PathBuf],
    favorites: &FolderList,
    recent: &FolderList,
    now: NaiveDateTime,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<MenuSection> {
    let mut out = Vec::new();
    let mut seen: Vec<&Path> = Vec::new();
    let mut windows = Vec::new();
    for path in explorer {
        if seen.iter().any(|s| same_folder(s, path)) {
            continue;
        }
        seen.push(path);
        windows.push(MenuRow {
            path: path.clone(),
            note: if windows.is_empty() {
                RowNote::LastActive
            } else {
                RowNote::None
            },
            favorite: favorites.contains(path),
        });
    }
    let starred: Vec<MenuRow> = favorites
        .entries()
        .iter()
        .filter(|e| exists(&e.path))
        .enumerate()
        .map(|(i, e)| MenuRow {
            path: e.path.clone(),
            note: if i == 0 {
                RowNote::Unpin
            } else {
                RowNote::None
            },
            favorite: true,
        })
        .collect();
    let used: Vec<MenuRow> = recent
        .entries()
        .iter()
        .filter(|e| !favorites.contains(&e.path) && exists(&e.path))
        .take(RECENT_CAP)
        .map(|e| MenuRow {
            path: e.path.clone(),
            note: e
                .used
                .and_then(local_time)
                .map_or(RowNote::None, |then| RowNote::Ago(ago(then, now))),
            favorite: false,
        })
        .collect();
    for (section, rows) in [
        (Section::Explorer, windows),
        (Section::Favorites, starred),
        (Section::Recent, used),
    ] {
        if !rows.is_empty() {
            out.push(MenuSection { section, rows });
        }
    }
    out
}

/// One line of the menu as drawn, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    Header(Section),
    Folder {
        section: Section,
        row: MenuRow,
    },
    /// "Belum ada folder": nothing in any section.
    Empty,
}

/// The sections as the menu's lines: each header, then its rows.
pub fn menu_items(sections: &[MenuSection]) -> Vec<MenuItem> {
    if sections.is_empty() {
        return vec![MenuItem::Empty];
    }
    let mut out = Vec::new();
    for s in sections {
        out.push(MenuItem::Header(s.section));
        out.extend(s.rows.iter().map(|row| MenuItem::Folder {
            section: s.section,
            row: row.clone(),
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "vale-folders-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn paths(list: &FolderList) -> Vec<String> {
        list.entries()
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect()
    }

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn paths_normalise_and_compare_without_case() {
        assert_eq!(normalize(Path::new(r"D:\Proyek\")), Some(p(r"D:\Proyek")));
        assert_eq!(normalize(Path::new(r"  D:\a\\ ")), Some(p(r"D:\a")));
        assert_eq!(normalize(Path::new(r"C:\")), Some(p(r"C:\")));
        assert_eq!(normalize(Path::new(r"C:\\")), Some(p(r"C:\")));
        assert_eq!(normalize(Path::new("  ")), None);
        assert!(same_folder(
            Path::new(r"d:\PROYEK\"),
            Path::new(r"D:\Proyek")
        ));
        assert!(!same_folder(
            Path::new(r"D:\Proyek"),
            Path::new(r"D:\Proyek2")
        ));
        assert!(!same_folder(Path::new(""), Path::new("")));
    }

    #[test]
    fn recent_is_newest_first_once_and_capped_at_five() {
        let mut r = FolderList::new(ListKind::Recent);
        for (i, f) in ["a", "b", "c", "B\\", "d", "e", "f"].iter().enumerate() {
            r.push_recent(&p(&format!(r"D:\{f}")), 100 + i as i64);
        }
        assert_eq!(paths(&r), [r"D:\f", r"D:\e", r"D:\d", r"D:\B", r"D:\c"]);
        assert_eq!(r.entries()[0].used, Some(106));
        assert_eq!(r.entries()[3].used, Some(103), "the repeat moved up");
        r.push_recent(Path::new(" "), 200);
        assert_eq!(r.entries().len(), RECENT_CAP);
    }

    #[test]
    fn favorites_keep_their_order_and_toggle() {
        let mut f = FolderList::new(ListKind::Favorites);
        assert!(f.set_favorite(&p(r"D:\b"), true));
        assert!(f.set_favorite(&p(r"D:\a\"), true));
        assert!(!f.set_favorite(&p(r"d:\B"), true), "already starred");
        assert_eq!(paths(&f), [r"D:\b", r"D:\a"]);
        assert!(f.contains(&p(r"D:\A")));
        assert!(f.set_favorite(&p(r"D:\B\"), false));
        assert!(!f.set_favorite(&p(r"D:\b"), false), "already gone");
        assert_eq!(paths(&f), [r"D:\a"]);
        for i in 0..40 {
            f.set_favorite(&p(&format!(r"D:\x{i}")), true);
        }
        assert_eq!(f.entries().len(), FAVORITES_CAP);
        assert!(!f.set_favorite(&p(r"D:\one-more"), true), "full");
    }

    #[test]
    fn lists_round_trip_through_their_files() {
        let dir = TempDir::new("round-trip");
        let fav = dir.0.join("sub").join("cli-favorites.txt");
        let rec = dir.0.join("cli-recent.txt");
        assert_eq!(
            FolderList::load(ListKind::Favorites, &fav),
            FolderList::new(ListKind::Favorites)
        );
        let mut f = FolderList::new(ListKind::Favorites);
        f.set_favorite(&p(r"D:\github\dynamic-island"), true);
        f.set_favorite(&p(r"C:\Users\hartono\Kerja ✓"), true);
        f.save(&fav).unwrap();
        assert_eq!(FolderList::load(ListKind::Favorites, &fav), f);
        assert!(!dir.0.join("sub").join("cli-favorites.txt.tmp").exists());

        let mut r = FolderList::new(ListKind::Recent);
        r.push_recent(&p(r"D:\a"), 1_700_000_000);
        r.push_recent(&p(r"D:\b"), 1_700_000_500);
        r.save(&rec).unwrap();
        assert_eq!(
            std::fs::read_to_string(&rec).unwrap(),
            "1700000500\tD:\\b\n1700000000\tD:\\a\n"
        );
        assert_eq!(FolderList::load(ListKind::Recent, &rec), r);
    }

    #[test]
    fn hand_edited_files_load_cleaned_up() {
        let dir = TempDir::new("hand");
        let rec = dir.0.join("cli-recent.txt");
        // No timestamp, blank lines, repeats in another case, ANSI bytes.
        std::fs::write(
            &rec,
            b"D:\\no-time\r\n\n1700000000\tD:\\Kerja\\\n12\tD:\\KERJA\n\t\nD:\\caf\xE9\n",
        )
        .unwrap();
        let r = FolderList::load(ListKind::Recent, &rec);
        assert_eq!(paths(&r), [r"D:\no-time", r"D:\Kerja", "D:\\caf\u{FFFD}"]);
        assert_eq!(
            r.entries().iter().map(|e| e.used).collect::<Vec<_>>(),
            [None, Some(1_700_000_000), None]
        );
        // More than five lines: the newest five (the file's first) stay.
        let many: String = (0..9).map(|i| format!("{i}\tD:\\f{i}\n")).collect();
        std::fs::write(&rec, many).unwrap();
        let r = FolderList::load(ListKind::Recent, &rec);
        assert_eq!(
            paths(&r),
            [r"D:\f0", r"D:\f1", r"D:\f2", r"D:\f3", r"D:\f4"]
        );
        // A favourites file never keeps times.
        std::fs::write(&rec, "5\tD:\\x\n").unwrap();
        let f = FolderList::load(ListKind::Favorites, &rec);
        assert_eq!(
            f.entries(),
            [Entry {
                path: p(r"D:\x"),
                used: None
            }]
        );
    }

    #[test]
    fn start_folder_follows_explorer_only_when_it_may() {
        let explorer = [p(r"D:\github\project-a"), p(r"D:\other")];
        let here = p(r"C:\Users\hartono");
        let pick = |mode, running, pinned, list: &[PathBuf], cur: &Path| {
            choose_on_open(mode, running, pinned, list, cur)
        };
        // Follows the last active window.
        assert_eq!(
            pick(CliStart::Explorer, false, false, &explorer, &here),
            Some(p(r"D:\github\project-a"))
        );
        // "Folder terakhir" keeps cli-cwd.txt.
        assert_eq!(pick(CliStart::Last, false, false, &explorer, &here), None);
        // Never while a command runs.
        assert_eq!(
            pick(CliStart::Explorer, true, false, &explorer, &here),
            None
        );
        // A manual cd or a menu pick this visit wins.
        assert_eq!(
            pick(CliStart::Explorer, false, true, &explorer, &here),
            None
        );
        // Already there (any case, trailing slash): nothing to do.
        let same = p(r"d:\GITHUB\project-a\");
        assert_eq!(
            pick(CliStart::Explorer, false, false, &explorer, &same),
            None
        );
        // No Explorer window.
        assert_eq!(pick(CliStart::Explorer, false, false, &[], &here), None);
    }

    #[test]
    fn start_mode_is_a_name_and_a_typo_is_explorer() {
        assert_eq!(CliStart::default(), CliStart::Explorer);
        for s in CliStart::ALL {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<CliStart>(&json).unwrap(), s);
        }
        assert_eq!(serde_json::to_string(&CliStart::Last).unwrap(), "\"Last\"");
        assert_eq!(
            serde_json::from_str::<CliStart>("\"last\"").unwrap(),
            CliStart::Last
        );
        assert_eq!(
            serde_json::from_str::<CliStart>("\"terakhir\"").unwrap(),
            CliStart::Explorer
        );
    }

    #[test]
    fn relative_times() {
        let now = at(27, 14, 32);
        assert_eq!(ago(at(27, 14, 32), now), Ago::JustNow);
        assert_eq!(ago(at(27, 14, 40), now), Ago::JustNow, "clock went back");
        assert_eq!(ago(at(27, 14, 30), now), Ago::Minutes(2));
        assert_eq!(ago(at(27, 13, 33), now), Ago::Minutes(59));
        assert_eq!(ago(at(27, 9, 0), now), Ago::Hours(5));
        assert_eq!(ago(at(26, 23, 50), now), Ago::Yesterday);
        assert_eq!(
            ago(at(20, 9, 0), now),
            Ago::On(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap())
        );
    }

    fn stamp(t: NaiveDateTime) -> i64 {
        Local
            .from_local_datetime(&t)
            .earliest()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn menu_has_explorer_favorites_and_recent() {
        let now = at(27, 14, 32);
        let explorer = [
            p(r"D:\github\project-a"),
            p(r"D:\Proyek\rumah-kemang\gambar"),
            p(r"d:\GITHUB\project-a"),
        ];
        let mut fav = FolderList::new(ListKind::Favorites);
        fav.set_favorite(&p(r"D:\github\dynamic-island"), true);
        fav.set_favorite(&p(r"D:\gone"), true);
        fav.set_favorite(&p(r"D:\Proyek\rumah-kemang\gambar"), true);
        let mut rec = FolderList::new(ListKind::Recent);
        rec.push_recent(&p(r"C:\Users\hartono"), stamp(at(26, 18, 0)));
        rec.push_recent(&p(r"D:\gone-too"), stamp(at(27, 10, 0)));
        rec.push_recent(&p(r"D:\github\dynamic-island"), stamp(at(27, 14, 0)));
        rec.push_recent(&p(r"D:\Proyek"), stamp(at(27, 14, 30)));
        let exists = |path: &Path| !path.to_string_lossy().contains("gone");
        let menu = menu_rows(&explorer, &fav, &rec, now, &exists);
        let sections: Vec<Section> = menu.iter().map(|s| s.section).collect();
        assert_eq!(
            sections,
            [Section::Explorer, Section::Favorites, Section::Recent]
        );
        // Explorer: z-order, each folder once, the first one last active.
        let w = &menu[0].rows;
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].note, w[0].favorite, w[1].note, w[1].favorite),
            (RowNote::LastActive, false, RowNote::None, true)
        );
        // Favourites: in order, gone ones hidden, the hint on the first.
        let f: Vec<(&Path, RowNote)> = menu[1]
            .rows
            .iter()
            .map(|r| (r.path.as_path(), r.note))
            .collect();
        assert_eq!(
            f,
            [
                (Path::new(r"D:\github\dynamic-island"), RowNote::Unpin),
                (Path::new(r"D:\Proyek\rumah-kemang\gambar"), RowNote::None)
            ]
        );
        assert!(menu[1].rows.iter().all(|r| r.favorite));
        // Recent: no favourite repeated, gone ones hidden, times relative.
        let r: Vec<(&Path, RowNote)> = menu[2]
            .rows
            .iter()
            .map(|r| (r.path.as_path(), r.note))
            .collect();
        assert_eq!(
            r,
            [
                (Path::new(r"D:\Proyek"), RowNote::Ago(Ago::Minutes(2))),
                (Path::new(r"C:\Users\hartono"), RowNote::Ago(Ago::Yesterday)),
            ]
        );
        // As lines: header, rows, next header...
        let items = menu_items(&menu);
        assert_eq!(items.len(), 3 + 2 + 2 + 2);
        assert_eq!(items[0], MenuItem::Header(Section::Explorer));
        assert_eq!(items[3], MenuItem::Header(Section::Favorites));
        assert!(matches!(
            &items[8],
            MenuItem::Folder { section: Section::Recent, row } if row.path == p(r"C:\Users\hartono")
        ));
    }

    #[test]
    fn menu_leaves_out_empty_sections_and_says_when_there_is_nothing() {
        let now = at(27, 14, 32);
        let fav = FolderList::new(ListKind::Favorites);
        let mut rec = FolderList::new(ListKind::Recent);
        let all = |_: &Path| true;
        assert!(menu_rows(&[], &fav, &rec, now, &all).is_empty());
        assert_eq!(menu_items(&[]), [MenuItem::Empty]);
        // A recent line without a time has no note.
        rec.entries.push(Entry {
            path: p(r"D:\a"),
            used: None,
        });
        let menu = menu_rows(&[], &fav, &rec, now, &all);
        assert_eq!(menu.len(), 1);
        assert_eq!(menu[0].section, Section::Recent);
        assert_eq!(menu[0].rows[0].note, RowNote::None);
        // Only Explorer windows.
        let menu = menu_rows(
            &[p(r"D:\x")],
            &fav,
            &FolderList::new(ListKind::Recent),
            now,
            &all,
        );
        assert_eq!(menu.len(), 1);
        assert_eq!(menu[0].section, Section::Explorer);
    }
}
