//! Which tiles the Pintasan page shows, in what order: the "Pintasan" setting.
//! The nine built-in tiles (Search is the ninth, since 3.2.1) are always
//! listed once each (one that is off is hidden, not lost); custom tiles (an
//! app, a folder, a web address) are the owner's own and can be removed.
//! At most twelve show: a tile cannot be switched on while twelve already
//! are, and a built-in a saved list does not name rejoins switched off when
//! the page is full. Names in the file are read without regard to case.

use serde::{Deserialize, Serialize};

use crate::model::Shortcut;

/// Tiles shown at once: four columns, three rows.
pub const MAX_SHOWN: usize = 12;
/// Custom tiles kept in the list.
pub const MAX_CUSTOM: usize = 8;
/// A custom tile's label, in characters (it sits under an icon).
pub const LABEL_MAX: usize = 14;
/// A custom tile's target (a path or an address), in characters.
pub const TARGET_MAX: usize = 500;

/// The `tile` name a custom entry carries in `config.json`.
const CUSTOM: &str = "Custom";

/// What a custom tile opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomKind {
    /// A program or a file, by its path.
    App,
    /// A folder, by its path.
    Folder,
    /// A web address (http, https) or a mail link (mailto).
    Url,
}

impl CustomKind {
    pub const ALL: [CustomKind; 3] = [CustomKind::App, CustomKind::Folder, CustomKind::Url];

    fn name(self) -> &'static str {
        match self {
            CustomKind::App => "App",
            CustomKind::Folder => "Folder",
            CustomKind::Url => "Url",
        }
    }

    fn from_name(name: &str) -> Option<CustomKind> {
        Self::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(name.trim()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomTile {
    pub label: String,
    pub kind: CustomKind,
    pub target: String,
}

impl CustomTile {
    /// A tile from what the owner typed: both parts trimmed, within their
    /// limits and free of control characters, a web address with a safe
    /// scheme, and an app or folder as an absolute local path. `None` when
    /// any of that fails: nothing is cut off to make it pass.
    pub fn new(label: &str, kind: CustomKind, target: &str) -> Option<CustomTile> {
        let (label, target) = (label.trim(), target.trim());
        let clean = |s: &str, max: usize| {
            !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
        };
        if !clean(label, LABEL_MAX) || !clean(target, TARGET_MAX) {
            return None;
        }
        let valid = match kind {
            CustomKind::Url => safe_url(target),
            CustomKind::App | CustomKind::Folder => local_path(target),
        };
        if !valid {
            return None;
        }
        Some(CustomTile {
            label: label.to_owned(),
            kind,
            target: target.to_owned(),
        })
    }
}

/// An address the shell may open from a tile: http(s) with a host, or a
/// mail link without a `?` query (which could fill in a body, copies or an
/// attachment). Anything else (file:, javascript:, ms-*: handlers, bare
/// words) is not a link this page launches.
fn safe_url(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    let host = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    match host {
        Some(rest) => rest.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()),
        None => lower
            .strip_prefix("mailto:")
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('?')),
    }
}

/// A path an app or folder tile may open: absolute and local, a drive
/// letter then `:\` ("C:\Program Files\…"), with no other `:` (no stream
/// name, no `scheme:` further on). Relative paths (looked up wherever the
/// shell looks), UNC shares (`\\server\…`), `\\?\` forms and addresses are
/// refused.
fn local_path(target: &str) -> bool {
    let b = target.as_bytes();
    b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'\\'
        && !target[2..].contains(':')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tile {
    Builtin(Shortcut),
    Custom(CustomTile),
}

/// One tile in `config.json`: a built-in by name (`{"tile":"Lock","on":false}`)
/// or a custom one (`{"tile":"Custom","label":"Gmail","kind":"Url","target":
/// "https://mail.google.com"}`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileEntry {
    pub tile: String,
    /// Absent means shown, for an entry written by hand.
    #[serde(default = "shown")]
    pub on: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
}

fn shown() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutTiles {
    entries: Vec<(Tile, bool)>,
}

impl Default for ShortcutTiles {
    fn default() -> Self {
        ShortcutTiles {
            entries: Shortcut::ALL
                .iter()
                .map(|s| (Tile::Builtin(*s), true))
                .collect(),
        }
    }
}

fn shortcut_name(s: Shortcut) -> &'static str {
    match s {
        Shortcut::Screenshot => "Screenshot",
        Shortcut::Clipboard => "Clipboard",
        Shortcut::Search => "Search",
        Shortcut::Emoji => "Emoji",
        Shortcut::Explorer => "Explorer",
        Shortcut::Calculator => "Calculator",
        Shortcut::TaskManager => "TaskManager",
        Shortcut::WindowsSettings => "WindowsSettings",
        Shortcut::Lock => "Lock",
    }
}

