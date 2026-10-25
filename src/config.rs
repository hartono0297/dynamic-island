use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// The island's own settings (`mem`: style, autostart, onboarding, pomodoro)
/// over everything else as it is on disk (Google client, muted apps, battery
/// threshold), which the owner may have edited by hand while the island ran.
pub fn merge_owned(disk: Config, mem: &Config) -> Config {
    Config {
        style: mem.style,
        start_with_windows: mem.start_with_windows,
        onboarding_done: mem.onboarding_done,
        pomodoro: mem.pomodoro,
        ..disk
    }
}

/// What to write when the island saves `mem`, given the file as read now:
/// a missing file gets `mem`; a valid one keeps its other fields
/// (`merge_owned`); an unreadable or half-edited one is left alone (`None`)
/// rather than overwritten.
pub fn merge_for_save(read: std::io::Result<String>, mem: &Config) -> Option<Config> {
    match read {
        Ok(text) => try_parse(&text).map(|disk| merge_owned(disk, mem)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(mem.clone()),
        Err(_) => None,
    }
}

/// The current file merged under the island's own settings; `None` when it
/// is missing or not a config, so the caller keeps what it has.
pub fn reload(mem: &Config) -> Option<Config> {
    let disk = try_parse(&std::fs::read_to_string(path()).ok()?)?;
    Some(merge_owned(disk, mem))
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
    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
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
    }

    fn owned(c: &Config) -> (Style, bool, bool, PomoSettings) {
        (c.style, c.start_with_windows, c.onboarding_done, c.pomodoro)
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

    #[test]
    fn save_never_clobbers_the_file() {
        let (disk, mem) = disk_and_mem();
        let text = serde_json::to_string_pretty(&disk).unwrap();
        let merged = merge_for_save(Ok(format!("\u{feff}{text}")), &mem).unwrap();
        assert_eq!(merged, merge_owned(disk, &mem));
        // No file yet: the island's config as it is.
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(merge_for_save(Err(missing), &mem), Some(mem.clone()));
        // Half-edited JSON or an unreadable file: leave it alone.
        assert_eq!(
            merge_for_save(Ok("{\"google_client_id\": ".into()), &mem),
            None
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(merge_for_save(Err(denied), &mem), None);
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
