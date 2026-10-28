//! The island's own cleaner (the "Pilih sendiri" mode of the Bersihkan
//! panel, keputusan-v3.1 decision 4b option D).
//!
//! Seven categories, each a list of locations in `LOCATIONS`. Selection is
//! pure apart from reading the folders it is given: `select` takes the root
//! folders (`Roots`, so tests pass a temp tree), the time now and the set of
//! running exe names, and returns the files a clean would remove. Scanning
//! and deleting run on a parked worker thread (`Cleaner`), never on the UI
//! thread, and answer through `Event::Clean`.
//!
//! What is removed and how:
//! - cache categories: file by file, permanently; a locked or denied file
//!   is skipped and counted, never retried;
//! - old installers in Downloads: to the Recycle Bin (`IFileOperation`
//!   with undo), never permanently;
//! - Windows Update leftovers: by this exe started again as administrator
//!   (`--clean-admin windows-update`, UAC), never the whole island.
//!
//! Nothing follows a symlink, junction or other reparse point. Nothing is
//! deleted except on an explicit request the panel only sends after a
//! confirmed second click.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    FOFX_RECYCLEONDELETE, FOLDERID_Downloads, FOLDERID_LocalAppData, FOLDERID_Profile,
    FOLDERID_RoamingAppData, FOLDERID_Windows, FileOperation, IFileOperation, IShellItem,
    KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHGetKnownFolderPath, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{GUID, HSTRING, PCWSTR, w};

use crate::i18n::{self, CleanNote, Key, Lang, text};
use crate::model::{Event, Notice, NoticeKind};

/// The argument that starts this exe as the administrator helper, and the
/// one job it knows. `main` runs it without a window and exits.
pub const ADMIN_FLAG: &str = "--clean-admin";
pub const ADMIN_WINDOWS_UPDATE: &str = "windows-update";
/// The helper's exit code: 0..=`ADMIN_SKIPPED_MAX` is the number of files
/// it had to skip (capped); `ADMIN_FAILED` means it could not run its job.
pub const ADMIN_SKIPPED_MAX: u32 = 250;
pub const ADMIN_FAILED: i32 = 255;
/// How long the worker waits for the helper before giving up on it.
const ADMIN_WAIT_MS: u32 = 10 * 60 * 1000;
const DAY: u64 = 24 * 60 * 60;
/// `FILE_ATTRIBUTE_REPARSE_POINT`: symlinks, junctions, cloud placeholders.
const REPARSE_POINT: u32 = 0x400;

crate::theme::named_enum! {
    /// The cleaner's two modes (the segment in its header): the island's
    /// own ("Pilih sendiri", the default) or Mole's ("Otomatis · Mole",
    /// `crate::mole`). The last one used is remembered (`clean_mode`).
    CleanMode { Own, Mole }
}

/// The panel's rows, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CleanCat {
    /// Temp, logs and crash reports older than 7 days.
    TempLogs,
    /// Chrome, Edge, Firefox, Brave and Opera caches; an open browser is
    /// skipped and reported.
    BrowserCache,
    /// VS Code, Teams, Discord, Spotify, Zoom.
    AppCache,
    /// npm, pip, NuGet, Go, Cargo: downloaded again on the next build.
    DevCache,
    AutodeskAdobe,
    /// `.exe` / `.msi` in Downloads older than 30 days, to the Recycle Bin.
    OldInstallers,
    /// `SoftwareDistribution\Download`, through the administrator helper.
    WindowsUpdate,
}

impl CleanCat {
    pub const ALL: [CleanCat; 7] = [
        CleanCat::TempLogs,
        CleanCat::BrowserCache,
        CleanCat::AppCache,
        CleanCat::DevCache,
        CleanCat::AutodeskAdobe,
        CleanCat::OldInstallers,
        CleanCat::WindowsUpdate,
    ];

    /// The row (`ALL[i].index() == i`).
    pub fn index(self) -> usize {
        self as usize
    }

    /// Ticked when the panel opens: only what is safe and costs nothing to
    /// lose (the top three).
    pub fn checked_at_start(self) -> bool {
        matches!(
            self,
            CleanCat::TempLogs | CleanCat::BrowserCache | CleanCat::AppCache
        )
    }

    /// Needs the administrator helper (the ochre "admin" tag).
    pub fn admin(self) -> bool {
        self == CleanCat::WindowsUpdate
    }

    pub fn removal(self) -> Removal {
        match self {
            CleanCat::OldInstallers => Removal::RecycleBin,
            CleanCat::WindowsUpdate => Removal::Admin,
            _ => Removal::Permanent,
        }
    }
}

/// How a category's files go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// Caches: deleted file by file.
    Permanent,
    /// The user's own files: to the Recycle Bin.
    RecycleBin,
    /// System files: by the elevated helper.
    Admin,
}

/// A folder the locations hang from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    /// `%TEMP%`.
    Temp,
    /// `%LOCALAPPDATA%`.
    LocalAppData,
    /// `%APPDATA%`.
    RoamingAppData,
    /// `%USERPROFILE%`.
    Profile,
    /// The user's Downloads known folder.
    Downloads,
    /// `%WINDIR%`.
    Windows,
}

/// Where each `Root` is. The island reads the known folders; tests map
/// every root into a temp tree of their own.
#[derive(Clone, Debug, Default)]
pub struct Roots(HashMap<Root, PathBuf>);

impl Roots {
    pub fn with(mut self, root: Root, path: impl Into<PathBuf>) -> Self {
        self.0.insert(root, path.into());
        self
    }

    pub fn get(&self, root: Root) -> Option<&Path> {
        self.0.get(&root).map(PathBuf::as_path)
    }

    /// This user's folders, from `SHGetKnownFolderPath` (Temp from
    /// `GetTempPath`). A folder Windows cannot name is left out.
    pub fn known() -> Self {
        let mut roots = Roots::default();
        let known = [
            (Root::LocalAppData, FOLDERID_LocalAppData),
            (Root::RoamingAppData, FOLDERID_RoamingAppData),
            (Root::Profile, FOLDERID_Profile),
            (Root::Downloads, FOLDERID_Downloads),
            (Root::Windows, FOLDERID_Windows),
        ];
        for (root, id) in known {
            if let Some(path) = known_folder(&id) {
                roots = roots.with(root, path);
            }
        }
        roots.with(Root::Temp, std::env::temp_dir())
    }
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path.filter(|p| p.is_absolute())
    }
}

