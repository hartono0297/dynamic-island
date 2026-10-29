//! The quick search: what a typed line finds. Apps (the Start Menu's
//! shortcuts, indexed once in memory), a calculator, a few Windows settings
//! pages and a web search. Everything here is pure except `index_start_menu`,
//! which reads the Start Menu folders; `App` runs the chosen result.
//!
//! Files and folders (names under the user's Desktop, Documents, Downloads,
//! Pictures, Videos and Music) come from `search_files`; they are matched
//! from two typed characters and share the rows with the apps.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::i18n::Lang;
use crate::search_files::{FileEntry, RootKind};

/// Results the list shows at most.
pub const MAX_RESULTS: usize = 6;
/// Typed characters (after trimming) before files and folders are searched;
/// apps and settings pages match from the first one.
pub const FILE_MIN_CHARS: usize = 2;
/// File and folder rows at most while apps still have hits to show.
pub const FILE_ROWS_MAX: usize = 3;
/// App rows kept when apps match, however many files do.
pub const APP_ROWS_MIN: usize = 2;
/// The query's longest length, in characters.
pub const QUERY_MAX: usize = 120;
/// The Start Menu folders are read this many levels down.
const INDEX_DEPTH: usize = 4;
/// Shortcuts the index keeps at most (a safety net for a huge menu).
const INDEX_MAX: usize = 4000;
/// How deep a calculator expression may nest parentheses.
const CALC_DEPTH: usize = 48;

/// An app the Start Menu offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    /// The `.lnk` file, run as the shell would.
    pub path: String,
}

/// What a result looks like (its icon) and means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    Setting,
    Calc,
    Web,
    File,
    Folder,
}

/// What Enter does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `ShellExecute` "open" on a shortcut file.
    Launch(String),
    /// `ShellExecute` "open" on an `ms-settings:` page.
    OpenSetting(&'static str),
    /// Put this text on the clipboard.
    Copy(String),
    /// `ShellExecute` "open" on an https address built here.
    OpenWeb(String),
    /// `ShellExecute` "open" on a file or folder found under the user's own
    /// folders (a double-click's effect).
    OpenPath(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    /// What the details (icon, folder count, install size) are looked up by:
    /// an app's shortcut, a file's or folder's own path.
    pub path: Option<PathBuf>,
}

// ---------------------------------------------------------------------
// Windows settings pages

/// A settings page: its names and its `ms-settings:` address.
pub struct SettingsPage {
    pub id: &'static str,
    pub en: &'static str,
    pub uri: &'static str,
}

impl SettingsPage {
    pub fn name(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Id => self.id,
            Lang::En => self.en,
        }
    }
}

pub const SETTINGS_PAGES: [SettingsPage; 16] = [
    SettingsPage { id: "Layar", en: "Display", uri: "ms-settings:display" },
    SettingsPage { id: "Suara", en: "Sound", uri: "ms-settings:sound" },
    SettingsPage { id: "Notifikasi", en: "Notifications", uri: "ms-settings:notifications" },
    SettingsPage { id: "Baterai", en: "Battery", uri: "ms-settings:batterysaver" },
    SettingsPage { id: "Penyimpanan", en: "Storage", uri: "ms-settings:storagesense" },
    SettingsPage { id: "Wi-Fi", en: "Wi-Fi", uri: "ms-settings:network-wifi" },
    SettingsPage { id: "Bluetooth", en: "Bluetooth", uri: "ms-settings:bluetooth" },
    SettingsPage { id: "Printer dan pemindai", en: "Printers & scanners", uri: "ms-settings:printers" },
    SettingsPage { id: "Aplikasi terpasang", en: "Installed apps", uri: "ms-settings:appsfeatures" },
    SettingsPage { id: "Aplikasi bawaan", en: "Default apps", uri: "ms-settings:defaultapps" },
    SettingsPage { id: "Personalisasi", en: "Personalization", uri: "ms-settings:personalization" },
    SettingsPage { id: "Tanggal dan waktu", en: "Date & time", uri: "ms-settings:dateandtime" },
    SettingsPage { id: "Bahasa dan wilayah", en: "Language & region", uri: "ms-settings:regionlanguage" },
    SettingsPage { id: "Privasi dan keamanan", en: "Privacy & security", uri: "ms-settings:privacy" },
    SettingsPage { id: "Pembaruan Windows", en: "Windows Update", uri: "ms-settings:windowsupdate" },
    SettingsPage { id: "Tentang perangkat", en: "About", uri: "ms-settings:about" },
];

// ---------------------------------------------------------------------
// The calculator

/// What a line is worth as a calculation: its value and whether it used an
/// operator or parentheses (a lone number is not worth a result row).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calc {
    pub value: f64,
    pub compound: bool,
}

/// `+ - * / % ^` (also `×` and `÷`), parentheses, decimals (`.` or `,`) and
/// spaces. `%` is the remainder. The language's thousands mark (`.` in
/// Indonesian, `,` in English) is read as one when exactly three digits
/// follow it, before any decimal mark: "1.000 + 500" is 1500 in Indonesian,
/// "1,000 + 500" in English; otherwise either mark is a decimal one.
/// Anything else, a division by zero or a result that is not a finite number
/// gives `None`; it never panics.
pub fn calculate(line: &str, lang: Lang) -> Option<Calc> {
    if line.chars().count() > QUERY_MAX {
        return None;
    }
    if line.trim().is_empty() {
        return None;
    }
    let mut p = Parser {
        chars: line.chars().collect(),
        at: 0,
        depth: 0,
        compound: false,
        group: match lang {
            Lang::Id => '.',
            Lang::En => ',',
        },
    };
    let value = p.expr()?;
    if p.peek().is_some() || !value.is_finite() {
        return None;
    }
    Some(Calc {
        value,
        compound: p.compound,
    })
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
    compound: bool,
    /// The thousands mark of the line's language.
    group: char,
}