/// A built-in by its name in the file, in any case ("lock" is `Lock`).
fn shortcut_from_name(name: &str) -> Option<Shortcut> {
    Shortcut::ALL
        .into_iter()
        .find(|s| shortcut_name(*s).eq_ignore_ascii_case(name.trim()))
}

/// The entry names a custom tile ("Custom", in any case).
fn is_custom_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(CUSTOM)
}

impl ShortcutTiles {
    /// The saved list made whole: unknown names, repeated built-ins and
    /// custom entries that fail `CustomTile::new` are dropped (so are custom
    /// entries past `MAX_CUSTOM`), a built-in the file does not name (an
    /// older config) joins right after the one before it in `Shortcut::ALL`,
    /// and a list that shows nothing shows the built-ins.
    pub fn from_saved(saved: &[TileEntry]) -> Self {
        let mut entries: Vec<(Tile, bool)> = Vec::new();
        let mut customs = 0;
        for e in saved {
            if is_custom_name(&e.tile) {
                let tile = CustomKind::from_name(&e.kind)
                    .and_then(|kind| CustomTile::new(&e.label, kind, &e.target));
                if let Some(tile) = tile
                    && customs < MAX_CUSTOM
                {
                    customs += 1;
                    entries.push((Tile::Custom(tile), e.on));
                }
            } else if let Some(s) = shortcut_from_name(&e.tile)
                && !entries.iter().any(|(t, _)| *t == Tile::Builtin(s))
            {
                entries.push((Tile::Builtin(s), e.on));
            }
        }
        for (i, s) in Shortcut::ALL.iter().enumerate() {
            let tile = Tile::Builtin(*s);
            if entries.iter().any(|(t, _)| *t == tile) {
                continue;
            }
            let after = Shortcut::ALL[..i]
                .iter()
                .rev()
                .find_map(|prev| entries.iter().position(|(t, _)| *t == Tile::Builtin(*prev)));
            // A tile the file never named comes in shown, unless the page is
            // full already: it must not push the owner's own tile off it.
            let room = entries.iter().filter(|(_, on)| *on).count() < MAX_SHOWN;
            entries.insert(after.map_or(0, |a| a + 1), (tile, room));
        }
        if entries.iter().all(|(_, on)| !on) {
            for e in &mut entries {
                if matches!(e.0, Tile::Builtin(_)) {
                    e.1 = true;
                }
            }
        }
        ShortcutTiles { entries }
    }

    pub fn to_saved(&self) -> Vec<TileEntry> {
        self.entries
            .iter()
            .map(|(tile, on)| match tile {
                Tile::Builtin(s) => TileEntry {
                    tile: shortcut_name(*s).to_owned(),
                    on: *on,
                    ..TileEntry::default()
                },
                Tile::Custom(c) => TileEntry {
                    tile: CUSTOM.to_owned(),
                    on: *on,
                    label: c.label.clone(),
                    kind: c.kind.name().to_owned(),
                    target: c.target.clone(),
                },
            })
            .collect()
    }

    pub fn entries(&self) -> &[(Tile, bool)] {
        &self.entries
    }

    /// The tiles the page shows, in order, at most `MAX_SHOWN`.
    pub fn visible(&self) -> Vec<Tile> {
        self.entries
            .iter()
            .filter(|(_, on)| *on)
            .map(|(t, _)| t.clone())
            .take(MAX_SHOWN)
            .collect()
    }

    pub fn shown(&self) -> usize {
        self.entries.iter().filter(|(_, on)| *on).count()
    }

    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Only `add` and tests count.
    pub fn customs(&self) -> usize {
        self.entries
            .iter()
            .filter(|(t, _)| matches!(t, Tile::Custom(_)))
            .count()
    }

    /// Twelve tiles show already: no other can be switched on.
    pub fn full(&self) -> bool {
        self.shown() >= MAX_SHOWN
    }

    /// Entry `i` is switched on but past the twelfth, so the page does not
    /// show it (a list edited by hand can have more on than fit).
    pub fn beyond_limit(&self, i: usize) -> bool {
        let before = |i: usize| self.entries[..i].iter().filter(|(_, on)| *on).count();
        matches!(self.entries.get(i), Some((_, true))) && before(i) >= MAX_SHOWN
    }