/// A browser whose caches are left alone while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Browser {
    pub name: &'static str,
    /// The process image name, compared without case.
    pub exe: &'static str,
}

pub const CHROME: Browser = Browser {
    name: "Chrome",
    exe: "chrome.exe",
};
pub const EDGE: Browser = Browser {
    name: "Edge",
    exe: "msedge.exe",
};
pub const FIREFOX: Browser = Browser {
    name: "Firefox",
    exe: "firefox.exe",
};
pub const BRAVE: Browser = Browser {
    name: "Brave",
    exe: "brave.exe",
};
pub const OPERA: Browser = Browser {
    name: "Opera",
    exe: "opera.exe",
};

/// How far under a location's folder files are taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// Every file in the folder and its subfolders.
    Tree,
    /// Only the files directly in it.
    Top,
}

/// One place a category cleans, and which of its files.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub cat: CleanCat,
    pub root: Root,
    /// Under the root, `\`-separated; empty is the root itself. A `*`
    /// segment is any one subfolder (each profile, each product version).
    pub path: &'static str,
    pub depth: Depth,
    /// Only these extensions (lowercase, no dot); empty takes any file.
    pub only: &'static [&'static str],
    /// Never these extensions: settings and sign-in kept beside a cache.
    pub keep: &'static [&'static str],
    /// Only files last modified more than this many days ago; 0 takes any.
    pub min_days: u64,
    /// Left alone while this browser runs.
    pub browser: Option<Browser>,
}

const fn tree(cat: CleanCat, root: Root, path: &'static str) -> Location {
    Location {
        cat,
        root,
        path,
        depth: Depth::Tree,
        only: &[],
        keep: &[],
        min_days: 0,
        browser: None,
    }
}

const fn older(days: u64, l: Location) -> Location {
    Location {
        min_days: days,
        ..l
    }
}

const fn of(browser: Browser, l: Location) -> Location {
    Location {
        browser: Some(browser),
        ..l
    }
}

const fn top_files(
    cat: CleanCat,
    root: Root,
    path: &'static str,
    only: &'static [&'static str],
    days: u64,
) -> Location {
    Location {
        depth: Depth::Top,
        only,
        min_days: days,
        ..tree(cat, root, path)
    }
}

use CleanCat::{
    AppCache, AutodeskAdobe, BrowserCache, DevCache, OldInstallers, TempLogs, WindowsUpdate,
};
use Root::{Downloads, LocalAppData, Profile, RoamingAppData, Temp, Windows};