impl Parser {
    /// The next character, spaces skipped (never inside a number).
    fn peek(&mut self) -> Option<char> {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                self.compound = true;
                v += self.term()?;
            } else if self.eat('-') {
                self.compound = true;
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    /// term := unary (('*' | '×' | '/' | '÷' | '%') unary)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') || self.eat('×') {
                self.compound = true;
                v *= self.unary()?;
            } else if self.eat('/') || self.eat('÷') {
                self.compound = true;
                let d = self.unary()?;
                if d == 0.0 {
                    return None;
                }
                v /= d;
            } else if self.eat('%') {
                self.compound = true;
                let d = self.unary()?;
                if d == 0.0 {
                    return None;
                }
                v %= d;
            } else {
                return Some(v);
            }
        }
    }

    /// unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > CALC_DEPTH {
            return None;
        }
        let v = if self.eat('-') {
            self.unary().map(|v| -v)
        } else if self.eat('+') {
            self.unary()
        } else {
            self.power()
        };
        self.depth -= 1;
        v
    }

    /// power := atom ('^' unary)?   (right to left: 2^3^2 = 2^9)
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            self.compound = true;
            let exp = self.unary()?;
            let v = base.powf(exp);
            return v.is_finite().then_some(v);
        }
        Some(base)
    }

    /// atom := number | '(' expr ')'
    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            self.compound = true;
            let v = self.expr()?;
            return self.eat(')').then_some(v);
        }
        self.number()
    }

    fn number(&mut self) -> Option<f64> {
        self.peek();
        let mut text = String::new();
        let mut dot = false;
        // Digits since the start or the last thousands mark.
        let mut run = 0;
        let mut grouped = false;
        while let Some(&c) = self.chars.get(self.at) {
            match c {
                '0'..='9' => {
                    text.push(c);
                    run += 1;
                    self.at += 1;
                }
                // A thousands mark: after one to three digits that do not
                // start with 0 ("0.001" is a decimal, "1234.567" is not
                // grouped), before any decimal mark, with exactly three
                // digits after it. It adds nothing.
                c if c == self.group
                    && !dot
                    && (if grouped {
                        run == 3
                    } else {
                        (1..=3).contains(&run) && !text.starts_with('0')
                    })
                    && self.three_digits_follow() =>
                {
                    grouped = true;
                    run = 0;
                    self.at += 1;
                }
                '.' | ',' if !dot => {
                    dot = true;
                    text.push('.');
                    self.at += 1;
                }
                _ => break,
            }
        }
        if text.is_empty() || text == "." {
            return None;
        }
        text.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    /// Exactly three digits follow the mark at `at`, and no fourth.
    fn three_digits_follow(&self) -> bool {
        let digit = |k: usize| self.chars.get(self.at + k).is_some_and(|c| c.is_ascii_digit());
        (1..=3).all(digit) && !digit(4)
    }
}