    /// Flips entry `i`. The last tile still shown stays on, and a tile stays
    /// off while twelve show already.
    pub fn toggle(&mut self, i: usize) -> bool {
        let shown = self.shown();
        match self.entries.get_mut(i) {
            Some((_, on)) if *on && shown > 1 => *on = false,
            Some((_, on)) if !*on && shown < MAX_SHOWN => *on = true,
            _ => return false,
        }
        true
    }

    /// Moves entry `i` by `by` places; false when it is already at that end.
    pub fn move_by(&mut self, i: usize, by: i32) -> bool {
        let Some(j) = i.checked_add_signed(by as isize) else {
            return false;
        };
        if i >= self.entries.len() || j >= self.entries.len() || i == j {
            return false;
        }
        self.entries.swap(i, j);
        true
    }

    /// Removes entry `i` when it is a custom tile.
    pub fn remove(&mut self, i: usize) -> bool {
        if matches!(self.entries.get(i), Some((Tile::Custom(_), _))) {
            self.entries.remove(i);
            true
        } else {
            false
        }
    }

    /// Adds a custom tile at the end, on; false when the list is full.
    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Only the debug demo adds one.
    pub fn add(&mut self, tile: CustomTile) -> bool {
        if self.customs() >= MAX_CUSTOM {
            return false;
        }
        self.entries.push((Tile::Custom(tile), true));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(label: &str, kind: CustomKind, target: &str) -> CustomTile {
        CustomTile::new(label, kind, target).expect("valid")
    }

    fn entry(tile: &str, on: bool) -> TileEntry {
        TileEntry {
            tile: tile.into(),
            on,
            ..TileEntry::default()
        }
    }

    fn names(t: &ShortcutTiles) -> Vec<String> {
        t.entries()
            .iter()
            .map(|(tile, on)| {
                let name = match tile {
                    Tile::Builtin(s) => shortcut_name(*s).to_owned(),
                    Tile::Custom(c) => format!("+{}", c.label),
                };
                if *on { name } else { format!("({name})") }
            })
            .collect()
    }

    #[test]
    fn the_default_is_the_nine_built_ins_in_order() {
        let t = ShortcutTiles::default();
        let all: Vec<Tile> = Shortcut::ALL.iter().map(|s| Tile::Builtin(*s)).collect();
        assert_eq!(t.visible(), all);
        assert_eq!(ShortcutTiles::from_saved(&[]), t);
    }

    #[test]
    fn a_saved_list_keeps_its_order_and_gets_the_missing_built_ins_back() {
        let saved = [entry("Lock", true), entry("Emoji", false), entry("Bogus", true)];
        let t = ShortcutTiles::from_saved(&saved);
        // Each missing one lands right after the one before it in the
        // default order (Screenshot has none before it: the front).
        assert_eq!(
            names(&t),
            [
                "Screenshot",
                "Clipboard",
                "Search",
                "Lock",
                "(Emoji)",
                "Explorer",
                "Calculator",
                "TaskManager",
                "WindowsSettings"
            ]
        );
    }

    #[test]
    fn repeats_and_a_list_with_everything_off_are_made_whole() {
        let t = ShortcutTiles::from_saved(&[entry("Lock", true), entry("Lock", false)]);
        assert_eq!(t.entries().iter().filter(|(x, _)| *x == Tile::Builtin(Shortcut::Lock)).count(), 1);
        let all_off: Vec<TileEntry> = Shortcut::ALL
            .iter()
            .map(|s| entry(shortcut_name(*s), false))
            .collect();
        assert_eq!(ShortcutTiles::from_saved(&all_off).shown(), Shortcut::ALL.len());
    }

    #[test]
    fn custom_tiles_are_validated() {
        let ok = |label, kind, target| CustomTile::new(label, kind, target).is_some();
        assert!(ok("Gmail", CustomKind::Url, "https://mail.google.com"));
        assert!(ok("Surel", CustomKind::Url, "mailto:a@b.co"));
        assert!(ok("Proyek", CustomKind::Folder, r"D:\Proyek"));
        assert!(ok("Editor", CustomKind::App, r"C:\Program Files\Editor\editor.exe"));
        // Not a link this page launches.
        for bad in [
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:",
            "mail.google.com",
            "https://",
            "http:// x",
            "mailto:",
            "mailto:a@b.co?attach=C:\\secret.txt",
            "mailto:a@b.co?body=x",
        ] {
            assert!(!ok("X", CustomKind::Url, bad), "{bad}");
        }
        // An app or folder is an absolute local path, nothing else.
        assert!(ok("Akar", CustomKind::Folder, r"C:\"));
        assert!(ok("Proyek", CustomKind::Folder, r"d:\Proyek\2026"));
        for kind in [CustomKind::App, CustomKind::Folder] {
            for bad in [
                "a.exe",
                r"Proyek\a.exe",
                r"\Windows\notepad.exe",
                r"\\server\share\a.exe",
                r"\\?\C:\a.exe",
                "C:a.exe",
                "C:/Tools/a.exe",
                r"C:\a.txt:stream.exe",
                r"C:\x\https://evil",
                "file:///C:/a.exe",
                "https://example.com",
                "shell:startup",
                r"%APPDATA%\a.exe",
                r"1:\a.exe",
            ] {
                assert!(!ok("X", kind, bad), "{kind:?} {bad}");
            }
        }
        // Empty, control characters, too long.
        assert!(!ok("", CustomKind::App, r"C:\a.exe"));
        assert!(!ok("  ", CustomKind::App, r"C:\a.exe"));
        assert!(!ok("X", CustomKind::App, ""));
        assert!(!ok("X\n", CustomKind::App, "C:\\a.exe\0"));
        assert!(!ok("Ini label terlalu panjang", CustomKind::App, r"C:\a.exe"));
        let long = format!(r"C:\{}", "a".repeat(TARGET_MAX));
        assert!(!ok("X", CustomKind::App, &long));
        // Trimmed.
        let t = CustomTile::new("  Gmail ", CustomKind::Url, " https://x.co ").unwrap();
        assert_eq!((t.label.as_str(), t.target.as_str()), ("Gmail", "https://x.co"));
    }

    #[test]
    fn custom_entries_round_trip_and_bad_ones_are_dropped() {
        let mut t = ShortcutTiles::default();
        assert!(t.add(custom("Gmail", CustomKind::Url, "https://mail.google.com")));
        assert!(t.add(custom("Proyek", CustomKind::Folder, r"D:\Proyek")));
        let saved = t.to_saved();
        assert_eq!(ShortcutTiles::from_saved(&saved), t);
        let json = serde_json::to_string(&saved[Shortcut::ALL.len()]).unwrap();
        assert_eq!(
            json,
            r#"{"tile":"Custom","on":true,"label":"Gmail","kind":"Url","target":"https://mail.google.com"}"#
        );
        // Hand-edited into nonsense: those entries go, the rest stays.
        let mixed = [
            TileEntry {
                tile: CUSTOM.into(),
                on: true,
                label: "Buruk".into(),
                kind: "Url".into(),
                target: "javascript:1".into(),
            },
            TileEntry {
                tile: CUSTOM.into(),
                on: true,
                label: "Tanpa jenis".into(),
                kind: "Perintah".into(),
                target: "calc".into(),
            },
        ];
        assert_eq!(ShortcutTiles::from_saved(&mixed), ShortcutTiles::default());
    }

    #[test]
    fn the_list_and_the_shown_tiles_have_limits() {
        let mut t = ShortcutTiles::default();
        for i in 0..MAX_CUSTOM {
            assert!(t.add(custom(&format!("T{i}"), CustomKind::App, r"C:\a.exe")));
        }
        assert!(!t.add(custom("Lebih", CustomKind::App, r"C:\a.exe")), "full");
        // Nine built-in + 8 own, all on: only the first twelve show.
        assert_eq!(t.entries().len(), Shortcut::ALL.len() + MAX_CUSTOM);
        assert_eq!(t.visible().len(), MAX_SHOWN);
        // More customs in a file than fit: the extra are dropped.
        let many: Vec<TileEntry> = (0..MAX_CUSTOM + 3)
            .map(|i| TileEntry {
                tile: CUSTOM.into(),
                on: true,
                label: format!("T{i}"),
                kind: "App".into(),
                target: r"C:\a.exe".into(),
            })
            .collect();
        assert_eq!(ShortcutTiles::from_saved(&many).customs(), MAX_CUSTOM);
    }

    #[test]
    fn a_new_built_in_never_pushes_an_own_tile_off_a_full_page() {
        // A saved list of 8 built-ins and 4 own tiles, all on (12 shown),
        // from before "Search" existed.
        let mut saved: Vec<TileEntry> = Shortcut::ALL
            .iter()
            .filter(|s| shortcut_name(**s) != "Search")
            .map(|s| entry(shortcut_name(*s), true))
            .collect();
        for i in 0..4 {
            saved.push(TileEntry {
                tile: CUSTOM.into(),
                on: true,
                label: format!("T{i}"),
                kind: "App".into(),
                target: r"C:\a.exe".into(),
            });
        }
        let t = ShortcutTiles::from_saved(&saved);
        assert_eq!(t.entries().len(), 13);
        assert_eq!(t.visible().len(), MAX_SHOWN);
        assert!(
            t.visible().iter().filter(|x| matches!(x, Tile::Custom(_))).count() == 4,
            "every own tile still shows"
        );
        assert!(t.entries().iter().any(|(x, on)| *x == Tile::Builtin(Shortcut::Search) && !on));
        // With room, it comes in shown; a fresh install shows all.
        assert_eq!(ShortcutTiles::from_saved(&saved[..9]).shown(), 10, "8 + 1 own + the new one");
        assert_eq!(ShortcutTiles::default().shown(), Shortcut::ALL.len());
    }

    #[test]
    fn toggle_move_and_remove() {
        let mut t = ShortcutTiles::default();
        assert!(t.toggle(0));
        assert_eq!(t.shown(), Shortcut::ALL.len() - 1);
        assert!(t.toggle(0), "back on");
        assert!(!t.toggle(99));
        // The last one shown cannot go off.
        for i in 1..Shortcut::ALL.len() {
            assert!(t.toggle(i));
        }
        assert!(!t.toggle(0));
        assert_eq!(t.shown(), 1);

        let mut t = ShortcutTiles::default();
        assert!(t.move_by(0, 1));
        assert_eq!(&names(&t)[..2], ["Clipboard", "Screenshot"]);
        assert!(!t.move_by(0, -1));
        assert!(!t.move_by(Shortcut::ALL.len() - 1, 1));
        assert!(!t.move_by(99, -1));

        // Only a custom tile can be removed.
        assert!(!t.remove(0));
        t.add(custom("Gmail", CustomKind::Url, "https://mail.google.com"));
        assert!(t.remove(Shortcut::ALL.len()));
        assert_eq!(t.entries().len(), Shortcut::ALL.len());
    }

    /// Nine built-ins plus `own` custom tiles, all on.
    fn with_own(own: usize) -> ShortcutTiles {
        let mut t = ShortcutTiles::default();
        for i in 0..own {
            assert!(t.add(custom(&format!("T{i}"), CustomKind::App, r"C:.exe")));
        }
        t
    }

    #[test]
    fn no_tile_goes_on_while_twelve_show() {
        // 9 + 3 = 12 shown: the page is full.
        let mut t = with_own(3);
        assert!(t.full());
        assert!(t.toggle(0), "off is always fine");
        assert!(!t.full());
        assert!(t.toggle(0), "back on: 12 again");
        // A new own tile comes in on, past the twelfth: it cannot be
        // switched off and on again to jump the queue.
        assert!(t.add(custom("Baru", CustomKind::App, r"C:\b.exe")));
        let last = t.entries().len() - 1;
        assert!(t.toggle(last), "off");
        assert!(!t.toggle(last), "full: stays off");
        assert!(!t.entries()[last].1);
        // Room again after one goes off.
        assert!(t.toggle(1));
        assert!(t.toggle(last));
        assert_eq!(t.shown(), MAX_SHOWN);
    }

    #[test]
    fn tiles_on_past_the_twelfth_are_marked() {
        // Hand-added: 9 + 5 on, two past the limit.
        let t = with_own(5);
        assert_eq!(t.shown(), 14);
        let beyond: Vec<usize> = (0..t.entries().len()).filter(|i| t.beyond_limit(*i)).collect();
        assert_eq!(beyond, [12, 13]);
        assert_eq!(t.visible().len(), MAX_SHOWN);
        // An entry that is off is never "beyond"; neither is one that shows.
        let mut t = with_own(5);
        assert!(t.toggle(12));
        assert!(!t.beyond_limit(12));
        assert!(t.beyond_limit(13));
        assert!(!t.beyond_limit(0));
        assert!(!t.beyond_limit(99));
    }

    #[test]
    fn names_are_read_in_any_case() {
        let saved = [
            entry("lock", false),
            entry(" SCREENSHOT ", false),
            TileEntry {
                tile: "custom".into(),
                on: true,
                label: "Gmail".into(),
                kind: "url".into(),
                target: "https://mail.google.com".into(),
            },
        ];
        let t = ShortcutTiles::from_saved(&saved);
        assert_eq!(&names(&t)[..2], ["(Lock)", "(Screenshot)"]);
        assert_eq!(t.customs(), 1);
        assert!(names(&t).contains(&"+Gmail".to_owned()));
        // Each built-in still once, however it was written.
        let twice = ShortcutTiles::from_saved(&[entry("Lock", true), entry("LOCK", false)]);
        assert_eq!(twice.entries().len(), Shortcut::ALL.len());
        // Written back in the canonical spelling.
        assert_eq!(t.to_saved()[0].tile, "Lock");
        assert_eq!(t.to_saved().last().unwrap().tile, "Custom");
    }
}