/// Every location, by category.
///
/// The list follows Mole for Windows by tw93 (github.com/tw93/mole, branch
/// `windows`, `lib/clean/user.ps1`, `caches.ps1`, `dev.ps1`, `apps.ps1` and
/// `system.ps1` at commit 500dfc4c, version 1.30.0, the last release of
/// that branch under the MIT licence; see NOTICE). Only the data is taken
/// (which folders, which age rules); the code here is the island's own.
/// Where the island is narrower than Mole it says so:
/// - Temp & logs: only the user's own folders (no admin), 7 days as Mole.
/// - Teams keeps its databases, IndexedDB and Local Storage (sign-in).
/// - Zoom keeps its settings and databases (`keep`).
/// - NuGet: the HTTP and plugin caches, not the package folder.
/// - Firefox: Mole names the roaming profiles; the disk cache lives in the
///   local ones, so both are listed. npm likewise: Mole names the roaming
///   cache, npm's default is the local one.
/// - Windows Update: the download cache only, without stopping the service
///   (a file in use is skipped).
pub const LOCATIONS: &[Location] = &[
    // File sementara & log.
    older(7, tree(TempLogs, Temp, "")),
    older(7, tree(TempLogs, LocalAppData, "Microsoft\\Windows\\WER")),
    older(7, tree(TempLogs, LocalAppData, "CrashDumps")),
    top_files(TempLogs, RoamingAppData, "", &["log"], 7),
    top_files(TempLogs, Profile, "", &["log"], 7),
    // Cache browser.
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\Default\\Cache",
        ),
    ),
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\Default\\Code Cache",
        ),
    ),
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\Default\\GPUCache",
        ),
    ),
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\Default\\Service Worker\\CacheStorage",
        ),
    ),
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\ShaderCache",
        ),
    ),
    of(
        CHROME,
        tree(
            BrowserCache,
            LocalAppData,
            "Google\\Chrome\\User Data\\GrShaderCache",
        ),
    ),
    of(
        EDGE,
        tree(
            BrowserCache,
            LocalAppData,
            "Microsoft\\Edge\\User Data\\Default\\Cache",
        ),
    ),
    of(
        EDGE,
        tree(
            BrowserCache,
            LocalAppData,
            "Microsoft\\Edge\\User Data\\Default\\Code Cache",
        ),
    ),
    of(
        EDGE,
        tree(
            BrowserCache,
            LocalAppData,
            "Microsoft\\Edge\\User Data\\Default\\GPUCache",
        ),
    ),
    of(
        EDGE,
        tree(
            BrowserCache,
            LocalAppData,
            "Microsoft\\Edge\\User Data\\Default\\Service Worker\\CacheStorage",
        ),
    ),
    of(
        EDGE,
        tree(
            BrowserCache,
            LocalAppData,
            "Microsoft\\Edge\\User Data\\ShaderCache",
        ),
    ),
    of(
        FIREFOX,
        tree(
            BrowserCache,
            RoamingAppData,
            "Mozilla\\Firefox\\Profiles\\*\\cache2",
        ),
    ),
    of(
        FIREFOX,
        tree(
            BrowserCache,
            RoamingAppData,
            "Mozilla\\Firefox\\Profiles\\*\\startupCache",
        ),
    ),
    of(
        FIREFOX,
        tree(
            BrowserCache,
            RoamingAppData,
            "Mozilla\\Firefox\\Profiles\\*\\shader-cache",
        ),
    ),
    of(
        FIREFOX,
        tree(
            BrowserCache,
            LocalAppData,
            "Mozilla\\Firefox\\Profiles\\*\\cache2",
        ),
    ),
    of(
        FIREFOX,
        tree(
            BrowserCache,
            LocalAppData,
            "Mozilla\\Firefox\\Profiles\\*\\startupCache",
        ),
    ),
    of(
        BRAVE,
        tree(
            BrowserCache,
            LocalAppData,
            "BraveSoftware\\Brave-Browser\\User Data\\Default\\Cache",
        ),
    ),
    of(
        OPERA,
        tree(
            BrowserCache,
            RoamingAppData,
            "Opera Software\\Opera Stable\\Cache",
        ),
    ),
    // Cache aplikasi.
    tree(AppCache, RoamingAppData, "Code\\Cache"),
    tree(AppCache, RoamingAppData, "Code\\CachedData"),
    tree(AppCache, RoamingAppData, "Code\\CachedExtensions"),
    tree(AppCache, RoamingAppData, "Code\\CachedExtensionVSIXs"),
    tree(AppCache, RoamingAppData, "Code\\Code Cache"),
    tree(AppCache, RoamingAppData, "Code\\GPUCache"),
    tree(AppCache, RoamingAppData, "Microsoft\\Teams\\Cache"),
    tree(AppCache, RoamingAppData, "Microsoft\\Teams\\blob_storage"),
    tree(AppCache, RoamingAppData, "Microsoft\\Teams\\GPUCache"),
    tree(AppCache, RoamingAppData, "Microsoft\\Teams\\tmp"),
    tree(AppCache, RoamingAppData, "discord\\Cache"),
    tree(AppCache, RoamingAppData, "discord\\Code Cache"),
    tree(AppCache, RoamingAppData, "discord\\GPUCache"),
    tree(AppCache, LocalAppData, "Spotify\\Data"),
    tree(AppCache, LocalAppData, "Spotify\\Storage"),
    Location {
        keep: &["db", "enc", "ini", "json", "xml"],
        ..tree(AppCache, RoamingAppData, "Zoom\\data")
    },
    // Cache developer.
    tree(DevCache, RoamingAppData, "npm-cache"),
    tree(DevCache, LocalAppData, "npm-cache"),
    tree(DevCache, LocalAppData, "pip\\Cache"),
    tree(DevCache, LocalAppData, "NuGet\\v3-cache"),
    tree(DevCache, LocalAppData, "NuGet\\plugins-cache"),
    tree(DevCache, Profile, "go\\pkg\\mod\\cache"),
    tree(DevCache, Profile, ".cargo\\registry\\cache"),
    tree(DevCache, Profile, ".cargo\\git\\checkouts"),
    // Cache Autodesk & Adobe.
    tree(AutodeskAdobe, LocalAppData, "Autodesk\\*\\Cache"),
    tree(AutodeskAdobe, RoamingAppData, "Autodesk\\*\\cache"),
    tree(AutodeskAdobe, LocalAppData, "Adobe\\*\\Cache"),
    tree(AutodeskAdobe, LocalAppData, "Adobe\\*\\CameraRaw\\Cache"),
    tree(
        AutodeskAdobe,
        RoamingAppData,
        "Adobe\\Common\\Media Cache Files",
    ),
    tree(AutodeskAdobe, RoamingAppData, "Adobe\\Common\\Peak Files"),
    tree(
        AutodeskAdobe,
        RoamingAppData,
        "Adobe\\Common\\Team Projects Cache",
    ),
    // Installer lama di Downloads.
    top_files(OldInstallers, Downloads, "", &["exe", "msi"], 30),
    // Sisa Windows Update.
    tree(WindowsUpdate, Windows, "SoftwareDistribution\\Download"),
];

/// Lowercased exe names, for `select`.
pub fn running_set<I, S>(names: I) -> HashSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    names
        .into_iter()
        .map(|n| n.as_ref().to_ascii_lowercase())
        .collect()
}

/// `browser` has a process in `running` (lowercased by `running_set`).
pub fn browser_running(browser: Browser, running: &HashSet<String>) -> bool {
    running.contains(&browser.exe.to_ascii_lowercase())
}

/// Last modified more than `days` whole days before `now`. A time in the
/// future, or `days` 0 with any time, counts as old enough only for 0.
pub fn old_enough(modified: SystemTime, now: SystemTime, days: u64) -> bool {
    if days == 0 {
        return true;
    }
    now.duration_since(modified)
        .is_ok_and(|age| age > Duration::from_secs(days * DAY))
}

/// A file a clean would remove.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub cat: CleanCat,
    pub path: PathBuf,
    pub size: u64,
}

/// What `select` found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub targets: Vec<Target>,
    /// Per category (`CleanCat::index`): false when a folder of it exists
    /// but could not be read (Windows Update without admin): its size is
    /// unknown, not zero.
    pub measured: [bool; 7],
    /// Browsers left alone because they run, in table order.
    pub skipped_browsers: Vec<Browser>,
    /// The location folders the targets came from (for pruning empty
    /// folders afterwards).
    pub folders: Vec<(CleanCat, PathBuf)>,
}

/// The files `cats` would remove under `roots` at `now`, with the browsers
/// in `running` left out. Reads the folders; deletes nothing.
pub fn select(
    roots: &Roots,
    cats: &[CleanCat],
    now: SystemTime,
    running: &HashSet<String>,
) -> Selection {
    let mut sel = Selection {
        measured: [true; 7],
        ..Selection::default()
    };
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for loc in LOCATIONS.iter().filter(|l| cats.contains(&l.cat)) {
        if let Some(b) = loc.browser
            && browser_running(b, running)
        {
            if !sel.skipped_browsers.contains(&b) {
                sel.skipped_browsers.push(b);
            }
            continue;
        }
        let Some(root) = roots.get(loc.root) else {
            continue;
        };
        for dir in expand(root, loc.path) {
            match collect(&dir, loc, now) {
                Ok(files) => {
                    for (path, size) in files {
                        if seen.insert(path.clone()) {
                            sel.targets.push(Target {
                                cat: loc.cat,
                                path,
                                size,
                            });
                        }
                    }
                    sel.folders.push((loc.cat, dir));
                }
                Err(_) => sel.measured[loc.cat.index()] = false,
            }
        }
    }
    sel
}

