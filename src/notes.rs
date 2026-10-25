// Not referenced by the app yet: wired in Task 10.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

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
}

impl NotesStore {
    /// A missing, unreadable or malformed file yields an empty store. The file itself
    /// is left untouched until the next `save`.
    pub fn open(path: PathBuf) -> Self {
        let mut notes: Vec<Note> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| {
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                serde_json::from_str(text).ok()
            })
            .unwrap_or_default();
        notes.sort_by_key(|n| std::cmp::Reverse(n.updated));
        NotesStore {
            path,
            notes,
            counter: 0,
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
    /// file so a crash mid-write never leaves a truncated store.
    pub fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
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
    }
}
