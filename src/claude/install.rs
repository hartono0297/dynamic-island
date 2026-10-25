use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::i18n::{self, Key, tr};

/// Claude Code hook events the island listens to.
const EVENTS: [&str; 5] = [
    "Stop",
    "Notification",
    "UserPromptSubmit",
    "PostToolUse",
    "SessionEnd",
];
/// Seconds Claude Code waits for `--hook` before giving up on it.
const HOOK_TIMEOUT_SECS: u64 = 5;
const BACKUP_NAME: &str = "settings.json.bak-dynamic-island";
const TEMP_NAME: &str = "settings.json.tmp-dynamic-island";
/// Any hook command ending like this was written by some copy of the island.
const ISLAND_COMMAND_SUFFIX: &str = "dynamic-island.exe\" --hook";

#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    Installed,
    AlreadyInstalled,
    Error(String),
}

/// Makes `command` the island's hook for each island event, leaving every
/// other setting and hook in place. An island hook pointing at another exe
/// (the island was moved) is repointed rather than duplicated. True when
/// anything changed.
pub fn add_island_hooks(settings: &mut Value, command: &str) -> bool {
    let Some(root) = settings.as_object_mut() else {
        return false;
    };
    let Some(hooks) = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    else {
        return false;
    };
    let mut changed = false;
    for event in EVENTS {
        let Some(groups) = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
        else {
            continue;
        };
        let mut present = groups.iter().any(|group| group_has_command(group, command));
        changed |= repoint_stale_hooks(groups, command, &mut present);
        if present {
            continue;
        }
        let entry = json!([{"type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS}]);
        groups.push(if event == "PostToolUse" {
            json!({"matcher": "*", "hooks": entry})
        } else {
            json!({"hooks": entry})
        });
        changed = true;
    }
    changed
}

/// Points the first stale island hook at `command` (when none is `present`
/// yet) and drops the others; a group left empty by that is dropped too.
fn repoint_stale_hooks(groups: &mut Vec<Value>, command: &str, present: &mut bool) -> bool {
    let mut changed = false;
    groups.retain_mut(|group| {
        // `get_mut`, not `group["hooks"]`: IndexMut would insert `"hooks": null`
        // into foreign groups and panic on groups that are not objects.
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = hooks.len();
        hooks.retain_mut(|hook| {
            if !is_stale_island_hook(hook, command) {
                return true;
            }
            changed = true;
            if *present {
                return false;
            }
            hook["command"] = json!(command);
            *present = true;
            true
        });
        before == 0 || !hooks.is_empty()
    });
    changed
}

fn is_stale_island_hook(hook: &Value, command: &str) -> bool {
    hook["command"].as_str().is_some_and(|existing| {
        existing != command
            && existing
                .to_ascii_lowercase()
                .ends_with(ISLAND_COMMAND_SUFFIX)
    })
}

fn group_has_command(group: &Value, command: &str) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hooks| hooks.iter().any(|hook| hook["command"] == command))
}

fn is_island_hook(hook: &Value) -> bool {
    hook["command"]
        .as_str()
        .is_some_and(|c| c.to_ascii_lowercase().ends_with(ISLAND_COMMAND_SUFFIX))
}

/// Drops every hook written by any copy of the island, plus the groups and
/// event lists left empty by that. Foreign hooks stay untouched. True when
/// anything changed.
pub fn remove_island_hooks(settings: &mut Value) -> bool {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    hooks.retain(|_, groups| {
        let Some(list) = groups.as_array_mut() else {
            return true;
        };
        let before = list.len();
        list.retain_mut(|group| {
            let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let had = inner.len();
            inner.retain(|hook| !is_island_hook(hook));
            if inner.len() == had {
                return true;
            }
            changed = true;
            !inner.is_empty()
        });
        before == 0 || !list.is_empty()
    });
    changed
}

fn all_events_have_command(settings: &Value, command: &str) -> bool {
    EVENTS.iter().all(|event| {
        settings["hooks"][*event]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| group_has_command(group, command)))
    })
}

