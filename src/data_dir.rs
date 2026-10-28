//! Where Vale keeps its files: `%APPDATA%\Vale`. Before the rename the
//! folder was `%APPDATA%\DynamicIsland`; `migrate` moves it once, at startup,
//! and never deletes the old folder when it cannot move it.

use std::io;
use std::path::{Path, PathBuf};

/// The folder's name under `%APPDATA%`.
pub const DIR: &str = "Vale";
/// The folder's name before the rename.
pub const OLD_DIR: &str = "DynamicIsland";
/// Left in the old folder when it had to be copied instead of moved.
pub const MOVED_NOTE: &str = "MOVED-TO-Vale.txt";

/// What `migrate` did.
#[derive(Debug, PartialEq)]
pub enum Migration {
    /// No old folder: nothing to move.
    Nothing,
    /// The new folder exists already; the old one is left as it is.
    AlreadyThere,
    /// The old folder was renamed to the new one.
    Moved,
    /// Renaming failed (a file in use); everything was copied and the old
    /// folder got a note saying where the data went.
    Copied,
    /// Nothing could be moved or copied; the old folder stays in use.
    Failed(String),
}

/// `%APPDATA%`, or the current directory when it is unset.
pub fn root() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The data folder in use: `Vale`, or the old folder while a failed
/// migration left the data there.
pub fn dir() -> PathBuf {
    pick(&root())
}

/// `root\Vale` when it exists or nothing else does; the old folder while it
/// is the only one (a migration that failed).
pub fn pick(root: &Path) -> PathBuf {
    let new = root.join(DIR);
    let old = root.join(OLD_DIR);
    if !new.exists() && old.is_dir() {
        old
    } else {
        new
    }
}

/// Moves `root\DynamicIsland` to `root\Vale` once. Call it before anything
/// reads the data folder, in the single running instance.
pub fn migrate(root: &Path) -> Migration {
    migrate_with(root, |from, to| std::fs::rename(from, to), copy_dir)
}

fn migrate_with(
    root: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
    copy: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Migration {
    let old = root.join(OLD_DIR);
    let new = root.join(DIR);
    if !old.is_dir() {
        return Migration::Nothing;
    }
    // An empty folder of that name holds nothing of Vale's: it goes (only
    // while empty) and the data moves in.
    if new.exists() && std::fs::remove_dir(&new).is_err() {
        return Migration::AlreadyThere;
    }
    if rename(&old, &new).is_ok() {
        return Migration::Moved;
    }
    // A file in the old folder is held open: copy beside it, then put the
    // copy in place in one rename, so a half copy is never the data folder.
    let staging = root.join(format!("{DIR}.migrating"));
    let _ = std::fs::remove_dir_all(&staging);
    let copied = copy(&old, &staging).and_then(|()| std::fs::rename(&staging, &new));
    match copied {
        Ok(()) => {
            let note = format!(
                "The data in this folder was copied to {}.\r\nVale uses that folder now; this one can be deleted.\r\n",
                new.display()
            );
            let _ = std::fs::write(old.join(MOVED_NOTE), note);
            Migration::Copied
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            Migration::Failed(error.to_string())
        }
    }
}

/// Copies `from` into a new folder `to`, recursively.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vale-data-dir-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn old_with_data(root: &Path) {
        let old = root.join(OLD_DIR);
        std::fs::create_dir_all(old.join("lyrics-cache")).unwrap();
        std::fs::write(old.join("config.json"), "{}").unwrap();
        std::fs::write(old.join("lyrics-cache").join("a.lrc"), "[00:01]x").unwrap();
    }

    fn no_rename(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::other("in use"))
    }

    #[test]
    fn nothing_to_move_without_the_old_folder() {
        let root = temp_root("nothing");
        assert_eq!(migrate(&root), Migration::Nothing);
        assert!(!root.join(DIR).exists());
        assert_eq!(pick(&root), root.join(DIR));
    }

    #[test]
    fn the_old_folder_is_renamed() {
        let root = temp_root("moved");
        old_with_data(&root);
        assert_eq!(migrate(&root), Migration::Moved);
        assert!(!root.join(OLD_DIR).exists());
        let new = root.join(DIR);
        assert_eq!(
            std::fs::read_to_string(new.join("config.json")).unwrap(),
            "{}"
        );
        assert!(new.join("lyrics-cache").join("a.lrc").is_file());
        assert_eq!(pick(&root), new);
    }

    #[test]
    fn an_existing_new_folder_is_kept_and_the_old_one_left_alone() {
        let root = temp_root("already");
        old_with_data(&root);
        std::fs::create_dir_all(root.join(DIR)).unwrap();
        std::fs::write(root.join(DIR).join("config.json"), "new").unwrap();
        assert_eq!(migrate(&root), Migration::AlreadyThere);
        assert_eq!(
            std::fs::read_to_string(root.join(DIR).join("config.json")).unwrap(),
            "new"
        );
        assert!(root.join(OLD_DIR).join("config.json").is_file());
        assert_eq!(pick(&root), root.join(DIR));
    }

    #[test]
    fn an_empty_new_folder_does_not_stop_the_move() {
        // "Vale" is a common name: an empty folder of that name (made by
        // anything) must not leave the user's data behind.
        let root = temp_root("empty-new");
        old_with_data(&root);
        std::fs::create_dir_all(root.join(DIR)).unwrap();
        assert_eq!(migrate(&root), Migration::Moved);
        assert!(root.join(DIR).join("config.json").is_file());
        assert!(!root.join(OLD_DIR).exists());
    }

    #[test]
    fn a_folder_in_use_is_copied_and_the_old_one_gets_a_note() {
        let root = temp_root("copied");
        old_with_data(&root);
        assert_eq!(migrate_with(&root, no_rename, copy_dir), Migration::Copied);
        let new = root.join(DIR);
        assert!(new.join("lyrics-cache").join("a.lrc").is_file());
        // Nothing is deleted; the note says where the data went.
        let old = root.join(OLD_DIR);
        assert!(old.join("config.json").is_file());
        assert!(old.join(MOVED_NOTE).is_file());
        assert_eq!(pick(&root), new);
    }

    #[test]
    fn a_failed_copy_leaves_no_half_folder_and_keeps_using_the_old_one() {
        let root = temp_root("failed");
        old_with_data(&root);
        let half = |from: &Path, to: &Path| -> io::Result<()> {
            copy_dir(from, to)?;
            Err(io::Error::other("disk full"))
        };
        assert!(matches!(
            migrate_with(&root, no_rename, half),
            Migration::Failed(_)
        ));
        assert!(!root.join(DIR).exists(), "no half-copied folder");
        assert!(root.join(OLD_DIR).join("config.json").is_file());
        assert_eq!(pick(&root), root.join(OLD_DIR));
    }
}
