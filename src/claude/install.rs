use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::{Value, json};

/// Claude Code hook events the island listens to.
const EVENTS: [&str; 4] = ["Stop", "Notification", "UserPromptSubmit", "PostToolUse"];
/// Seconds Claude Code waits for `--hook` before giving up on it.
const HOOK_TIMEOUT_SECS: u64 = 5;
const BACKUP_NAME: &str = "settings.json.bak-dynamic-island";
const TEMP_NAME: &str = "settings.json.tmp-dynamic-island";

/// Adds a `command` hook group to each island event that lacks one, leaving
/// every other setting and hook in place. True when anything was added.
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
        if groups.iter().any(|group| group_has_command(group, command)) {
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

fn group_has_command(group: &Value, command: &str) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hooks| hooks.iter().any(|hook| hook["command"] == command))
}

fn all_events_have_command(settings: &Value, command: &str) -> bool {
    EVENTS.iter().all(|event| {
        settings["hooks"][*event]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| group_has_command(group, command)))
    })
}

/// `--install-hooks` mode: registers this executable as a Claude Code hook in
/// `%USERPROFILE%\.claude\settings.json`, backing the file up first.
pub fn install_hooks() {
    match try_install_hooks() {
        Ok(true) => println!("Hook Dynamic Island terpasang."),
        Ok(false) => println!("Sudah terpasang."),
        Err(e) => eprintln!("Gagal memasang hook: {e}"),
    }
}

/// True when the hooks were added, false when they were already there.
fn try_install_hooks() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("\"{}\" --hook", exe.display());
    let dir = PathBuf::from(std::env::var_os("USERPROFILE").ok_or("USERPROFILE tidak diset")?)
        .join(".claude");
    let path = dir.join("settings.json");

    let original = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut settings = match original.as_deref() {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text)
            .map_err(|e| format!("{} bukan JSON yang valid: {e}", path.display()))?,
        _ => json!({}),
    };

    let changed = add_island_hooks(&mut settings, &command);
    if !all_events_have_command(&settings, &command) {
        return Err(format!("format {} tidak dikenali", path.display()));
    }
    if !changed {
        return Ok(false);
    }

    let mut text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    text.push('\n');
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(original) = original {
        fs::write(dir.join(BACKUP_NAME), original).map_err(|e| e.to_string())?;
    }
    // Write beside the target, then swap, so a failed write never truncates settings.json.
    let temp = dir.join(TEMP_NAME);
    fs::write(&temp, text).map_err(|e| e.to_string())?;
    fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