/// True when every island event has a hook from some copy of the island.
fn has_island_hooks(settings: &Value) -> bool {
    EVENTS.iter().all(|event| {
        settings["hooks"][*event].as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group["hooks"]
                    .as_array()
                    .is_some_and(|hooks| hooks.iter().any(is_island_hook))
            })
        })
    })
}

/// `%USERPROFILE%\.claude\settings.json`.
fn user_settings_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
}

/// Whether the island's hooks are in the settings file at `path`; read only.
/// `None` when the file exists but cannot be read or is not JSON.
pub fn hooks_present_at(path: &Path) -> Option<bool> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Some(false),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .map(|v| has_island_hooks(&v)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// `hooks_present_at` for the current user's Claude Code settings.
pub fn hooks_present() -> Option<bool> {
    hooks_present_at(&user_settings_path()?)
}

#[derive(Debug, PartialEq)]
pub enum UninstallOutcome {
    Removed,
    NothingToRemove,
    Error(String),
}

/// `uninstall_at` as an outcome for the settings window.
pub fn uninstall_outcome_at(path: &Path) -> UninstallOutcome {
    match uninstall_at(path) {
        Ok(true) => UninstallOutcome::Removed,
        Ok(false) => UninstallOutcome::NothingToRemove,
        Err(e) => UninstallOutcome::Error(e),
    }
}

/// Removes the island's hooks from the current user's Claude Code settings.
pub fn uninstall_for_current_user() -> UninstallOutcome {
    match user_settings_path() {
        Some(path) => uninstall_outcome_at(&path),
        None => UninstallOutcome::Error(tr(Key::UserProfileUnset).into()),
    }
}

/// `--install-hooks` mode: prints the outcome of `install_for_current_exe`.
pub fn install_hooks() {
    match install_for_current_exe() {
        InstallOutcome::Installed => println!("Hook Dynamic Island terpasang."),
        InstallOutcome::AlreadyInstalled => println!("Sudah terpasang."),
        InstallOutcome::Error(e) => eprintln!("Gagal memasang hook: {e}"),
    }
}

/// Registers this executable as a Claude Code hook in
/// `%USERPROFILE%\.claude\settings.json`.
pub fn install_for_current_exe() -> InstallOutcome {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return InstallOutcome::Error(e.to_string()),
    };
    let Some(path) = user_settings_path() else {
        return InstallOutcome::Error(tr(Key::UserProfileUnset).into());
    };
    install_at(&path, &format!("\"{}\" --hook", exe.display()))
}

/// Adds `command` as the island's hook to the settings file at `path`. The
/// file as it was before the island first changed it is kept beside it as
/// `settings.json.bak-dynamic-island`.
pub fn install_at(path: &Path, command: &str) -> InstallOutcome {
    match try_install_at(path, command) {
        Ok(true) => InstallOutcome::Installed,
        Ok(false) => InstallOutcome::AlreadyInstalled,
        Err(e) => InstallOutcome::Error(e),
    }
}

