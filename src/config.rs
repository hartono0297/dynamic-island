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
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str(text).unwrap_or_default()
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
