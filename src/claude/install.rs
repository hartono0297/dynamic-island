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
const BACKUP_NAME: &str = "settings.json.bak-vale";
/// Dynamic Island's backup, from before the rename: the true original.
const OLD_BACKUP_NAME: &str = "settings.json.bak-dynamic-island";
const TEMP_NAME: &str = "settings.json.tmp-vale";
/// Any hook command ending in one of these exe names (after a folder or the
/// opening quote) was written by some copy of the island: Vale, or Dynamic
/// Island from before the rename.
const ISLAND_EXES: [&str; 2] = ["vale.exe\" --hook", "dynamic-island.exe\" --hook"];

/// Whether `command` runs some copy of the island's `--hook`.
fn is_island_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    ISLAND_EXES.iter().any(|exe| {
        lower
            .strip_suffix(exe)
            .is_some_and(|before| before.ends_with('\\') || before.ends_with('"'))
    })
}

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
    hook["command"]
        .as_str()
        .is_some_and(|existing| existing != command && is_island_command(existing))
}

fn group_has_command(group: &Value, command: &str) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hooks| hooks.iter().any(|hook| hook["command"] == command))
}

fn is_island_hook(hook: &Value) -> bool {
    hook["command"].as_str().is_some_and(is_island_command)
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

/// True when any island event has a hook from some copy of the island: a
/// partial install still has something to remove.
fn any_island_hooks(settings: &Value) -> bool {
    EVENTS.iter().any(|event| {
        settings["hooks"][*event].as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group["hooks"]
                    .as_array()
                    .is_some_and(|hooks| hooks.iter().any(is_island_hook))
            })
        })
    })
}

/// The island's hooks as found in a Claude Code settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookState {
    /// Every island event runs this exe's command. Hooks of a moved exe (or
    /// of the other build) do not count, so installing can repoint them.
    pub complete: bool,
    /// Some island event has a hook from any copy of the island
    /// (`any_island_hooks`): there is something to remove.
    pub any: bool,
}

impl HookState {
    const NONE: HookState = HookState {
        complete: false,
        any: false,
    };

    fn of(settings: &Value, command: &str) -> Self {
        HookState {
            complete: all_events_have_command(settings, command),
            any: any_island_hooks(settings),
        }
    }
}

/// `%USERPROFILE%\.claude\settings.json`.
fn user_settings_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
}

/// The island's hooks in the settings file at `path`, measured against the
/// hook `command` of the running exe; read only. `None` when the file exists
/// but cannot be read or is not JSON.
pub fn hook_state_at(path: &Path, command: &str) -> Option<HookState> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Some(HookState::NONE),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .map(|v| HookState::of(&v, command)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(HookState::NONE),
        Err(_) => None,
    }
}

/// `hook_state_at` for the current user's Claude Code settings and this exe.
pub fn hook_state() -> Option<HookState> {
    hook_state_at(&user_settings_path()?, &current_command().ok()?)
}

/// The hook command for the running exe: `"<exe>" --hook`.
fn current_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\" --hook", exe.display()))
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
        InstallOutcome::Installed => println!("Hook Vale terpasang."),
        InstallOutcome::AlreadyInstalled => println!("Sudah terpasang."),
        InstallOutcome::Error(e) => eprintln!("Gagal memasang hook: {e}"),
    }
}

/// Registers this executable as a Claude Code hook in
/// `%USERPROFILE%\.claude\settings.json`.
pub fn install_for_current_exe() -> InstallOutcome {
    let command = match current_command() {
        Ok(command) => command,
        Err(e) => return InstallOutcome::Error(e),
    };
    let Some(path) = user_settings_path() else {
        return InstallOutcome::Error(tr(Key::UserProfileUnset).into());
    };
    install_at(&path, &command)
}