/// True when the hooks were added, false when they were already there.
fn try_install_at(path: &Path, command: &str) -> Result<bool, String> {
    let original = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut settings = match original.as_deref() {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str(text).map_err(|e| not_valid_json(path, &e))?
        }
        _ => json!({}),
    };

    let changed = add_island_hooks(&mut settings, command);
    if !all_events_have_command(&settings, command) {
        return Err(i18n::unknown_format(
            i18n::lang(),
            &path.display().to_string(),
        ));
    }
    if !changed {
        return Ok(false);
    }

    let mut text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    text.push('\n');
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    if let Some(original) = original {
        write_backup_once(&dir.join(BACKUP_NAME), &original)?;
    }
    // Write beside the target, then swap, so a failed write never truncates settings.json.
    let temp = dir.join(TEMP_NAME);
    fs::write(&temp, text).map_err(|e| e.to_string())?;
    fs::rename(&temp, path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// `--uninstall-hooks` mode: removes the island's hooks from
/// `%USERPROFILE%\.claude\settings.json`, used by the uninstall script.
pub fn uninstall_hooks() {
    let Some(path) = user_settings_path() else {
        eprintln!("USERPROFILE tidak diset");
        return;
    };
    match uninstall_outcome_at(&path) {
        UninstallOutcome::Removed => println!("Hook Dynamic Island dihapus."),
        UninstallOutcome::NothingToRemove => println!("Tidak ada hook Dynamic Island."),
        UninstallOutcome::Error(e) => eprintln!("Gagal menghapus hook: {e}"),
    }
}

fn not_valid_json(path: &Path, e: &serde_json::Error) -> String {
    i18n::not_valid_json(i18n::lang(), &path.display().to_string(), &e.to_string())
}

/// True when island hooks were found and removed from the file at `path`.
pub fn uninstall_at(path: &Path) -> Result<bool, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(false);
    }
    let mut settings: Value = serde_json::from_str(&text).map_err(|e| not_valid_json(path, &e))?;
    if !remove_island_hooks(&mut settings) {
        return Ok(false);
    }
    let mut out = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    out.push('\n');
    let dir = path.parent().unwrap_or(Path::new("."));
    write_backup_once(&dir.join(BACKUP_NAME), &text)?;
    let temp = dir.join(TEMP_NAME);
    fs::write(&temp, out).map_err(|e| e.to_string())?;
    fs::rename(&temp, path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Keeps the oldest backup: the settings as they were before the island
/// first touched them.
fn write_backup_once(backup: &Path, original: &str) -> Result<(), String> {
    match OpenOptions::new().write(true).create_new(true).open(backup) {
        Ok(mut file) => file
            .write_all(original.as_bytes())
            .map_err(|e| format!("{}: {e}", backup.display())),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("{}: {e}", backup.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "\"C:\\old\\dynamic-island.exe\" --hook";
    const NEW: &str = "\"D:\\new\\dynamic-island.exe\" --hook";

    fn island_commands(v: &Value, event: &str) -> Vec<String> {
        v["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .filter(|c| c.ends_with(ISLAND_COMMAND_SUFFIX))
            .map(String::from)
            .collect()
    }

    #[test]
    fn add_hooks_preserves_existing_and_is_idempotent() {
        let mut v = serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}});
        assert!(add_island_hooks(&mut v, "\"C:\\di.exe\" --hook"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        for e in EVENTS {
            assert!(v["hooks"][e].is_array());
        }
        assert!(!add_island_hooks(&mut v, "\"C:\\di.exe\" --hook"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "orca.cmd");
    }

    #[test]
    fn installs_five_events_including_session_end() {
        assert_eq!(
            EVENTS,
            [
                "Stop",
                "Notification",
                "UserPromptSubmit",
                "PostToolUse",
                "SessionEnd"
            ]
        );
        let mut v = serde_json::json!({});
        assert!(add_island_hooks(&mut v, "di --hook"));
        assert_eq!(v["hooks"].as_object().unwrap().len(), 5);
        assert!(v["hooks"]["SessionEnd"][0].get("matcher").is_none());
    }

    #[test]
    fn four_event_install_gains_session_end() {
        let mut v = serde_json::json!({});
        add_island_hooks(&mut v, NEW);
        v["hooks"].as_object_mut().unwrap().remove("SessionEnd");
        assert!(add_island_hooks(&mut v, NEW));
        assert_eq!(island_commands(&v, "SessionEnd"), vec![NEW.to_string()]);
        assert!(!add_island_hooks(&mut v, NEW));
    }

    #[test]
    fn add_hooks_to_empty_object() {
        let mut v = serde_json::json!({});
        assert!(add_island_hooks(&mut v, "di --hook"));
        for e in EVENTS {
            let groups = v["hooks"][e].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{e}");
            assert_eq!(
                groups[0]["hooks"][0],
                serde_json::json!({"type":"command","command":"di --hook","timeout":5})
            );
        }
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert!(v["hooks"]["Stop"][0].get("matcher").is_none());
    }

    #[test]
    fn keeps_other_settings_untouched() {
        let mut v = serde_json::json!({"model":"opus","env":{"A":"1"}});
        assert!(add_island_hooks(&mut v, "di --hook"));
        assert_eq!(v["model"], "opus");
        assert_eq!(v["env"]["A"], "1");
    }

    #[test]
    fn moved_exe_repoints_the_old_island_hook() {
        let mut v = serde_json::json!({});
        add_island_hooks(&mut v, OLD);
        assert!(add_island_hooks(&mut v, NEW));
        for e in EVENTS {
            assert_eq!(island_commands(&v, e), vec![NEW.to_string()], "{e}");
            assert_eq!(v["hooks"][e].as_array().unwrap().len(), 1, "{e}");
        }
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 5);
        assert!(!add_island_hooks(&mut v, NEW));
    }

    #[test]
    fn stale_island_hook_is_dropped_when_current_one_exists() {
        let mut v = serde_json::json!({"hooks":{"Stop":[
            {"hooks":[{"type":"command","command":OLD}]},
            {"hooks":[{"type":"command","command":"orca.cmd"},{"type":"command","command":OLD}]},
            {"hooks":[{"type":"command","command":NEW}]}
        ]}});
        assert!(add_island_hooks(&mut v, NEW));
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(island_commands(&v, "Stop"), vec![NEW.to_string()]);
        // Every stale copy goes, and so does a group left empty by that;
        // groups with other hooks stay.
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "orca.cmd");
        assert_eq!(stop[1]["hooks"][0]["command"], NEW);
    }

    #[test]
    fn odd_foreign_groups_survive_byte_identical() {
        let foreign = [
            serde_json::json!({"matcher":"*"}),
            serde_json::json!("x"),
            Value::Null,
        ];
        let mut stop: Vec<Value> = foreign.to_vec();
        stop.push(serde_json::json!({"hooks":[{"type":"command","command":OLD}]}));
        let mut v = serde_json::json!({"hooks":{"Stop":stop}});
        assert!(add_island_hooks(&mut v, NEW));
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 4);
        for (got, want) in stop.iter().zip(&foreign) {
            assert_eq!(got.to_string(), want.to_string());
        }
        assert_eq!(stop[3]["hooks"][0]["command"], NEW);
    }

    #[test]
    fn foreign_hooks_that_merely_mention_the_island_stay() {
        let other = "notepad.exe dynamic-island.exe --hook-log";
        let mut v =
            serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":other}]}]}});
        assert!(add_island_hooks(&mut v, NEW));
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], other);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn island_suffix_match_ignores_case() {
        let upper = "\"C:\\X\\Dynamic-Island.EXE\" --hook";
        let mut v =
            serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":upper}]}]}});
        assert!(add_island_hooks(&mut v, NEW));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], NEW);
    }

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("dynamic-island-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn settings(&self) -> PathBuf {
            self.0.join("settings.json")
        }

        fn backup(&self) -> PathBuf {
            self.0.join(BACKUP_NAME)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn creates_missing_settings_without_backup() {
        let dir = TempDir::new("missing");
        let nested = dir.0.join("nested");
        let path = nested.join("settings.json");
        assert_eq!(install_at(&path, NEW), InstallOutcome::Installed);
        let v = read_json(&path);
        for e in EVENTS {
            assert_eq!(island_commands(&v, e), vec![NEW.to_string()]);
        }
        assert!(!nested.join(BACKUP_NAME).exists());
        assert!(!nested.join(TEMP_NAME).exists());
    }

    #[test]
    fn installs_into_existing_file_and_backs_it_up() {
        let dir = TempDir::new("existing");
        let original = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}}"#;
        fs::write(dir.settings(), original).unwrap();
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        let v = read_json(&dir.settings());
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "orca.cmd");
        assert_eq!(island_commands(&v, "Stop"), vec![NEW.to_string()]);
        assert_eq!(fs::read_to_string(dir.backup()).unwrap(), original);
    }

    #[test]
    fn second_install_is_already_installed_and_leaves_file_alone() {
        let dir = TempDir::new("again");
        fs::write(dir.settings(), "{}").unwrap();
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        let written = fs::read_to_string(dir.settings()).unwrap();
        assert_eq!(
            install_at(&dir.settings(), NEW),
            InstallOutcome::AlreadyInstalled
        );
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), written);
    }

    #[test]
    fn existing_backup_is_never_overwritten() {
        let dir = TempDir::new("backup");
        fs::write(dir.settings(), r#"{"first":true}"#).unwrap();
        assert_eq!(install_at(&dir.settings(), OLD), InstallOutcome::Installed);
        // The exe moved: the next install rewrites settings but keeps the first backup.
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(
            fs::read_to_string(dir.backup()).unwrap(),
            r#"{"first":true}"#
        );
        let v = read_json(&dir.settings());
        for e in EVENTS {
            assert_eq!(island_commands(&v, e), vec![NEW.to_string()]);
        }
    }

    #[test]
    fn invalid_json_is_an_error_and_untouched() {
        let dir = TempDir::new("invalid");
        fs::write(dir.settings(), "{not json").unwrap();
        assert!(matches!(
            install_at(&dir.settings(), NEW),
            InstallOutcome::Error(_)
        ));
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), "{not json");
        assert!(!dir.backup().exists());
    }

    #[test]
    fn unrecognised_hooks_shape_is_an_error() {
        let dir = TempDir::new("shape");
        fs::write(dir.settings(), r#"{"hooks":[]}"#).unwrap();
        assert!(matches!(
            install_at(&dir.settings(), NEW),
            InstallOutcome::Error(_)
        ));
        assert_eq!(
            fs::read_to_string(dir.settings()).unwrap(),
            r#"{"hooks":[]}"#
        );
    }

    #[test]
    fn remove_drops_island_hooks_and_keeps_foreign_ones() {
        let mut v = json!({"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}});
        add_island_hooks(&mut v, NEW);
        assert!(remove_island_hooks(&mut v));
        assert_eq!(
            v,
            json!({"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}})
        );
        assert!(!remove_island_hooks(&mut v));
    }

    #[test]
    fn remove_handles_odd_shapes_without_panicking() {
        let mut v = json!({"hooks":{"Stop":[{"matcher":"*"}, "x", null], "Other": "y"}});
        let before = v.clone();
        assert!(!remove_island_hooks(&mut v));
        assert_eq!(v, before);
        assert!(!remove_island_hooks(&mut json!([1, 2])));
    }

    #[test]
    fn uninstall_at_rewrites_file_and_reports_missing() {
        let dir = TempDir::new("uninstall");
        assert_eq!(uninstall_at(&dir.settings()), Ok(false));
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(uninstall_at(&dir.settings()), Ok(true));
        let v: Value = serde_json::from_str(&fs::read_to_string(dir.settings()).unwrap()).unwrap();
        assert_eq!(v, json!({"hooks":{}}));
        assert_eq!(uninstall_at(&dir.settings()), Ok(false));
        fs::write(dir.settings(), "{not json").unwrap();
        assert!(uninstall_at(&dir.settings()).is_err());
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), "{not json");
    }

    #[test]
    fn status_check_reads_without_writing() {
        let dir = TempDir::new("status");
        assert_eq!(hooks_present_at(&dir.settings()), Some(false), "no file");
        fs::write(dir.settings(), "  ").unwrap();
        assert_eq!(hooks_present_at(&dir.settings()), Some(false), "empty");
        let foreign = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}}"#;
        fs::write(dir.settings(), foreign).unwrap();
        assert_eq!(hooks_present_at(&dir.settings()), Some(false));
        assert_eq!(install_at(&dir.settings(), OLD), InstallOutcome::Installed);
        // Any copy of the island counts, wherever its exe is.
        assert_eq!(hooks_present_at(&dir.settings()), Some(true));
        let before = fs::read_to_string(dir.settings()).unwrap();
        hooks_present_at(&dir.settings());
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), before);
        fs::write(dir.settings(), "{not json").unwrap();
        assert_eq!(hooks_present_at(&dir.settings()), None);
    }

    #[test]
    fn uninstall_outcomes() {
        let dir = TempDir::new("uninstall-outcome");
        assert_eq!(
            uninstall_outcome_at(&dir.settings()),
            UninstallOutcome::NothingToRemove
        );
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(
            uninstall_outcome_at(&dir.settings()),
            UninstallOutcome::Removed
        );
        assert_eq!(hooks_present_at(&dir.settings()), Some(false));
        fs::write(dir.settings(), "{not json").unwrap();
        assert!(matches!(
            uninstall_outcome_at(&dir.settings()),
            UninstallOutcome::Error(_)
        ));
    }
}
