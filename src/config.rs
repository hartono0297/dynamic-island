use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

use crate::i18n::Lang;
use crate::pomodoro::PomoSettings;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Pill,
    Notch,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub style: Style,
    pub battery_threshold: u8,
    pub start_with_windows: bool,
    pub muted_apps: Vec<String>,
    pub onboarding_done: bool,
    pub pomodoro: PomoSettings,
    /// OAuth desktop-client credentials for Google Calendar; empty until the user sets them.
    pub google_client_id: String,
    pub google_client_secret: String,
    /// UI language, "id" or "en".
    pub language: Lang,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            style: Style::Pill,
            battery_threshold: 20,
            start_with_windows: true,
            muted_apps: vec!["Claude".into(), "Orca".into()],
            onboarding_done: false,
            pomodoro: PomoSettings::default(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            language: Lang::Id,
        }
    }
}

/// Missing or malformed config falls back to defaults rather than failing startup.
/// A leading UTF-8 BOM (Notepad and PowerShell 5.1 may write one) is skipped.
pub fn parse(text: &str) -> Config {
    try_parse(text).unwrap_or_default()
}

/// `None` when `text` is not a config.
pub fn try_parse(text: &str) -> Option<Config> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str(text).ok()
}

/// config.json as found on disk.
#[derive(Debug, PartialEq)]
pub enum OnDisk {
    /// No file, or nothing in it but whitespace (or a BOM): nothing to keep.
    Missing,
    /// Text that is not a config (a typo while editing by hand), or a file
    /// that cannot be read.
    Invalid,
    Valid(Config),
}