/// Not a symlink, junction or other reparse point.
fn plain(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & REPARSE_POINT == 0
}

/// The existing folders `path` names under `root`: each segment must be a
/// real folder (never a reparse point), a `*` any one of them.
fn expand(root: &Path, path: &str) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for seg in path.split('\\').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for dir in &dirs {
            if seg == "*" {
                let Ok(entries) = fs::read_dir(dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if entry.metadata().is_ok_and(|m| m.is_dir() && plain(&m)) {
                        next.push(entry.path());
                    }
                }
            } else {
                let child = dir.join(seg);
                if fs::symlink_metadata(&child).is_ok_and(|m| m.is_dir() && plain(&m)) {
                    next.push(child);
                }
            }
        }
        dirs = next;
    }
    dirs.retain(|d| fs::symlink_metadata(d).is_ok_and(|m| m.is_dir() && plain(&m)));
    dirs
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The files under `dir` that `loc` takes, with their sizes. An error only
/// when `dir` itself cannot be listed; unreadable subfolders are passed by.
fn collect(dir: &Path, loc: &Location, now: SystemTime) -> std::io::Result<Vec<(PathBuf, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut first = true;
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(e) if first => return Err(e),
            Err(_) => continue,
        };
        first = false;
        for entry in entries.flatten() {
            // On Windows this is the entry itself, never what a link
            // points at.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !plain(&meta) {
                continue;
            }
            if meta.is_dir() {
                if loc.depth == Depth::Tree {
                    stack.push(entry.path());
                }
                continue;
            }
            let path = entry.path();
            let ext = ext_of(&path);
            if !loc.only.is_empty() && !loc.only.contains(&ext.as_str()) {
                continue;
            }
            if loc.keep.contains(&ext.as_str()) {
                continue;
            }
            let modified = meta.modified().unwrap_or(now);
            if !old_enough(modified, now, loc.min_days) {
                continue;
            }
            out.push((path, meta.len()));
        }
    }
    Ok(out)
}

/// One row's measured size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatSize {
    pub bytes: u64,
    pub files: u64,
    /// False: a folder could not be read; the row shows "—".
    pub measured: bool,
}

/// What a scan measured, per row, and the browsers it left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanResult {
    pub cats: [CatSize; 7],
    pub skipped_browsers: Vec<&'static str>,
}

pub fn summarize(sel: &Selection) -> ScanResult {
    let mut cats = [CatSize::default(); 7];
    for (i, c) in cats.iter_mut().enumerate() {
        c.measured = sel.measured[i];
    }
    for t in &sel.targets {
        let c = &mut cats[t.cat.index()];
        c.bytes = c.bytes.saturating_add(t.size);
        c.files += 1;
    }
    ScanResult {
        cats,
        skipped_browsers: sel.skipped_browsers.iter().map(|b| b.name).collect(),
    }
}

/// How a permanent delete went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub removed: u64,
    pub bytes: u64,
    /// Locked, denied or gone meanwhile.
    pub skipped: u32,
}

/// Deletes each target once. A file that cannot go (in use, denied, read
/// only, turned into a link meanwhile) is counted and left; nothing is
/// retried.
pub fn delete_files(targets: &[Target]) -> Tally {
    let mut tally = Tally::default();
    for t in targets {
        let still_plain = fs::symlink_metadata(&t.path).is_ok_and(|m| m.is_file() && plain(&m));
        if still_plain && fs::remove_file(&t.path).is_ok() {
            tally.removed += 1;
            tally.bytes = tally.bytes.saturating_add(t.size);
        } else {
            tally.skipped += 1;
        }
    }
    tally
}

/// Removes the empty folders under `dir` (not `dir` itself), deepest first.
/// Folders that are not empty, links and errors are left.
pub fn prune_empty(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.metadata().is_ok_and(|m| m.is_dir() && plain(&m)) {
            let sub = entry.path();
            prune_empty(&sub);
            let _ = fs::remove_dir(&sub);
        }
    }
}

/// Moves `paths` to the Recycle Bin in one `IFileOperation` (COM must be
/// initialised on this thread). Never deletes permanently: a file too big
/// for the bin brings up the shell's own warning instead. Returns how many
/// are still there afterwards (in use, refused, or kept at that warning).
pub fn recycle(paths: &[PathBuf]) -> u32 {
    if paths.is_empty() {
        return 0;
    }
    let _ = unsafe { recycle_all(paths) };
    paths.iter().filter(|p| p.exists()).count() as u32
}

unsafe fn recycle_all(paths: &[PathBuf]) -> windows::core::Result<()> {
    unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOperationFlags(
            FOF_ALLOWUNDO
                | FOFX_RECYCLEONDELETE
                | FOF_WANTNUKEWARNING
                | FOF_NOCONFIRMATION
                | FOF_NOERRORUI
                | FOF_SILENT,
        )?;
        let mut queued = 0;
        for p in paths {
            let name = HSTRING::from(p.as_os_str());
            let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&name, None) else {
                continue;
            };
            if op.DeleteItem(&item, None).is_ok() {
                queued += 1;
            }
        }
        if queued > 0 {
            op.PerformOperations()?;
        }
        Ok(())
    }
}

/// How the administrator part went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdminOutcome {
    /// Windows Update was not ticked.
    #[default]
    NotAsked,
    /// The helper ran; this many files were in use.
    Done { skipped: u32 },
    /// UAC was refused: nothing happened.
    Cancelled,
    /// The helper could not start or failed.
    Failed,
}

/// The helper's exit code read back.
pub fn admin_outcome(code: u32) -> AdminOutcome {
    if code <= ADMIN_SKIPPED_MAX {
        AdminOutcome::Done { skipped: code }
    } else {
        AdminOutcome::Failed
    }
}