/// Adds `command` as the island's hook to the settings file at `path`. The
/// file as it was before the island first changed it is kept beside it as
/// `settings.json.bak-vale`.
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
        eprintln!("{}", tr(Key::UserProfileUnset));
        return;
    };
    match uninstall_outcome_at(&path) {
        UninstallOutcome::Removed => println!("Hook Vale dihapus."),
        UninstallOutcome::NothingToRemove => println!("Tidak ada hook Vale."),
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
    // Dynamic Island's backup predates any island hooks: it becomes Vale's.
    if let Some(dir) = backup.parent() {
        let old = dir.join(OLD_BACKUP_NAME);
        if !backup.exists() && old.is_file() && fs::rename(&old, backup).is_ok() {
            return Ok(());
        }
    }
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
            .filter(|c| is_island_command(c))
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
    fn hooks_from_before_the_rename_are_repointed_to_vale_and_removed_with_it() {
        const VALE: &str = "\"C:\\Programs\\Vale\\vale.exe\" --hook";
        let mut v =
            serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":OLD}]}]}});
        assert!(add_island_hooks(&mut v, VALE));
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "repointed, not duplicated");
        assert_eq!(stop[0]["hooks"][0]["command"], VALE);
        assert!(remove_island_hooks(&mut v));
        assert!(v["hooks"].get("Stop").is_none());
        // Another program whose name merely ends in "vale" is not the island.
        let other = "\"C:\\x\\novale.exe\" --hook";
        let mut v =
            serde_json::json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":other}]}]}});
        assert!(!remove_island_hooks(&mut v));
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
            let dir = std::env::temp_dir().join(format!("vale-test-{}-{name}", std::process::id()));
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
    fn the_backup_from_before_the_rename_becomes_vales() {
        // Dynamic Island kept the untouched original under its own name;
        // the settings now hold its hooks, so they are no original.
        let dir = TempDir::new("old-backup");
        let old = dir.0.join(OLD_BACKUP_NAME);
        fs::write(&old, r#"{"pristine":true}"#).unwrap();
        fs::write(
            dir.settings(),
            format!(r#"{{"x":"{}"}}"#, "with island hooks"),
        )
        .unwrap();
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(
            fs::read_to_string(dir.backup()).unwrap(),
            r#"{"pristine":true}"#
        );
        assert!(!old.exists());
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

    const NO_HOOKS: Option<HookState> = Some(HookState {
        complete: false,
        any: false,
    });
    const ALL_HOOKS: Option<HookState> = Some(HookState {
        complete: true,
        any: true,
    });

    /// The hook state as the exe at `NEW` sees it.
    fn state(dir: &TempDir) -> Option<HookState> {
        hook_state_at(&dir.settings(), NEW)
    }

    #[test]
    fn status_check_reads_without_writing() {
        let dir = TempDir::new("status");
        assert_eq!(state(&dir), NO_HOOKS, "no file");
        fs::write(dir.settings(), "  ").unwrap();
        assert_eq!(state(&dir), NO_HOOKS, "empty");
        let foreign = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]}}"#;
        fs::write(dir.settings(), foreign).unwrap();
        assert_eq!(state(&dir), NO_HOOKS);
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(state(&dir), ALL_HOOKS);
        let before = fs::read_to_string(dir.settings()).unwrap();
        state(&dir);
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), before);
        fs::write(dir.settings(), "{not json").unwrap();
        assert_eq!(state(&dir), None);
    }

    #[test]
    fn hooks_of_a_moved_exe_need_installing_again() {
        let dir = TempDir::new("stale");
        // A full set written by the exe at its old path (moved, or the other build).
        assert_eq!(install_at(&dir.settings(), OLD), InstallOutcome::Installed);
        let stale = Some(HookState {
            complete: false,
            any: true,
        });
        assert_eq!(state(&dir), stale, "Pasang stays on, Hapus too");
        assert_eq!(hook_state_at(&dir.settings(), OLD), ALL_HOOKS, "old exe");
        // Installing from the new path repoints every hook instead of adding more.
        assert_eq!(install_at(&dir.settings(), NEW), InstallOutcome::Installed);
        assert_eq!(state(&dir), ALL_HOOKS);
        let v = read_json(&dir.settings());
        for event in EVENTS {
            assert_eq!(island_commands(&v, event), [NEW], "{event}");
        }
    }

    #[test]
    fn partial_install_has_hooks_but_is_not_complete() {
        let dir = TempDir::new("partial");
        // One island hook (an older install, or a hand edit), beside a foreign one.
        let partial = serde_json::json!({"hooks":{
            "Stop":[{"hooks":[{"type":"command","command":OLD}]}],
            "Notification":[{"hooks":[{"type":"command","command":"orca.cmd"}]}]
        }});
        fs::write(dir.settings(), partial.to_string()).unwrap();
        let before = fs::read_to_string(dir.settings()).unwrap();
        assert_eq!(
            state(&dir),
            Some(HookState {
                complete: false,
                any: true
            })
        );
        assert_eq!(fs::read_to_string(dir.settings()).unwrap(), before);
        // Odd shapes are read without panicking and hold no island hook.
        for odd in [
            serde_json::json!({"hooks":{"Stop":"x"}}),
            serde_json::json!({"hooks":[1]}),
            serde_json::json!({"hooks":{"Stop":[{"hooks":7}]}}),
            serde_json::json!([]),
        ] {
            assert_eq!(HookState::of(&odd, NEW), HookState::NONE, "{odd}");
        }
        assert_eq!(uninstall_at(&dir.settings()), Ok(true));
        assert_eq!(state(&dir), NO_HOOKS);
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
        assert_eq!(state(&dir), NO_HOOKS);
        fs::write(dir.settings(), "{not json").unwrap();
        assert!(matches!(
            uninstall_outcome_at(&dir.settings()),
            UninstallOutcome::Error(_)
        ));
    }
}