pub fn classify(read: io::Result<String>) -> OnDisk {
    match read {
        Ok(text) => {
            let body = text.strip_prefix('\u{feff}').unwrap_or(&text);
            if body.trim().is_empty() {
                OnDisk::Missing
            } else {
                try_parse(body).map_or(OnDisk::Invalid, OnDisk::Valid)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => OnDisk::Missing,
        Err(_) => OnDisk::Invalid,
    }
}

pub fn read_at(path: &Path) -> OnDisk {
    classify(std::fs::read_to_string(path))
}

/// The island's own settings (`mem`: style, autostart, onboarding, pomodoro,
/// language) over everything else as it is on disk (Google client, muted apps, battery
/// threshold), which the owner may have edited by hand while the island ran.
pub fn merge_owned(disk: Config, mem: &Config) -> Config {
    Config {
        style: mem.style,
        start_with_windows: mem.start_with_windows,
        onboarding_done: mem.onboarding_done,
        pomodoro: mem.pomodoro,
        language: mem.language,
        ..disk
    }
}

/// What a save found wrong with the file it replaced.
#[derive(Debug, PartialEq)]
pub enum Repair {
    /// The file was valid or absent.
    None,
    /// An invalid file was moved here before the island's config was written.
    BackedUp(PathBuf),
    /// An invalid file could not be moved aside, so nothing was written; the
    /// next save tries again.
    Blocked,
}

pub struct SaveOutcome {
    /// The config now in effect: the file's hand-edited fields under the
    /// island's own settings, or the island's config when the file had none.
    pub config: Config,
    pub repair: Repair,
}

/// Saves the island's settings `mem` to `path` without losing anything there:
/// a valid file keeps its other fields (`merge_owned`), a missing or empty one
/// gets `mem`, and an invalid one is renamed to a backup (`backup_path`) first.
/// Write errors only happen on a broken profile; the setting still holds for
/// this run, so they are not reported.
pub fn save_merged_at(path: &Path, mem: &Config) -> SaveOutcome {
    let (config, repair) = match read_at(path) {
        OnDisk::Valid(disk) => (merge_owned(disk, mem), Repair::None),
        OnDisk::Missing => (mem.clone(), Repair::None),
        OnDisk::Invalid => {
            let backup = backup_path(path);
            match std::fs::rename(path, &backup) {
                Ok(()) => (mem.clone(), Repair::BackedUp(backup)),
                // Gone meanwhile: nothing left to keep.
                Err(e) if e.kind() == io::ErrorKind::NotFound => (mem.clone(), Repair::None),
                Err(_) => {
                    return SaveOutcome {
                        config: mem.clone(),
                        repair: Repair::Blocked,
                    };
                }
            }
        }
    };
    let _ = config.save_to(path);
    SaveOutcome { config, repair }
}

pub fn save_merged(mem: &Config) -> SaveOutcome {
    save_merged_at(&path(), mem)
}

/// `<file>.bak`, or `.bak2`, `.bak3`… when earlier backups exist, so a
/// backup is never overwritten.
pub fn backup_path(path: &Path) -> PathBuf {
    let with = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    let first = with(".bak");
    if !first.exists() {
        return first;
    }
    (2..1000)
        .map(|n| with(&format!(".bak{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// `%APPDATA%\DynamicIsland\config.json`, or the current directory when APPDATA is unset.
pub fn path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("DynamicIsland")
        .join("config.json")
}

pub fn load() -> Config {
    parse(&std::fs::read_to_string(path()).unwrap_or_default())
}

impl Config {
    /// Pretty JSON to a sibling `.tmp` file, then renamed over `path`, so a
    /// crash mid-write never leaves a truncated config.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_default() {
        assert_eq!(parse(""), Config::default());
    }

    #[test]
    fn invalid_json_is_default() {
        assert_eq!(parse("{not json"), Config::default());
    }

    #[test]
    fn leading_byte_order_mark_is_ignored() {
        let c = parse("\u{feff}{\"style\":\"Notch\",\"battery_threshold\":30}");
        assert_eq!(c.style, Style::Notch);
        assert_eq!(c.battery_threshold, 30);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let c = parse(r#"{"style":"Notch"}"#);
        assert_eq!(c.style, Style::Notch);
        assert_eq!(
            c,
            Config {
                style: Style::Notch,
                ..Config::default()
            }
        );
    }

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.style, Style::Pill);
        assert_eq!(c.battery_threshold, 20);
        assert!(c.start_with_windows);
        assert_eq!(c.muted_apps, vec!["Claude".to_string(), "Orca".to_string()]);
        assert!(!c.onboarding_done);
        assert_eq!(c.language, Lang::Id);
    }

    #[test]
    fn round_trips_through_json() {
        let c = Config {
            style: Style::Notch,
            battery_threshold: 35,
            start_with_windows: false,
            muted_apps: vec!["Slack".into()],
            onboarding_done: true,
            pomodoro: PomoSettings {
                focus_min: 50,
                short_break_min: 10,
                long_break_min: 30,
                sessions: 3,
            },
            google_client_id: "id.apps.googleusercontent.com".into(),
            google_client_secret: "secret".into(),
            language: Lang::En,
        };
        assert_eq!(parse(&serde_json::to_string(&c).unwrap()), c);
    }

    #[test]
    fn old_file_without_new_fields_loads_defaults() {
        let old = r#"{
            "style": "Notch",
            "battery_threshold": 15,
            "start_with_windows": false,
            "muted_apps": ["Slack"],
            "onboarding_done": true
        }"#;
        let c = parse(old);
        assert_eq!(c.style, Style::Notch);
        assert_eq!(c.battery_threshold, 15);
        assert!(!c.start_with_windows);
        assert_eq!(c.muted_apps, vec!["Slack".to_string()]);
        assert!(c.onboarding_done);
        assert_eq!(c.pomodoro, PomoSettings::default());
        assert_eq!(c.google_client_id, "");
        assert_eq!(c.google_client_secret, "");
        assert_eq!(c.language, Lang::Id);
    }

    #[test]
    fn language_is_a_lowercase_code() {
        let en = Config {
            language: Lang::En,
            ..Config::default()
        };
        let json = serde_json::to_string(&en).unwrap();
        assert!(json.contains(r#""language":"en""#), "{json}");
        assert_eq!(parse(r#"{"language":"en"}"#).language, Lang::En);
        assert_eq!(parse(r#"{"language":"id"}"#).language, Lang::Id);
        // A typo keeps the rest of the file: only the language falls back.
        let typo = parse(r#"{"style":"Notch","language":"inggris"}"#);
        assert_eq!((typo.style, typo.language), (Style::Notch, Lang::Id));
    }

    fn owned(c: &Config) -> (Style, bool, bool, PomoSettings, Lang) {
        (
            c.style,
            c.start_with_windows,
            c.onboarding_done,
            c.pomodoro,
            c.language,
        )
    }

    /// The island started with an empty client ID; the owner then added one
    /// (and changed other fields) by hand while the island changed its own.
    fn disk_and_mem() -> (Config, Config) {
        let disk = Config {
            google_client_id: "id.apps.googleusercontent.com".into(),
            google_client_secret: "secret".into(),
            muted_apps: vec!["Slack".into()],
            battery_threshold: 35,
            // Stale copies of the island's own settings.
            style: Style::Pill,
            pomodoro: PomoSettings::default(),
            language: Lang::Id,
            ..Config::default()
        };
        let mem = Config {
            style: Style::Notch,
            start_with_windows: false,
            onboarding_done: true,
            pomodoro: PomoSettings {
                focus_min: 20,
                ..PomoSettings::default()
            },
            language: Lang::En,
            ..Config::default()
        };
        (disk, mem)
    }

    #[test]
    fn merge_keeps_hand_edits_and_island_settings() {
        let (disk, mem) = disk_and_mem();
        let merged = merge_owned(disk.clone(), &mem);
        assert_eq!(merged.google_client_id, disk.google_client_id);
        assert_eq!(merged.google_client_secret, disk.google_client_secret);
        assert_eq!(merged.muted_apps, disk.muted_apps);
        assert_eq!(merged.battery_threshold, 35);
        assert_eq!(owned(&merged), owned(&mem));
    }

    /// A fresh directory under the system temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "dynamic-island-config-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn on_disk(path: &Path) -> Config {
        try_parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn classify_tells_missing_invalid_and_valid_apart() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(classify(Err(missing)), OnDisk::Missing);
        for empty in ["", "  \r\n", "\u{feff}", "\u{feff} \n"] {
            assert_eq!(classify(Ok(empty.into())), OnDisk::Missing, "{empty:?}");
        }
        assert_eq!(
            classify(Ok("{\"google_client_id\": ".into())),
            OnDisk::Invalid
        );
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(classify(Err(denied)), OnDisk::Invalid);
        assert_eq!(
            classify(Ok("\u{feff}{\"style\":\"Notch\"}".into())),
            OnDisk::Valid(Config {
                style: Style::Notch,
                ..Config::default()
            })
        );
    }

    #[test]
    fn save_merges_into_a_valid_file() {
        let tmp = TempDir::new("valid");
        let path = tmp.file("config.json");
        let (disk, mem) = disk_and_mem();
        disk.save_to(&path).unwrap();
        let out = save_merged_at(&path, &mem);
        assert_eq!(out.repair, Repair::None);
        assert_eq!(out.config, merge_owned(disk, &mem));
        assert_eq!(on_disk(&path), out.config);
        assert!(
            !tmp.file("config.json.tmp").exists(),
            "atomic write cleaned up"
        );
    }

    #[test]
    fn save_writes_a_missing_or_empty_file() {
        let tmp = TempDir::new("empty");
        let path = tmp.file("config.json");
        let (_, mem) = disk_and_mem();
        let out = save_merged_at(&path, &mem);
        assert_eq!(
            (out.config.clone(), out.repair),
            (mem.clone(), Repair::None)
        );
        assert_eq!(on_disk(&path), mem);
        // A truncated (empty) file is not "invalid": nothing in it to keep.
        for empty in ["", "\u{feff}\r\n"] {
            std::fs::write(&path, empty).unwrap();
            let out = save_merged_at(&path, &mem);
            assert_eq!(out.repair, Repair::None);
            assert_eq!(on_disk(&path), mem);
        }
        assert!(!tmp.file("config.json.bak").exists());
    }

    #[test]
    fn save_backs_up_an_invalid_file_then_writes() {
        let tmp = TempDir::new("invalid");
        let path = tmp.file("config.json");
        let (_, mem) = disk_and_mem();
        let typo = "{\"google_client_id\": \"abc\",, }";
        std::fs::write(&path, typo).unwrap();
        let out = save_merged_at(&path, &mem);
        let bak = tmp.file("config.json.bak");
        assert_eq!(out.repair, Repair::BackedUp(bak.clone()));
        assert_eq!(out.config, mem);
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), typo);
        assert_eq!(on_disk(&path), mem);
        // A second broken file never overwrites the first backup.
        std::fs::write(&path, "garbage").unwrap();
        let out = save_merged_at(&path, &mem);
        let bak2 = tmp.file("config.json.bak2");
        assert_eq!(out.repair, Repair::BackedUp(bak2.clone()));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), typo);
        assert_eq!(std::fs::read_to_string(&bak2).unwrap(), "garbage");
        assert_eq!(on_disk(&path), mem);
    }

    #[test]
    fn save_writes_nothing_when_the_backup_fails() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new("blocked");
        let path = tmp.file("config.json");
        let (_, mem) = disk_and_mem();
        std::fs::write(&path, "{broken").unwrap();
        // Held open without delete sharing (an editor locking it): it can be
        // read, but not renamed.
        const FILE_SHARE_READ: u32 = 1;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let out = save_merged_at(&path, &mem);
        assert_eq!(out.repair, Repair::Blocked);
        assert_eq!(out.config, mem, "the setting still holds for this run");
        drop(lock);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        assert!(!tmp.file("config.json.bak").exists());
        // Once the file is free, the next save repairs it.
        let out = save_merged_at(&path, &mem);
        assert!(matches!(out.repair, Repair::BackedUp(_)));
        assert_eq!(on_disk(&path), mem);
    }

    #[test]
    fn pomodoro_round_trip() {
        let c = Config {
            pomodoro: PomoSettings {
                focus_min: 45,
                short_break_min: 7,
                long_break_min: 20,
                sessions: 6,
            },
            ..Config::default()
        };
        let back = parse(&serde_json::to_string_pretty(&c).unwrap());
        assert_eq!(back.pomodoro, c.pomodoro);
        assert_eq!(back, c);
        // A partial pomodoro object keeps the missing fields at their defaults.
        let partial = parse(r#"{"pomodoro":{"focus_min":40}}"#);
        assert_eq!(
            partial.pomodoro,
            PomoSettings {
                focus_min: 40,
                ..PomoSettings::default()
            }
        );
    }
}