/// Starts this exe elevated (`runas`, UAC) with `ADMIN_FLAG`, waits for it
/// and reads its exit code. A refused UAC prompt is `Cancelled`.
fn run_admin_helper() -> AdminOutcome {
    let Ok(exe) = std::env::current_exe() else {
        return AdminOutcome::Failed;
    };
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(format!("{ADMIN_FLAG} {ADMIN_WINDOWS_UPDATE}"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return if e.code() == ERROR_CANCELLED.to_hresult() {
            AdminOutcome::Cancelled
        } else {
            AdminOutcome::Failed
        };
    }
    if info.hProcess.is_invalid() {
        return AdminOutcome::Failed;
    }
    let outcome = unsafe {
        if WaitForSingleObject(info.hProcess, ADMIN_WAIT_MS) == WAIT_OBJECT_0 {
            let mut code = 0u32;
            match GetExitCodeProcess(info.hProcess, &mut code) {
                Ok(()) => admin_outcome(code),
                Err(_) => AdminOutcome::Failed,
            }
        } else {
            AdminOutcome::Failed
        }
    };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    outcome
}

/// `--clean-admin <job>`, already elevated: removes the Windows Update
/// download leftovers and nothing else, then returns the exit code
/// (`admin_outcome` reads it). No window, no island.
pub fn run_admin(job: Option<&str>) -> i32 {
    if job != Some(ADMIN_WINDOWS_UPDATE) {
        return ADMIN_FAILED;
    }
    let roots = Roots::known();
    if roots.get(Root::Windows).is_none() {
        return ADMIN_FAILED;
    }
    let sel = select(
        &roots,
        &[CleanCat::WindowsUpdate],
        SystemTime::now(),
        &HashSet::new(),
    );
    if !sel.measured[CleanCat::WindowsUpdate.index()] {
        return ADMIN_FAILED;
    }
    let tally = delete_files(&sel.targets);
    for (_, dir) in &sel.folders {
        prune_empty(dir);
    }
    tally.skipped.min(ADMIN_SKIPPED_MAX) as i32
}

/// Free bytes on `letter`'s drive.
pub(crate) fn drive_free(letter: char) -> Option<u64> {
    let root = HSTRING::from(format!("{letter}:\\"));
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), None, None) }.ok()?;
    Some(free)
}

fn drive_of(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let mut chars = s.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    (letter.is_ascii_alphabetic() && chars.next() == Some(':')).then_some(letter)
}

/// What the island tells after a clean.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CleanReport {
    /// The free space gained, measured on the drives before and after.
    pub freed: u64,
    /// The drive that gained the most, and its free space now.
    pub drive: Option<(char, u64)>,
    /// Files in use or denied, left in place.
    pub skipped: u32,
    pub skipped_browsers: Vec<&'static str>,
    pub admin: AdminOutcome,
}

/// Free space before and after, per drive: the total gained and the drive
/// that gained the most (ties: the first). A drive that lost space
/// meanwhile gained nothing.
pub fn measure_freed(drives: &[(char, u64, u64)]) -> (u64, Option<(char, u64)>) {
    let mut freed = 0u64;
    let mut best: Option<(char, u64, u64)> = None;
    for &(letter, before, after) in drives {
        let gain = after.saturating_sub(before);
        freed = freed.saturating_add(gain);
        if best.is_none_or(|(_, g, _)| gain > g) {
            best = Some((letter, gain, after));
        }
    }
    (freed, best.map(|(l, _, after)| (l, after)))
}

/// The notice after a clean (Bata happy): what was freed, the drive's free
/// space now, and what was left. `fits` says whether a body fits the
/// notice; what would not fit is left out, the least important first.
pub fn done_notice(lang: Lang, report: &CleanReport, fits: &dyn Fn(&str) -> bool) -> Notice {
    let note = CleanNote {
        drive: report.drive,
        skipped: report.skipped,
        browsers: &report.skipped_browsers,
        admin_skipped: matches!(report.admin, AdminOutcome::Cancelled | AdminOutcome::Failed),
    };
    Notice {
        key: "clean-done".into(),
        source: text(lang, Key::SystemClean).into(),
        title: i18n::clean_freed(lang, report.freed),
        body: i18n::clean_done_body(lang, &note, fits),
        kind: NoticeKind::CleanDone,
    }
}

/// Exe names of the running processes, lowercased.
pub fn process_names() -> HashSet<String> {
    let mut names = Vec::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return HashSet::new();
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    running_set(names)
}

// ---- The worker ----

#[derive(Clone, Debug, PartialEq)]
enum Request {
    Scan,
    Delete(Vec<CleanCat>),
}

/// The worker's answers.
#[derive(Clone, Debug, PartialEq)]
pub enum CleanEvent {
    Scanned(ScanResult),
    Done(CleanReport),
}

/// The cleaning thread, started on the first request. It waits on its
/// channel and handles each request in order; dropping it ends the thread.
pub struct Cleaner {
    requests: Sender<Request>,
}