/// A calculation's value as text: whole numbers without a point, others to
/// ten decimals with the zeros trimmed, huge or tiny ones in `e` notation.
/// `lang` picks the decimal mark ("," or ".").
pub fn format_number(value: f64, lang: Lang) -> String {
    let abs = value.abs();
    let text = if abs >= 1e15 || (abs != 0.0 && abs < 1e-9) {
        format!("{value:e}")
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        let s = format!("{value:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let text = if text == "-0" { "0".to_owned() } else { text };
    match lang {
        Lang::Id => text.replace('.', ","),
        Lang::En => text,
    }
}

// ---------------------------------------------------------------------
// The web

/// Google's search address for `query`, or `None` for an empty query or one
/// with control characters. The query is percent-encoded (a space is `+`),
/// so whatever is typed stays inside the `q=` value; the scheme and host are
/// ours, never the user's.
pub fn web_url(query: &str) -> Option<String> {
    let q = query.trim();
    if q.is_empty() || q.chars().count() > QUERY_MAX || q.chars().any(char::is_control) {
        return None;
    }
    let mut url = String::from("https://www.google.com/search?q=");
    for b in q.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                url.push(b as char)
            }
            b' ' => url.push('+'),
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    Some(url)
}

/// An address the island may open on its own: https only.
pub fn is_web_url(url: &str) -> bool {
    url.starts_with("https://www.google.com/search?q=")
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

// ---------------------------------------------------------------------
// Ranking

/// How well `name` matches `query` (both compared case-insensitively):
/// 3 starts with it, 2 a word starts with it, 1 contains it, 0 no match.
pub fn match_score(name: &str, query: &str) -> u8 {
    score_lower(&name.to_lowercase(), &query.trim().to_lowercase())
}

/// `match_score` for a name and a query that are lower case already (the
/// file index keeps lower-case names, so a keystroke lowers none of them).
fn score_lower(name: &str, query: &str) -> u8 {
    if query.is_empty() {
        return 0;
    }
    if name.starts_with(query) {
        return 3;
    }
    let word_start = name
        .match_indices(query)
        .any(|(i, _)| name[..i].chars().next_back().is_some_and(|c| !c.is_alphanumeric()));
    if word_start {
        2
    } else if name.contains(query) {
        1
    } else {
        0
    }
}

/// Indices into `names` that match `query`, best first; equal scores keep
/// their order in `names`.
pub fn rank<'a>(names: impl IntoIterator<Item = &'a str>, query: &str) -> Vec<usize> {
    let mut hits: Vec<(u8, usize)> = names
        .into_iter()
        .enumerate()
        .filter_map(|(i, n)| match match_score(n, query) {
            0 => None,
            s => Some((s, i)),
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

/// The text a result row says about its kind, in `lang`.
pub struct Labels<'a> {
    pub app: &'a str,
    pub setting: &'a str,
    pub copy: &'a str,
    pub web: &'a str,
    /// `Cari "…" di web`, already filled in for the query.
    pub web_title: &'a dyn Fn(&str) -> String,
    /// A root's name ("Unduhan").
    pub root: &'a dyn Fn(RootKind) -> &'static str,
    /// A file's size as a row says it ("2,4 MB").
    pub size: &'a dyn Fn(u64) -> String,
}

/// What `line` finds, at most `MAX_RESULTS` rows. A line starting with `?`
/// is a web search and nothing else. Otherwise: the calculation (when it is
/// one), then apps and settings pages by match quality, then files and
/// folders (from `FILE_MIN_CHARS` typed characters; `split_room` shares the
/// rows), and the web search last (always, so Enter always does something).
/// An empty line finds nothing.
pub fn search(
    line: &str,
    apps: &[AppEntry],
    files: &[FileEntry],
    lang: Lang,
    labels: &Labels,
) -> Vec<SearchResult> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    if let Some(rest) = line.strip_prefix('?') {
        return web_url(rest)
            .map(|url| vec![web_result(rest.trim(), url, labels)])
            .unwrap_or_default();
    }
    let mut out = Vec::new();
    if let Some(calc) = calculate(line, lang).filter(|c| c.compound) {
        let shown = format_number(calc.value, lang);
        out.push(SearchResult {
            kind: Kind::Calc,
            title: format!("= {shown}"),
            subtitle: labels.copy.to_owned(),
            action: Action::Copy(shown),
            path: None,
        });
    }
    let web = web_url(line).map(|url| web_result(line, url, labels));
    let room = MAX_RESULTS - out.len() - usize::from(web.is_some());

    // Apps and settings pages compete by score; an app wins a tie.
    let mut found: Vec<(u8, usize, SearchResult)> = Vec::new();
    for i in rank(apps.iter().map(|a| a.name.as_str()), line) {
        let a = &apps[i];
        found.push((
            match_score(&a.name, line),
            found.len(),
            SearchResult {
                kind: Kind::App,
                title: a.name.clone(),
                subtitle: labels.app.to_owned(),
                action: Action::Launch(a.path.clone()),
                path: Some(PathBuf::from(&a.path)),
            },
        ));
    }
    for i in rank(SETTINGS_PAGES.iter().map(|p| p.name(lang)), line) {
        let p = &SETTINGS_PAGES[i];
        found.push((
            match_score(p.name(lang), line),
            found.len(),
            SearchResult {
                kind: Kind::Setting,
                title: p.name(lang).to_owned(),
                subtitle: labels.setting.to_owned(),
                action: Action::OpenSetting(p.uri),
                path: None,
            },
        ));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let file_hits = if line.chars().count() >= FILE_MIN_CHARS {
        rank_files(files, line)
    } else {
        Vec::new()
    };
    let (app_rows, file_rows) = split_room(room, found.len(), file_hits.len());
    out.extend(found.into_iter().take(app_rows).map(|(_, _, r)| r));
    out.extend(
        file_hits
            .into_iter()
            .take(file_rows)
            .map(|i| file_result(&files[i], labels)),
    );
    out.extend(web);
    out
}

/// How `room` rows go to `apps` app/settings hits and `files` file hits:
/// files take at most `FILE_ROWS_MAX` and never so many that fewer than
/// `APP_ROWS_MIN` app rows are left when apps match; apps take the rest;
/// room still left over goes to the files that have not shown yet. Returns
/// `(app rows, file rows)`.
fn split_room(room: usize, apps: usize, files: usize) -> (usize, usize) {
    let files_now = files
        .min(FILE_ROWS_MAX)
        .min(room.saturating_sub(apps.min(APP_ROWS_MIN)));
    let apps_now = apps.min(room - files_now);
    let spare = room - files_now - apps_now;
    (apps_now, files_now + spare.min(files - files_now))
}

/// Indices into `files` that match `query`, best first: by score, then
/// folders before files, then the newest, then the shortest name, then the
/// walk's own order.
fn rank_files(files: &[FileEntry], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut hits: Vec<(u8, usize)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| match score_lower(&f.lower, &query) {
            0 => None,
            s => Some((s, i)),
        })
        .collect();
    hits.sort_by(|a, b| {
        let (fa, fb) = (&files[a.1], &files[b.1]);
        b.0.cmp(&a.0)
            .then(fb.is_dir.cmp(&fa.is_dir))
            .then(fb.modified.cmp(&fa.modified))
            .then(fa.name.chars().count().cmp(&fb.name.chars().count()))
            .then(a.1.cmp(&b.1))
    });
    hits.into_iter().map(|(_, i)| i).collect()
}

/// The row for a file or folder: it opens its own path. The subtitle names
/// where it is (the root, and the folder it is in, cut to the last two
/// levels), a file's size after it.
fn file_result(f: &FileEntry, labels: &Labels) -> SearchResult {
    let place = location((labels.root)(f.root), f.parent.as_deref());
    let (kind, subtitle) = if f.is_dir {
        (Kind::Folder, place)
    } else {
        (Kind::File, format!("{place} \u{b7} {}", (labels.size)(f.size)))
    };
    SearchResult {
        kind,
        title: f.name.clone(),
        subtitle,
        action: Action::OpenPath(f.path.clone()),
        path: Some(f.path.clone()),
    }
}

/// "Dokumen", "Dokumen \u{203a} Proyek", "Dokumen \u{203a} \u{2026} \u{203a} B \u{203a} C": the root, then
/// the last two folders of `parent` (backslash separated) when it has
/// more.
fn location(root: &str, parent: Option<&str>) -> String {
    let parts: Vec<&str> = parent
        .map(|p| p.split('\\').filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    match parts.len() {
        0 => root.to_owned(),
        1 | 2 => format!("{root} \u{203a} {}", parts.join(" \u{203a} ")),
        n => format!("{root} \u{203a} \u{2026} \u{203a} {}", parts[n - 2..].join(" \u{203a} ")),
    }
}

fn web_result(shown: &str, url: String, labels: &Labels) -> SearchResult {
    SearchResult {
        kind: Kind::Web,
        title: (labels.web_title)(shown),
        subtitle: labels.web.to_owned(),
        action: Action::OpenWeb(url),
        path: None,
    }
}

// ---------------------------------------------------------------------
// The Start Menu index

/// The shortcuts of the Start Menu folders (all users, then this user),
/// one per name (the first wins), uninstallers left out, by name.
pub fn index_start_menu() -> Vec<AppEntry> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramData", "APPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            roots.push(Path::new(&base).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
    }
    let mut out = Vec::new();
    for root in roots {
        collect(&root, INDEX_DEPTH, &mut out);
    }
    dedupe(out)
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if out.len() >= INDEX_MAX {
            return;
        }
        let path = entry.path();
        // A link to a folder is not followed: no loops.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if let Some(app) = app_entry(&path) {
            out.push(app);
        }
    }
}

