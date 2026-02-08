use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::Result;

/// Manages sidecar files: per-task context notes and history logs.
///
/// On-disk layout under `.tak/`:
///   - `context/{id}.md`  — free-form context notes (git-committed)
///   - `history/{id}.log` — append-only event log (git-committed)
pub struct SidecarStore {
    root: PathBuf,
}

impl SidecarStore {
    /// Open an existing sidecar store rooted at `.tak/`.
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.to_path_buf(),
        }
    }

    fn context_dir(&self) -> PathBuf {
        self.root.join("context")
    }

    fn history_dir(&self) -> PathBuf {
        self.root.join("history")
    }

    /// Path to the context file for a given task ID.
    pub fn context_path(&self, id: u64) -> PathBuf {
        self.context_dir().join(format!("{id}.md"))
    }

    /// Path to the history file for a given task ID.
    pub fn history_path(&self, id: u64) -> PathBuf {
        self.history_dir().join(format!("{id}.log"))
    }

    /// Read the context notes for a task. Returns None if no context file exists.
    pub fn read_context(&self, id: u64) -> Result<Option<String>> {
        let path = self.context_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        Ok(Some(content))
    }

    /// Write (overwrite) the context notes for a task.
    pub fn write_context(&self, id: u64, text: &str) -> Result<()> {
        let dir = self.context_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        fs::write(self.context_path(id), text)?;
        Ok(())
    }

    /// Delete the context file for a task, if it exists.
    pub fn delete_context(&self, id: u64) -> Result<()> {
        let path = self.context_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Append a timestamped entry to the history log for a task.
    pub fn append_history(&self, id: u64, event: &str) -> Result<()> {
        let dir = self.history_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let line = format!("{timestamp}  {event}\n");
        let path = self.history_path(id);

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Read the full history log for a task. Returns None if no history file exists.
    pub fn read_history(&self, id: u64) -> Result<Option<String>> {
        let path = self.history_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        Ok(Some(content))
    }

    /// Delete the history file for a task, if it exists.
    pub fn delete_history(&self, id: u64) -> Result<()> {
        let path = self.history_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Delete all sidecar files (context + history) for a task.
    pub fn delete(&self, id: u64) -> Result<()> {
        self.delete_context(id)?;
        self.delete_history(id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, SidecarStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(tak_root.join("context")).unwrap();
        fs::create_dir_all(tak_root.join("history")).unwrap();
        let store = SidecarStore::open(&tak_root);
        (dir, store)
    }

    #[test]
    fn context_round_trip() {
        let (_dir, store) = setup();
        assert!(store.read_context(1).unwrap().is_none());
        store.write_context(1, "some notes").unwrap();
        assert_eq!(
            store.read_context(1).unwrap().as_deref(),
            Some("some notes")
        );
    }

    #[test]
    fn context_overwrite() {
        let (_dir, store) = setup();
        store.write_context(1, "first").unwrap();
        store.write_context(1, "second").unwrap();
        assert_eq!(store.read_context(1).unwrap().as_deref(), Some("second"));
    }

    #[test]
    fn history_append() {
        let (_dir, store) = setup();
        assert!(store.read_history(1).unwrap().is_none());
        store.append_history(1, "started").unwrap();
        store.append_history(1, "finished").unwrap();
        let history = store.read_history(1).unwrap().unwrap();
        let lines: Vec<&str> = history.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("started"));
        assert!(lines[1].contains("finished"));
    }

    #[test]
    fn delete_removes_both_files() {
        let (_dir, store) = setup();
        store.write_context(1, "ctx").unwrap();
        store.append_history(1, "event").unwrap();
        assert!(store.context_path(1).exists());
        assert!(store.history_path(1).exists());
        store.delete(1).unwrap();
        assert!(!store.context_path(1).exists());
        assert!(!store.history_path(1).exists());
    }

    #[test]
    fn delete_nonexistent_is_noop() {
        let (_dir, store) = setup();
        // Should not error
        store.delete(999).unwrap();
    }

    #[test]
    fn history_entry_has_timestamp() {
        let (_dir, store) = setup();
        store.append_history(42, "test event").unwrap();
        let history = store.read_history(42).unwrap().unwrap();
        // Format: "2026-02-08T12:00:00Z  test event"
        let line = history.lines().next().unwrap();
        assert!(line.contains('T'), "should contain ISO timestamp");
        assert!(line.contains("test event"));
    }
}