impl Cleaner {
    /// `None` when the thread could not start.
    pub fn spawn(tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> Option<Self> {
        let (requests, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("cleaner".into())
            .spawn(move || serve(&rx, &tx, wake));
        match spawned {
            Ok(_) => Some(Self { requests }),
            Err(error) => {
                eprintln!("vale: no cleaner: {error}");
                None
            }
        }
    }

    /// Measures every category. False when the thread is gone.
    pub fn scan(&self) -> bool {
        self.requests.send(Request::Scan).is_ok()
    }

    /// Cleans `cats`. Only for a confirmed request.
    pub fn delete(&self, cats: Vec<CleanCat>) -> bool {
        self.requests.send(Request::Delete(cats)).is_ok()
    }
}

fn serve(rx: &Receiver<Request>, tx: &Sender<Event>, wake: impl Fn()) {
    // IFileOperation wants a single-threaded apartment.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    while let Ok(request) = rx.recv() {
        let event = match request {
            Request::Scan => {
                let sel = select(
                    &Roots::known(),
                    &CleanCat::ALL,
                    SystemTime::now(),
                    &process_names(),
                );
                CleanEvent::Scanned(summarize(&sel))
            }
            Request::Delete(cats) => CleanEvent::Done(clean(&cats)),
        };
        if tx.send(Event::Clean(event)).is_err() {
            return;
        }
        wake();
    }
}

/// The delete itself: the files chosen again now (the browsers checked
/// again just before), the drives measured before and after.
fn clean(cats: &[CleanCat]) -> CleanReport {
    let roots = Roots::known();
    let user: Vec<CleanCat> = cats
        .iter()
        .copied()
        .filter(|c| c.removal() != Removal::Admin)
        .collect();
    let sel = select(&roots, &user, SystemTime::now(), &process_names());
    let mut letters: Vec<char> = sel
        .targets
        .iter()
        .filter_map(|t| drive_of(&t.path))
        .collect();
    let admin = cats.iter().any(|c| c.removal() == Removal::Admin);
    if admin && let Some(l) = roots.get(Root::Windows).and_then(drive_of) {
        letters.push(l);
    }
    letters.sort_unstable();
    letters.dedup();
    let before: Vec<(char, u64)> = letters
        .iter()
        .filter_map(|l| drive_free(*l).map(|f| (*l, f)))
        .collect();

    let (permanent, recycled): (Vec<Target>, Vec<Target>) = sel
        .targets
        .into_iter()
        .partition(|t| t.cat.removal() == Removal::Permanent);
    let tally = delete_files(&permanent);
    let paths: Vec<PathBuf> = recycled.into_iter().map(|t| t.path).collect();
    let mut skipped = tally.skipped.saturating_add(recycle(&paths));
    for (cat, dir) in &sel.folders {
        if cat.removal() == Removal::Permanent {
            prune_empty(dir);
        }
    }
    let admin = if admin {
        run_admin_helper()
    } else {
        AdminOutcome::NotAsked
    };
    if let AdminOutcome::Done { skipped: s } = admin {
        skipped = skipped.saturating_add(s);
    }

    let drives: Vec<(char, u64, u64)> = before
        .iter()
        .map(|&(l, b)| (l, b, drive_free(l).unwrap_or(b)))
        .collect();
    let (freed, drive) = measure_freed(&drives);
    CleanReport {
        freed,
        drive,
        skipped,
        skipped_browsers: sel.skipped_browsers.iter().map(|b| b.name).collect(),
        admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A tree of its own under `target/`, removed when dropped. Every root
    /// points into it, so no test touches the real Temp, Downloads,
    /// AppData or browser folders.
    struct TestTree {
        base: PathBuf,
    }

    impl TestTree {
        fn new(name: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("clean-tests")
                .join(format!(
                    "{name}-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&base).expect("test tree");
            Self { base }
        }

        fn roots(&self) -> Roots {
            [
                (Root::Temp, "Temp"),
                (Root::LocalAppData, "Local"),
                (Root::RoamingAppData, "Roaming"),
                (Root::Profile, "Profile"),
                (Root::Downloads, "Downloads"),
                (Root::Windows, "Windows"),
            ]
            .into_iter()
            .fold(Roots::default(), |r, (root, dir)| {
                let path = self.base.join(dir);
                fs::create_dir_all(&path).expect("root");
                r.with(root, path)
            })
        }

        /// A file of `len` bytes, last modified `age` ago.
        fn file(&self, rel: &str, len: usize, age: Duration) -> PathBuf {
            let path = self.base.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut f = File::create(&path).unwrap();
            f.write_all(&vec![7u8; len]).unwrap();
            f.set_modified(SystemTime::now() - age).unwrap();
            path
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn days(d: f64) -> Duration {
        Duration::from_secs_f64(d * DAY as f64)
    }

    fn paths(sel: &Selection, cat: CleanCat) -> Vec<PathBuf> {
        let mut p: Vec<PathBuf> = sel
            .targets
            .iter()
            .filter(|t| t.cat == cat)
            .map(|t| t.path.clone())
            .collect();
        p.sort();
        p
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn table_is_well_formed() {
        for (i, c) in CleanCat::ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
            assert!(
                LOCATIONS.iter().any(|l| l.cat == *c),
                "{c:?} has no location"
            );
        }
        for l in LOCATIONS {
            assert!(!l.path.contains('/') && !l.path.contains(".."), "{l:?}");
            assert!(
                !l.path.starts_with('\\') && !l.path.ends_with('\\'),
                "{l:?}"
            );
            assert_eq!(
                l.browser.is_some(),
                l.cat == CleanCat::BrowserCache,
                "{l:?}"
            );
        }
        let defaults: Vec<CleanCat> = CleanCat::ALL
            .into_iter()
            .filter(|c| c.checked_at_start())
            .collect();
        assert_eq!(
            defaults,
            [
                CleanCat::TempLogs,
                CleanCat::BrowserCache,
                CleanCat::AppCache
            ]
        );
        assert_eq!(CleanCat::OldInstallers.removal(), Removal::RecycleBin);
        assert_eq!(CleanCat::WindowsUpdate.removal(), Removal::Admin);
        assert!(CleanCat::WindowsUpdate.admin() && !CleanCat::DevCache.admin());
        for c in [
            CleanCat::TempLogs,
            CleanCat::BrowserCache,
            CleanCat::DevCache,
        ] {
            assert_eq!(c.removal(), Removal::Permanent);
        }
    }

    #[test]
    fn age_rule_at_seven_and_thirty_days() {
        let now = SystemTime::now();
        let ago = |d: f64| now - days(d);
        assert!(!old_enough(ago(6.99), now, 7));
        assert!(!old_enough(ago(7.0), now, 7), "exactly 7 days is not older");
        assert!(old_enough(ago(7.01), now, 7));
        assert!(!old_enough(ago(29.99), now, 30));
        assert!(old_enough(ago(30.01), now, 30));
        // A clock in the future never counts as old; no rule takes all.
        assert!(!old_enough(now + days(1.0), now, 7));
        assert!(old_enough(now, now, 0));

        let t = TestTree::new("age");
        let roots = t.roots();
        let young = t.file("Temp\\young.tmp", 10, days(6.9));
        let old = t.file("Temp\\sub\\old.tmp", 20, days(7.1));
        let inst_young = t.file("Downloads\\setup-new.exe", 30, days(29.9));
        let inst_old = t.file("Downloads\\setup-old.exe", 40, days(30.1));
        let sel = select(
            &roots,
            &[CleanCat::TempLogs, CleanCat::OldInstallers],
            SystemTime::now(),
            &none(),
        );
        assert_eq!(paths(&sel, CleanCat::TempLogs), [old]);
        assert_eq!(paths(&sel, CleanCat::OldInstallers), [inst_old]);
        assert!(young.exists() && inst_young.exists());
    }

    #[test]
    fn downloads_only_takes_exe_and_msi_at_the_top() {
        let t = TestTree::new("downloads");
        let roots = t.roots();
        let old = days(45.0);
        let exe = t.file("Downloads\\Setup.EXE", 5, old);
        let msi = t.file("Downloads\\tool.msi", 5, old);
        t.file("Downloads\\photo.jpg", 5, old);
        t.file("Downloads\\archive.zip", 5, old);
        t.file("Downloads\\notes.exe.txt", 5, old);
        t.file("Downloads\\nested\\deep.exe", 5, old);
        let sel = select(
            &roots,
            &[CleanCat::OldInstallers],
            SystemTime::now(),
            &none(),
        );
        let mut want = vec![exe, msi];
        want.sort();
        assert_eq!(paths(&sel, CleanCat::OldInstallers), want);
        // Nothing of another category.
        assert!(sel.targets.iter().all(|t| t.cat == CleanCat::OldInstallers));
    }

    #[test]
    fn an_open_browser_is_skipped_and_reported() {
        let t = TestTree::new("browser");
        let roots = t.roots();
        let chrome = t.file(
            "Local\\Google\\Chrome\\User Data\\Default\\Cache\\Cache_Data\\f_000001",
            100,
            Duration::ZERO,
        );
        let edge = t.file(
            "Local\\Microsoft\\Edge\\User Data\\Default\\Code Cache\\js\\index",
            50,
            Duration::ZERO,
        );
        let firefox = t.file(
            "Roaming\\Mozilla\\Firefox\\Profiles\\ab12.default\\cache2\\entries\\E1",
            70,
            Duration::ZERO,
        );
        let cats = [CleanCat::BrowserCache];
        let all = select(&roots, &cats, SystemTime::now(), &none());
        let mut want = vec![chrome.clone(), edge.clone(), firefox.clone()];
        want.sort();
        assert_eq!(paths(&all, CleanCat::BrowserCache), want);
        assert!(all.skipped_browsers.is_empty());
        // Chrome runs (any case): its cache is left and named.
        let running = running_set(["CHROME.EXE", "explorer.exe"]);
        let sel = select(&roots, &cats, SystemTime::now(), &running);
        let mut want = vec![edge, firefox];
        want.sort();
        assert_eq!(paths(&sel, CleanCat::BrowserCache), want);
        assert_eq!(sel.skipped_browsers, [CHROME]);
        assert_eq!(summarize(&sel).skipped_browsers, ["Chrome"]);
        assert!(browser_running(EDGE, &running_set(["MsEdge.exe"])));
        assert!(!browser_running(EDGE, &running_set(["msedgewebview2.exe"])));
    }

    #[test]
    fn scan_totals_on_a_built_tree() {
        let t = TestTree::new("totals");
        let roots = t.roots();
        let old = days(10.0);
        t.file("Temp\\a.tmp", 1000, old);
        t.file("Temp\\deep\\er\\b.log", 500, old);
        t.file("Temp\\fresh.tmp", 999, Duration::ZERO);
        t.file("Local\\CrashDumps\\app.dmp", 250, old);
        t.file("Roaming\\old.log", 40, old);
        t.file("Roaming\\keep.txt", 40, old);
        t.file("Roaming\\Code\\Cache\\x", 300, Duration::ZERO);
        t.file("Roaming\\discord\\GPUCache\\y", 200, Duration::ZERO);
        t.file("Roaming\\Zoom\\data\\img.cache", 60, Duration::ZERO);
        t.file("Roaming\\Zoom\\data\\zoomus.enc", 60, Duration::ZERO);
        t.file(
            "Profile\\.cargo\\registry\\cache\\idx\\serde.crate",
            4096,
            Duration::ZERO,
        );
        t.file(
            "Local\\Autodesk\\AutoCAD 2025\\Cache\\c1",
            77,
            Duration::ZERO,
        );
        t.file(
            "Windows\\SoftwareDistribution\\Download\\kb\\u.cab",
            900,
            Duration::ZERO,
        );
        let sel = select(&roots, &CleanCat::ALL, SystemTime::now(), &none());
        let scan = summarize(&sel);
        let size = |c: CleanCat| scan.cats[c.index()];
        assert_eq!(
            size(CleanCat::TempLogs),
            CatSize {
                bytes: 1000 + 500 + 250 + 40,
                files: 4,
                measured: true
            }
        );
        // Zoom's settings file stays.
        assert_eq!(size(CleanCat::AppCache).bytes, 300 + 200 + 60);
        assert_eq!(size(CleanCat::DevCache).bytes, 4096);
        assert_eq!(size(CleanCat::AutodeskAdobe).bytes, 77);
        assert_eq!(size(CleanCat::WindowsUpdate).bytes, 900);
        assert_eq!(
            size(CleanCat::BrowserCache),
            CatSize {
                bytes: 0,
                files: 0,
                measured: true
            }
        );
        assert_eq!(size(CleanCat::OldInstallers).files, 0);
        // A category with a missing root is simply empty.
        let partial = Roots::default().with(Root::Temp, t.base.join("Temp"));
        let s = summarize(&select(
            &partial,
            &CleanCat::ALL,
            SystemTime::now(),
            &none(),
        ));
        assert_eq!(s.cats[CleanCat::TempLogs.index()].files, 2);
        assert!(s.cats.iter().all(|c| c.measured));
    }

    #[test]
    fn delete_removes_targets_and_skips_a_locked_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = TestTree::new("locked");
        let roots = t.roots();
        let free = t.file("Roaming\\Code\\Cache\\free.bin", 10, Duration::ZERO);
        let locked = t.file("Roaming\\Code\\Cache\\sub\\locked.bin", 20, Duration::ZERO);
        let outside = t.file("Roaming\\Code\\settings.json", 5, Duration::ZERO);
        // Held open with no sharing, as a running app would.
        let holder = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .unwrap();
        let sel = select(&roots, &[CleanCat::AppCache], SystemTime::now(), &none());
        assert_eq!(sel.targets.len(), 2);
        let tally = delete_files(&sel.targets);
        assert_eq!(
            tally,
            Tally {
                removed: 1,
                bytes: 10,
                skipped: 1
            }
        );
        assert!(!free.exists() && locked.exists() && outside.exists());
        drop(holder);
        for (_, dir) in &sel.folders {
            prune_empty(dir);
        }
        // The folder with the skipped file stays; the location itself too.
        assert!(locked.parent().unwrap().exists());
        assert!(t.base.join("Roaming\\Code\\Cache").exists());
        // Once free, the next clean takes it and prunes its folder.
        let again = select(&roots, &[CleanCat::AppCache], SystemTime::now(), &none());
        assert_eq!(delete_files(&again.targets).skipped, 0);
        for (_, dir) in &again.folders {
            prune_empty(dir);
        }
        assert!(!locked.parent().unwrap().exists());
        assert!(t.base.join("Roaming\\Code\\Cache").exists());
    }

    /// A junction from `link` to `target` (no privilege needed), or a
    /// directory symlink; `false` when neither can be made here.
    fn make_link(link: &Path, target: &Path) -> bool {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
            && link.exists()
    }

    #[test]
    fn links_are_never_followed() {
        let t = TestTree::new("links");
        let roots = t.roots();
        let precious = t.file("Elsewhere\\precious.doc", 123, days(60.0));
        let real = t.file("Temp\\real.tmp", 10, days(60.0));
        let elsewhere = t.base.join("Elsewhere");
        // A link inside a location, and a location that is itself a link.
        let inside = t.base.join("Temp\\link");
        let as_location = t.base.join("Roaming\\Code\\Cache");
        fs::create_dir_all(as_location.parent().unwrap()).unwrap();
        if !make_link(&inside, &elsewhere) || !make_link(&as_location, &elsewhere) {
            eprintln!("links_are_never_followed: cannot make links here, skipped");
            return;
        }
        let sel = select(
            &roots,
            &[CleanCat::TempLogs, CleanCat::AppCache],
            SystemTime::now(),
            &none(),
        );
        assert_eq!(paths(&sel, CleanCat::TempLogs), [real]);
        assert!(paths(&sel, CleanCat::AppCache).is_empty());
        delete_files(&sel.targets);
        for (_, dir) in &sel.folders {
            prune_empty(dir);
        }
        assert!(precious.exists(), "nothing behind a link is touched");
        assert!(inside.exists() && as_location.exists());
        // Remove the links before the tree goes (never through them).
        let _ = fs::remove_dir(&inside);
        let _ = fs::remove_dir(&as_location);
        assert!(precious.exists());
    }

    #[test]
    fn freed_space_is_the_measured_gain() {
        assert_eq!(measure_freed(&[]), (0, None));
        let (freed, drive) = measure_freed(&[('C', 100, 350), ('D', 500, 520)]);
        assert_eq!((freed, drive), (270, Some(('C', 350))));
        // A drive that filled up meanwhile gains nothing.
        let (freed, drive) = measure_freed(&[('C', 100, 90), ('D', 500, 505)]);
        assert_eq!((freed, drive), (5, Some(('D', 505))));
        assert_eq!(drive_of(Path::new("c:\\Users\\x")), Some('C'));
        assert_eq!(drive_of(Path::new("\\\\server\\share")), None);
    }

    #[test]
    fn the_done_notice_tells_what_was_freed_and_left() {
        let gib = 1024 * 1024 * 1024u64;
        let report = CleanReport {
            freed: gib * 23 / 10,
            drive: Some(('C', gib * 495 / 10)),
            skipped: 3,
            skipped_browsers: vec![],
            admin: AdminOutcome::NotAsked,
        };
        let all = |_: &str| true;
        let n = done_notice(Lang::Id, &report, &all);
        assert_eq!(n.kind, NoticeKind::CleanDone);
        assert_eq!(n.title, "2,3 GB dibebaskan");
        assert_eq!(
            n.body,
            "Disk C: sekarang sisa 49,5 GB · 3 item dilewati (sedang dipakai)"
        );
        assert_eq!(n.source, "Bersihkan");
        // The notice's 360 px decide (`Renderer::notice_text_fits`): here
        // a stand-in of 40 characters takes the shorter wording.
        let narrow = |s: &str| s.chars().count() <= 40;
        let n = done_notice(Lang::Id, &report, &narrow);
        assert_eq!(n.body, "C: sisa 49,5 GB · 3 item dilewati");
        let refused = CleanReport {
            skipped: 0,
            admin: AdminOutcome::Cancelled,
            skipped_browsers: vec!["Edge"],
            ..report
        };
        let n = done_notice(Lang::En, &refused, &all);
        assert_eq!(n.title, "2.3 GB freed");
        assert_eq!(
            n.body,
            "Disk C: now 49.5 GB free · Edge open · Windows Update skipped"
        );
    }

    #[test]
    fn helper_exit_codes() {
        assert_eq!(admin_outcome(0), AdminOutcome::Done { skipped: 0 });
        assert_eq!(admin_outcome(7), AdminOutcome::Done { skipped: 7 });
        assert_eq!(
            admin_outcome(ADMIN_SKIPPED_MAX),
            AdminOutcome::Done {
                skipped: ADMIN_SKIPPED_MAX
            }
        );
        assert_eq!(admin_outcome(ADMIN_FAILED as u32), AdminOutcome::Failed);
        // An unknown job does nothing and fails, before reading any folder.
        assert_eq!(run_admin(None), ADMIN_FAILED);
        assert_eq!(run_admin(Some("everything")), ADMIN_FAILED);
    }
}