/// The entry for a `.lnk` file, unless it is an uninstaller or a help link.
fn app_entry(path: &Path) -> Option<AppEntry> {
    let ext = path.extension()?.to_str()?;
    if !ext.eq_ignore_ascii_case("lnk") {
        return None;
    }
    let name = path.file_stem()?.to_str()?.trim().to_owned();
    let lower = name.to_lowercase();
    let skip = ["uninstall", "copot", "hapus ", "readme", "help", "bantuan", "release notes"];
    if name.is_empty() || skip.iter().any(|s| lower.contains(s)) {
        return None;
    }
    Some(AppEntry {
        name,
        path: path.to_str()?.to_owned(),
    })
}

/// One entry per name (case-insensitive, the first kept), sorted by name.
pub fn dedupe(mut apps: Vec<AppEntry>) -> Vec<AppEntry> {
    let mut seen = std::collections::HashSet::new();
    apps.retain(|a| seen.insert(a.name.to_lowercase()));
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Labels<'static> {
        Labels {
            app: "Aplikasi",
            setting: "Pengaturan Windows",
            copy: "Enter untuk menyalin",
            web: "Cari di web",
            web_title: &|q| format!("Cari \"{q}\" di web"),
            root: &|kind| match kind {
                RootKind::Desktop => "Desktop",
                RootKind::Documents => "Dokumen",
                RootKind::Downloads => "Unduhan",
                RootKind::Pictures => "Gambar",
                RootKind::Videos => "Video",
                RootKind::Music => "Musik",
            },
            size: &|bytes| format!("{bytes} B"),
        }
    }

    /// A file or folder as the walk would have found it.
    fn entry(
        name: &str,
        root: RootKind,
        parent: Option<&str>,
        is_dir: bool,
        age: u64,
    ) -> FileEntry {
        let path = match parent {
            Some(p) => format!(r"C:\Home\{p}\{name}"),
            None => format!(r"C:\Home\{name}"),
        };
        FileEntry {
            name: name.into(),
            lower: name.to_lowercase(),
            path: PathBuf::from(path),
            root,
            parent: parent.map(str::to_owned),
            is_dir,
            size: 2048,
            // A smaller `age` is newer.
            modified: Some(
                SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 - age),
            ),
        }
    }

    fn file(name: &str) -> FileEntry {
        entry(name, RootKind::Documents, None, false, 10)
    }

    fn app(name: &str) -> AppEntry {
        AppEntry {
            name: name.into(),
            path: format!(r"C:\Menu\{name}.lnk"),
        }
    }

    /// The line's value, the same in both languages (no thousands mark in
    /// it).
    fn calc(line: &str) -> Option<f64> {
        let id = calc_in(line, Lang::Id);
        assert_eq!(id, calc_in(line, Lang::En), "{line:?}");
        id
    }

    fn calc_in(line: &str, lang: Lang) -> Option<f64> {
        calculate(line, lang).map(|c| c.value)
    }

    #[test]
    fn the_calculator_follows_precedence_and_parentheses() {
        assert_eq!(calc("1+2*3"), Some(7.0));
        assert_eq!(calc("(1+2)*3"), Some(9.0));
        assert_eq!(calc("10-4-3"), Some(3.0), "left to right");
        assert_eq!(calc("2^3^2"), Some(512.0), "right to left");
        assert_eq!(calc("-2^2"), Some(-4.0));
        assert_eq!(calc("2*-3"), Some(-6.0));
        assert_eq!(calc("--4"), Some(4.0));
        assert_eq!(calc("7%3"), Some(1.0));
        assert_eq!(calc("8÷2×3"), Some(12.0));
        assert_eq!(calc(" 12 / 4 + 1 "), Some(4.0));
        assert_eq!(calc("1,5+1.25"), Some(2.75), "either decimal mark");
        assert_eq!(calc(".5*4"), Some(2.0));
        assert_eq!(calc("((((2))))+1"), Some(3.0));
    }

    #[test]
    fn the_calculator_refuses_what_is_not_arithmetic() {
        for bad in [
            "", "   ", "abc", "1+", "+", "(", ")", "()", "(1+2", "1+2)", "1..2", "1.2.3", "2 3",
            "sqrt(4)", "1+a", "0x10", "1e3", "5!", "1/0", "1%0", "-", "^2", "2^", "1;2",
            "javascript:alert(1)", "C:\\Windows",
        ] {
            assert_eq!(calc(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_calculator_survives_huge_and_deep_input() {
        // Not finite: refused, not a panic or "inf".
        assert_eq!(calc("10^1000"), None);
        assert_eq!(calc("9^9^9"), None);
        assert_eq!(calc(&format!("1{}", "0".repeat(400))), None);
        // A very long or very deep line is refused.
        assert_eq!(calc(&"1+".repeat(200)), None, "too long");
        let deep = format!("{}1{}", "(".repeat(55), ")".repeat(55));
        assert_eq!(calc(&deep), None, "too deep");
        let signs = format!("{}1", "-".repeat(100));
        assert_eq!(calc(&signs), None, "too many signs");
        assert_eq!(calc("999999999999*999999999999").map(|v| v > 9e23), Some(true));
    }

    #[test]
    fn a_lone_number_is_not_compound() {
        for lang in [Lang::Id, Lang::En] {
            assert_eq!(calculate("42", lang).map(|c| c.compound), Some(false));
            assert_eq!(calculate("(42)", lang).map(|c| c.compound), Some(true));
            assert_eq!(calculate("1+1", lang).map(|c| c.compound), Some(true));
            assert_eq!(calculate("1.000", lang).map(|c| c.compound), Some(false));
        }
    }

    #[test]
    fn the_thousands_mark_follows_the_language() {
        // Indonesian: "." groups thousands, "," is the decimal mark.
        assert_eq!(calc_in("1.000 + 500", Lang::Id), Some(1500.0));
        assert_eq!(calc_in("1.000.000 / 1.000", Lang::Id), Some(1000.0));
        assert_eq!(calc_in("1.000,5 * 2", Lang::Id), Some(2001.0));
        assert_eq!(calc_in("2,5 * 2", Lang::Id), Some(5.0));
        assert_eq!(calc_in("1,000 + 1", Lang::Id), Some(2.0), "a comma is decimal");
        // Not exactly three digits after it: a decimal point after all.
        assert_eq!(calc_in("1.5 + 1", Lang::Id), Some(2.5));
        assert_eq!(calc_in("1.25 + 1", Lang::Id), Some(2.25));
        assert_eq!(calc_in("1.0000 + 1", Lang::Id), Some(2.0));
        assert_eq!(calc_in(".500 * 2", Lang::Id), Some(1.0), "no digit before");
        // English: "," groups thousands, "." is the decimal mark.
        assert_eq!(calc_in("1,000 + 500", Lang::En), Some(1500.0));
        assert_eq!(calc_in("1,000,000 / 1,000", Lang::En), Some(1000.0));
        assert_eq!(calc_in("1,000.5 * 2", Lang::En), Some(2001.0));
        assert_eq!(calc_in("1.000 + 500", Lang::En), Some(501.0), "a point is decimal");
        assert_eq!(calc_in("1,5 + 1", Lang::En), Some(2.5));
        // No thousands mark after the decimal one.
        assert_eq!(calc_in("1,5.000 + 1", Lang::Id), None);
        assert_eq!(calc_in("1.5,000 + 1", Lang::En), None);
        assert_eq!(calc_in("1..000", Lang::Id), None);
        // The search row shows the Indonesian sum.
        let r = search("1.000 + 500", &[], &[], Lang::Id, &labels());
        assert_eq!(r[0].title, "= 1500");
        // A leading 0 or more than three digits before the mark: a decimal.
        assert_eq!(calc_in("0.001 * 5", Lang::Id), Some(0.005));
        assert_eq!(calc_in("0.125 + 1", Lang::Id), Some(1.125));
        assert_eq!(calc_in("0,001 * 5", Lang::En), Some(0.005));
        assert_eq!(calc_in("1234.567", Lang::Id), Some(1234.567));
    }

    #[test]
    fn numbers_print_plainly_in_both_languages() {
        assert_eq!(format_number(7.0, Lang::En), "7");
        assert_eq!(format_number(-0.0, Lang::En), "0");
        assert_eq!(format_number(2.75, Lang::En), "2.75");
        assert_eq!(format_number(2.75, Lang::Id), "2,75");
        assert_eq!(format_number(1.0 / 3.0, Lang::En), "0.3333333333");
        assert_eq!(format_number(1e20, Lang::En), "1e20");
        assert_eq!(format_number(1e-12, Lang::En), "1e-12");
        assert_eq!(format_number(123456789012.0, Lang::Id), "123456789012");
        assert_eq!(format_number(-12.5, Lang::Id), "-12,5");
    }

    #[test]
    fn scores_rank_prefix_over_word_start_over_substring() {
        assert_eq!(match_score("Kalkulator", "kal"), 3);
        assert_eq!(match_score("Windows Kalender", "kal"), 2);
        assert_eq!(match_score("Pengkalkulasi", "kal"), 1);
        assert_eq!(match_score("Notepad", "kal"), 0);
        assert_eq!(match_score("Kalkulator", ""), 0);
        assert_eq!(match_score("Kalkulator", "  "), 0);
        assert_eq!(match_score("Visual Studio Code", "STUDIO"), 2);
        assert_eq!(match_score("Wi-Fi", "fi"), 2, "after a hyphen");
    }

    #[test]
    fn ranking_is_stable_for_equal_scores() {
        let names = ["Zeta kal", "Alfa kal", "Kal one", "Kal two", "Nope", "xkal"];
        // Prefix: 2, 3 in order; word start: 0, 1 in order; substring: 5.
        assert_eq!(rank(names, "kal"), [2, 3, 0, 1, 5]);
        assert_eq!(rank(names, "zzz"), Vec::<usize>::new());
    }

    #[test]
    fn web_urls_stay_inside_the_query_value() {
        let url = web_url("kopi susu & roti").unwrap();
        assert_eq!(url, "https://www.google.com/search?q=kopi+susu+%26+roti");
        assert!(is_web_url(&url));
        // Non-ASCII goes out as UTF-8 bytes.
        assert_eq!(
            web_url("café").unwrap(),
            "https://www.google.com/search?q=caf%C3%A9"
        );
        // A hostile line is only data in q=.
        for evil in [
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "https://evil.example/ x",
            "ms-settings:",
            "a#b?c=d&e",
            "\"><script>",
        ] {
            let url = web_url(evil).unwrap();
            assert!(url.starts_with("https://www.google.com/search?q="), "{url}");
            assert!(is_web_url(&url), "{url}");
            let q = &url["https://www.google.com/search?q=".len()..];
            assert!(
                q.bytes().all(|b| b.is_ascii_alphanumeric()
                    || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'%')),
                "{url}"
            );
        }
        assert_eq!(web_url(""), None);
        assert_eq!(web_url("   "), None);
        assert_eq!(web_url("a\nb"), None);
        assert_eq!(web_url(&"a".repeat(QUERY_MAX + 1)), None);
        assert!(!is_web_url("http://www.google.com/search?q=a"));
        assert!(!is_web_url("https://evil.example/?q=a"));
        assert!(!is_web_url("javascript:alert(1)"));
    }


    #[test]
    fn files_start_at_two_characters_apps_at_one() {
        let apps = [app("Kalender")];
        let files = [file("kartu.txt")];
        // (Settings pages match short lines too; they are not the point.)
        let kinds = |line: &str| -> Vec<Kind> {
            search(line, &apps, &files, Lang::Id, &labels())
                .iter()
                .map(|r| r.kind)
                .filter(|k| *k != Kind::Setting)
                .collect()
        };
        // One character: the app only (and the web).
        assert_eq!(kinds("k"), [Kind::App, Kind::Web]);
        assert_eq!(kinds("  k  "), [Kind::App, Kind::Web], "trimmed first");
        // Two: the file is found too, after the app and before the web.
        assert_eq!(kinds("ka"), [Kind::App, Kind::File, Kind::Web]);
        assert_eq!(kinds(" ka "), [Kind::App, Kind::File, Kind::Web]);
        // Two characters, not two bytes.
        let accented = [file("\u{e9}a.txt")];
        let r = search("\u{e9}a", &[], &accented, Lang::Id, &labels());
        assert_eq!(r[0].kind, Kind::File);
        let r = search("\u{e9}", &[], &accented, Lang::Id, &labels());
        assert!(r.iter().all(|r| r.kind != Kind::File));
    }

    #[test]
    fn files_rank_by_score_then_folders_then_newest() {
        let docs = RootKind::Documents;
        let files = [
            entry("old report.txt", docs, None, false, 500),
            entry("my report.txt", docs, None, false, 1),
            entry("report b.txt", docs, None, false, 100),
            entry("report a.txt", docs, None, false, 5),
            entry("Report", docs, None, true, 900),
            entry("reporting-tool-notes.txt", docs, None, false, 1),
        ];
        let names = |line: &str, files: &[FileEntry]| -> Vec<String> {
            search(line, &[], files, Lang::Id, &labels())
                .into_iter()
                .filter(|r| matches!(r.kind, Kind::File | Kind::Folder))
                .map(|r| r.title)
                .collect()
        };
        // Prefix matches first: the folder before the files, then the newest
        // file (a newer than b). Only so many rows show: the rest is cut.
        assert_eq!(
            names("report", &files),
            ["Report", "reporting-tool-notes.txt", "report a.txt", "report b.txt", "my report.txt"]
        );
        // A word start (2) ranks under a prefix (3).
        assert_eq!(names("report", &files[..2]), ["my report.txt", "old report.txt"]);
        // Equal everything but the length: the shorter name first.
        let same = [
            entry("notes-long-name.txt", docs, None, false, 3),
            entry("notes.txt", docs, None, false, 3),
        ];
        assert_eq!(names("notes", &same), ["notes.txt", "notes-long-name.txt"]);
        // The score still beats a folder: a prefix file over a substring folder.
        let mixed = [
            entry("xxreportxx", docs, None, true, 1),
            entry("report.txt", docs, None, false, 900),
        ];
        assert_eq!(names("report", &mixed), ["report.txt", "xxreportxx"]);
    }

    #[test]
    fn rows_are_shared_between_apps_and_files() {
        let apps: Vec<AppEntry> = (0..5).map(|i| app(&format!("Tool {i}"))).collect();
        let files: Vec<FileEntry> = (0..5)
            .map(|i| file(&format!("tool file {i}.txt")))
            .collect();
        let count = |r: &[SearchResult], kind: Kind| r.iter().filter(|r| r.kind == kind).count();
        let order = |r: &[SearchResult]| -> Vec<Kind> { r.iter().map(|r| r.kind).collect() };

        // 5 apps and 5 files: 3 files and 2 apps (room for 5 with the web).
        let r = search("tool", &apps, &files, Lang::Id, &labels());
        assert_eq!((count(&r, Kind::App), count(&r, Kind::File)), (2, 3));
        assert_eq!(r.len(), MAX_RESULTS);
        // Grouped, not interleaved: apps, then files, then the web.
        assert_eq!(
            order(&r),
            [Kind::App, Kind::App, Kind::File, Kind::File, Kind::File, Kind::Web]
        );

        // Files only: up to 5.
        let r = search("tool", &[], &files, Lang::Id, &labels());
        assert_eq!((count(&r, Kind::App), count(&r, Kind::File)), (0, 5));
        // Apps only: up to 5.
        let r = search("tool", &apps, &[], Lang::Id, &labels());
        assert_eq!((count(&r, Kind::App), count(&r, Kind::File)), (5, 0));
        // One app and many files: the leftover room goes to the files.
        let r = search("tool", &apps[..1], &files, Lang::Id, &labels());
        assert_eq!((count(&r, Kind::App), count(&r, Kind::File)), (1, 4));
        // Many apps, one file: the file keeps its row, apps take the rest.
        let r = search("tool", &apps, &files[..1], Lang::Id, &labels());
        assert_eq!((count(&r, Kind::App), count(&r, Kind::File)), (4, 1));

        // With a calculation and the web: still six, in order.
        let apps: Vec<AppEntry> = (0..5).map(|i| app(&format!("1+1 App {i}"))).collect();
        let files: Vec<FileEntry> = (0..5)
            .map(|i| file(&format!("1+1 file {i}.txt")))
            .collect();
        let r = search("1+1", &apps, &files, Lang::Id, &labels());
        assert_eq!(r.len(), MAX_RESULTS);
        assert_eq!(
            order(&r),
            [Kind::Calc, Kind::App, Kind::App, Kind::File, Kind::File, Kind::Web]
        );
    }

    #[test]
    fn a_file_row_opens_its_path_and_names_its_folder() {
        let nested = entry("laporan.pdf", RootKind::Downloads, Some("Kantor"), false, 1);
        let top = entry("Gambar kerja", RootKind::Documents, None, true, 1);
        let deep = entry("c.txt", RootKind::Documents, Some(r"A\B\C\D"), false, 1);
        let files = [nested.clone(), top.clone(), deep.clone()];
        let r = search("laporan", &[], &files, Lang::Id, &labels());
        assert_eq!(r[0].kind, Kind::File);
        assert_eq!(r[0].title, "laporan.pdf");
        assert_eq!(r[0].action, Action::OpenPath(nested.path.clone()));
        assert_eq!(r[0].path, Some(nested.path.clone()));
        // Root, its folder, then the size.
        assert_eq!(r[0].subtitle, "Unduhan \u{203a} Kantor \u{b7} 2048 B");
        // A folder: where it is, no size.
        let r = search("gambar", &[], &files, Lang::Id, &labels());
        assert_eq!(r[0].kind, Kind::Folder);
        assert_eq!(r[0].action, Action::OpenPath(top.path.clone()));
        assert_eq!(r[0].subtitle, "Dokumen");
        // A deep folder is cut to its last two levels.
        let r = search("c.txt", &[], &files, Lang::Id, &labels());
        assert_eq!(
            r[0].subtitle,
            "Dokumen \u{203a} \u{2026} \u{203a} C \u{203a} D \u{b7} 2048 B"
        );
        // An app and a settings row carry what the details need, or nothing.
        let apps = [app("Laporan Studio")];
        let r = search("laporan", &apps, &[], Lang::Id, &labels());
        assert_eq!(r[0].path, Some(PathBuf::from(r"C:\Menu\Laporan Studio.lnk")));
        let r = search("layar", &apps, &[], Lang::Id, &labels());
        assert_eq!((r[0].kind, r[0].path.clone()), (Kind::Setting, None));
    }

    #[test]
    fn a_question_mark_line_still_finds_only_the_web_with_files_present() {
        let apps = [app("Kopi")];
        let files = [file("kopi susu.txt")];
        let r = search("? kopi", &apps, &files, Lang::Id, &labels());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, Kind::Web);
        assert!(search("?", &apps, &files, Lang::Id, &labels()).is_empty());
    }

    #[test]
    fn a_question_mark_line_is_only_a_web_search() {
        let apps = [app("Kalkulator")];
        let r = search("? kopi susu", &apps, &[], Lang::Id, &labels());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, Kind::Web);
        assert_eq!(r[0].title, "Cari \"kopi susu\" di web");
        assert_eq!(
            r[0].action,
            Action::OpenWeb("https://www.google.com/search?q=kopi+susu".into())
        );
        assert!(search("?", &apps, &[], Lang::Id, &labels()).is_empty());
        assert!(search("?   ", &apps, &[], Lang::Id, &labels()).is_empty());
    }

    #[test]
    fn a_line_finds_the_calculation_apps_settings_and_the_web_in_order() {
        let apps = [app("Kalender"), app("Kalkulator"), app("Notepad")];
        let r = search("kal", &apps, &[], Lang::Id, &labels());
        let kinds: Vec<Kind> = r.iter().map(|r| r.kind).collect();
        // Two apps, then the web; "kal" matches no settings page.
        assert_eq!(kinds, [Kind::App, Kind::App, Kind::Web]);
        assert_eq!(r[0].title, "Kalender");
        assert_eq!(r[1].action, Action::Launch(r"C:\Menu\Kalkulator.lnk".into()));

        // A calculation comes first, and is what Enter copies.
        let r = search("2+2*3", &apps, &[], Lang::Id, &labels());
        assert_eq!(r[0].kind, Kind::Calc);
        assert_eq!(r[0].title, "= 8");
        assert_eq!(r[0].action, Action::Copy("8".into()));
        assert_eq!(r.last().unwrap().kind, Kind::Web);
        // A lone number is not one.
        assert!(search("42", &apps, &[], Lang::Id, &labels()).iter().all(|r| r.kind != Kind::Calc));
        // The decimal mark follows the language.
        let r = search("1/4", &apps, &[], Lang::Id, &labels());
        assert_eq!(r[0].title, "= 0,25");
        // A settings page, in the language.
        let r = search("layar", &apps, &[], Lang::Id, &labels());
        assert_eq!(r[0].kind, Kind::Setting);
        assert_eq!(r[0].action, Action::OpenSetting("ms-settings:display"));
        let r = search("display", &apps, &[], Lang::En, &labels());
        assert_eq!(r[0].title, "Display");
        // Nothing typed, nothing found.
        assert!(search("  ", &apps, &[], Lang::Id, &labels()).is_empty());
    }

    #[test]
    fn the_list_is_capped_and_always_ends_with_the_web() {
        let apps: Vec<AppEntry> = (0..30).map(|i| app(&format!("App {i:02}"))).collect();
        let r = search("app", &apps, &[], Lang::En, &labels());
        assert_eq!(r.len(), MAX_RESULTS);
        assert_eq!(r.last().unwrap().kind, Kind::Web);
        assert_eq!(r[0].title, "App 00");
        // With a calculation too: still six.
        let r = search("1+1", &apps, &[], Lang::En, &labels());
        assert!(r.len() <= MAX_RESULTS);
        let many: Vec<AppEntry> = (0..30).map(|i| app(&format!("1+1 App {i}"))).collect();
        let r = search("1+1", &many, &[], Lang::En, &labels());
        assert_eq!(r.len(), MAX_RESULTS);
        assert_eq!(r[0].kind, Kind::Calc);
        assert_eq!(r.last().unwrap().kind, Kind::Web);
        // A line the web cannot take (control character) loses only that row.
        let r = search("app\u{7}", &apps, &[], Lang::En, &labels());
        assert!(r.iter().all(|r| r.kind != Kind::Web));
    }

    #[test]
    fn an_app_wins_a_tie_with_a_settings_page() {
        let apps = [app("Display Fusion")];
        let r = search("display", &apps, &[], Lang::En, &labels());
        assert_eq!((r[0].kind, r[1].kind), (Kind::App, Kind::Setting));
    }

    #[test]
    fn every_settings_page_has_both_names_and_a_settings_address() {
        for p in &SETTINGS_PAGES {
            assert!(!p.id.is_empty() && !p.en.is_empty(), "{}", p.uri);
            assert!(p.uri.starts_with("ms-settings:"), "{}", p.uri);
        }
        let mut uris: Vec<&str> = SETTINGS_PAGES.iter().map(|p| p.uri).collect();
        uris.sort_unstable();
        uris.dedup();
        assert_eq!(uris.len(), SETTINGS_PAGES.len());
    }

    #[test]
    fn the_index_keeps_apps_once_by_name_and_drops_uninstallers() {
        assert!(app_entry(Path::new(r"C:\M\Notepad.lnk")).is_some());
        assert!(app_entry(Path::new(r"C:\M\NOTEPAD.LNK")).is_some());
        assert!(app_entry(Path::new(r"C:\M\Uninstall Foo.lnk")).is_none());
        assert!(app_entry(Path::new(r"C:\M\Foo Help.lnk")).is_none());
        assert!(app_entry(Path::new(r"C:\M\notes.txt")).is_none());
        assert!(app_entry(Path::new(r"C:\M\.lnk")).is_none());
        let apps = dedupe(vec![
            AppEntry { name: "b app".into(), path: "1".into() },
            AppEntry { name: "A App".into(), path: "2".into() },
            AppEntry { name: "B App".into(), path: "3".into() },
        ]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["A App", "b app"], "first kept, by name");
    }

    #[test]
    fn the_index_walks_folders_to_a_depth_and_reads_only_shortcuts() {
        let dir = std::env::temp_dir().join(format!("vale-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let deep = dir.join("A").join("B");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.join("Top.lnk"), b"").unwrap();
        std::fs::write(dir.join("readme.txt"), b"").unwrap();
        std::fs::write(deep.join("Deep.lnk"), b"").unwrap();
        let mut out = Vec::new();
        collect(&dir, INDEX_DEPTH, &mut out);
        let mut names: Vec<String> = out.into_iter().map(|a| a.name).collect();
        names.sort();
        assert_eq!(names, ["Deep", "Top"]);
        let mut shallow = Vec::new();
        collect(&dir, 0, &mut shallow);
        assert_eq!(shallow.len(), 1, "no deeper than asked");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
