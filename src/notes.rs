use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub text: String,
    /// Unix seconds of the last edit.
    pub updated: i64,
}

/// Plain-text notes kept in a local JSON file. Changes stay in memory until `save`.
pub struct NotesStore {
    path: PathBuf,
    /// Newest `updated` first.
    notes: Vec<Note>,
    counter: u64,
    /// The file existed but could not be read or parsed. It is moved aside
    /// (`notes.json.bak`) before the first save instead of being overwritten.
    keep_old: bool,
}

/// The notes in a file read as `read`, and whether the file must be kept
/// aside: it exists but is unreadable or not a notes list. A missing file is
/// simply empty.
fn load(read: io::Result<String>) -> (Vec<Note>, bool) {
    match read {
        Ok(text) => {
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            match serde_json::from_str(text) {
                Ok(notes) => (notes, false),
                Err(_) => (Vec::new(), true),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Vec::new(), false),
        Err(_) => (Vec::new(), true),
    }
}

/// `notes.json.bak`, or `notes.json.bak2`, `.bak3`… when earlier backups exist.
fn backup_path(path: &Path) -> PathBuf {
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

impl NotesStore {
    /// A missing, unreadable or malformed file yields an empty store. The file itself
    /// is left untouched until the next `save`, which first renames an unreadable or
    /// malformed one to `notes.json.bak`.
    pub fn open(path: PathBuf) -> Self {
        let (mut notes, keep_old) = load(std::fs::read_to_string(&path));
        notes.sort_by_key(|n| std::cmp::Reverse(n.updated));
        NotesStore {
            path,
            notes,
            counter: 0,
            keep_old,
        }
    }

    /// `%APPDATA%\DynamicIsland\notes.json`, next to the config file.
    pub fn default_path() -> PathBuf {
        crate::config::path().with_file_name("notes.json")
    }

    pub fn list(&self) -> &[Note] {
        &self.notes
    }

    pub fn get(&self, id: &str) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Creates or edits a note and returns its id. Text that is empty after trimming
    /// deletes the note instead (or creates nothing) and returns `None`. An unknown
    /// `id` is treated as a new note and gets a fresh id.
    pub fn upsert(&mut self, id: Option<&str>, text: &str, now: i64) -> Option<String> {
        let existing = id.and_then(|id| self.notes.iter().position(|n| n.id == id));
        if text.trim().is_empty() {
            if let Some(i) = existing {
                self.notes.remove(i);
            }
            return None;
        }
        let mut note = match existing {
            Some(i) => self.notes.remove(i),
            None => Note {
                id: self.next_id(now),
                text: String::new(),
                updated: now,
            },
        };
        note.text = text.to_string();
        note.updated = now;
        let id = note.id.clone();
        // Ahead of every note with the same or an older timestamp.
        let at = self.notes.partition_point(|n| n.updated > now);
        self.notes.insert(at, note);
        Some(id)
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// Writes pretty JSON to a sibling `.tmp` file, then renames it over the real
    /// file so a crash mid-write never leaves a truncated store. A file that could
    /// not be loaded is renamed to a backup first; if that fails, nothing is written.
    pub fn save(&mut self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.keep_old {
            match std::fs::rename(&self.path, backup_path(&self.path)) {
                Ok(()) => {}
                // Gone meanwhile: nothing left to keep.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            self.keep_old = false;
        }
        let json = serde_json::to_string_pretty(&self.notes).map_err(io::Error::other)?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// `n{now}-{counter}`, skipping ids already present: a store reopened within
    /// the same second restarts its counter at zero.
    fn next_id(&mut self, now: i64) -> String {
        loop {
            let id = format!("n{now}-{}", self.counter);
            self.counter += 1;
            if self.get(&id).is_none() {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh directory under the system temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "dynamic-island-notes-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("notes.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_is_empty() {
        let tmp = TempDir::new("missing");
        let store = NotesStore::open(tmp.file());
        assert!(store.list().is_empty());
        assert!(!tmp.file().exists());
    }

    #[test]
    fn upsert_new_then_edit_keeps_id() {
        let tmp = TempDir::new("edit");
        let mut store = NotesStore::open(tmp.file());
        let id = store.upsert(None, "buy milk", 100).unwrap();
        assert!(id.starts_with("n100-"), "id was {id}");
        let again = store.upsert(Some(&id), "buy oat milk", 200).unwrap();
        assert_eq!(again, id);
        assert_eq!(store.list().len(), 1);
        let note = store.get(&id).unwrap();
        assert_eq!(note.text, "buy oat milk");
        assert_eq!(note.updated, 200);
        // Many new notes in the same second still get distinct ids.
        let a = store.upsert(None, "a", 300).unwrap();
        let b = store.upsert(None, "b", 300).unwrap();
        let c = store.upsert(None, "c", 300).unwrap();
        assert!(a != b && b != c && a != c);
        assert_eq!(store.list().len(), 4);
    }

    #[test]
    fn list_is_newest_first() {
        let tmp = TempDir::new("order");
        let mut store = NotesStore::open(tmp.file());
        let old = store.upsert(None, "old", 10).unwrap();
        let new = store.upsert(None, "new", 30).unwrap();
        let mid = store.upsert(None, "mid", 20).unwrap();
        let ids: Vec<&str> = store.list().iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec![new.as_str(), mid.as_str(), old.as_str()]);
        // Editing moves a note to the top.
        store.upsert(Some(&old), "old, edited", 40);
        assert_eq!(store.list()[0].id, old);
    }

    #[test]
    fn empty_text_deletes_existing() {
        let tmp = TempDir::new("delete");
        let mut store = NotesStore::open(tmp.file());
        let id = store.upsert(None, "temporary", 1).unwrap();
        assert_eq!(store.upsert(Some(&id), "   \n\t ", 2), None);
        assert!(store.get(&id).is_none());
        assert!(store.list().is_empty());
        // Explicit delete reports whether anything was removed.
        let other = store.upsert(None, "keep", 3).unwrap();
        assert!(store.delete(&other));
        assert!(!store.delete(&other));
    }

    #[test]
    fn empty_text_creates_nothing() {
        let tmp = TempDir::new("nothing");
        let mut store = NotesStore::open(tmp.file());
        assert_eq!(store.upsert(None, "", 1), None);
        assert_eq!(store.upsert(None, "  \r\n ", 1), None);
        assert_eq!(store.upsert(Some("n0-unknown"), " ", 1), None);
        assert!(store.list().is_empty());
    }

    #[test]
    fn save_and_reopen_round_trip() {
        let tmp = TempDir::new("roundtrip");
        let mut store = NotesStore::open(tmp.file());
        let a = store.upsert(None, "first", 5).unwrap();
        let b = store
            .upsert(None, "second line\nwith \"quotes\"", 9)
            .unwrap();
        store.save().unwrap();
        assert!(!tmp.file().with_extension("json.tmp").exists());
        assert!(!tmp.file().with_extension("tmp").exists());

        let reopened = NotesStore::open(tmp.file());
        assert_eq!(reopened.list(), store.list());
        assert_eq!(reopened.get(&a).unwrap().text, "first");
        assert_eq!(reopened.get(&b).unwrap().updated, 9);
        // A reopened store never reuses an id from the same second.
        let mut reopened = reopened;
        let c = reopened.upsert(None, "third", 5).unwrap();
        assert!(c != a && c != b, "id {c} collided");
        reopened.delete(&c);
        // Saving again overwrites the existing file.
        reopened.delete(&a);
        reopened.save().unwrap();
        assert_eq!(NotesStore::open(tmp.file()).list().len(), 1);
    }

    #[test]
    fn corrupt_file_is_empty_not_panic() {
        let tmp = TempDir::new("corrupt");
        std::fs::write(tmp.file(), "{not json at all").unwrap();
        let mut store = NotesStore::open(tmp.file());
        assert!(store.list().is_empty());
        // The corrupt file is left alone until an explicit save.
        store.upsert(None, "fresh", 1);
        assert_eq!(
            std::fs::read_to_string(tmp.file()).unwrap(),
            "{not json at all"
        );
        store.save().unwrap();
        assert_eq!(NotesStore::open(tmp.file()).list().len(), 1);
        // ...which keeps the corrupt file as a backup instead of losing it.
        let bak = tmp.0.join("notes.json.bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{not json at all");
        // Only once: the next save just replaces the good file.
        store.upsert(None, "second", 2);
        store.save().unwrap();
        assert!(!tmp.0.join("notes.json.bak2").exists());
        assert_eq!(NotesStore::open(tmp.file()).list().len(), 2);
    }

    #[test]
    fn load_decides_what_to_keep() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(load(Err(missing)), (vec![], false));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(load(Err(denied)), (vec![], true), "unreadable is kept");
        assert_eq!(load(Ok("{not json".into())), (vec![], true));
        assert_eq!(load(Ok(String::new())), (vec![], true));
        assert_eq!(load(Ok("\u{feff}[]".into())), (vec![], false));
        let one = r#"[{"id":"n1-0","text":"a","updated":1}]"#;
        let (notes, keep) = load(Ok(one.into()));
        assert_eq!((notes.len(), keep), (1, false));
    }

    #[test]
    fn existing_backups_are_not_overwritten() {
        let tmp = TempDir::new("backups");
        std::fs::write(tmp.0.join("notes.json.bak"), "older").unwrap();
        std::fs::write(tmp.file(), "garbage").unwrap();
        let mut store = NotesStore::open(tmp.file());
        store.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("notes.json.bak")).unwrap(),
            "older"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("notes.json.bak2")).unwrap(),
            "garbage"
        );
    }

    #[test]
    fn good_file_is_never_backed_up() {
        let tmp = TempDir::new("nobackup");
        let mut store = NotesStore::open(tmp.file());
        store.upsert(None, "a", 1);
        store.save().unwrap();
        let mut store = NotesStore::open(tmp.file());
        store.upsert(None, "b", 2);
        store.save().unwrap();
        assert!(!tmp.0.join("notes.json.bak").exists());
        assert_eq!(NotesStore::open(tmp.file()).list().len(), 2);
    }
}
