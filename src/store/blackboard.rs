use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::{Result, TakError};
use crate::store::coordination::CoordinationLinks;
use crate::store::lock;

/// Lifecycle state of a blackboard note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum BlackboardStatus {
    Open,
    Closed,
}

impl std::fmt::Display for BlackboardStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// A shared note posted to the repository-local blackboard.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlackboardNote {
    pub id: u64,
    pub author: String,
    pub message: String,
    pub status: BlackboardStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "CoordinationLinks::is_empty")]
    pub links: CoordinationLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Counter {
    next_id: u64,
}

/// Manages the shared blackboard runtime under `.tak/runtime/blackboard/`.
pub struct BlackboardStore {
    root: PathBuf,
}

impl BlackboardStore {
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.join("runtime").join("blackboard"),
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.locks_dir())?;

        let notes_path = self.notes_path();
        if !notes_path.exists() {
            fs::write(notes_path, "[]")?;
        }

        let counter_path = self.counter_path();
        if !counter_path.exists() {
            fs::write(counter_path, r#"{"next_id":1}"#)?;
        }

        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn notes_path(&self) -> PathBuf {
        self.root.join("notes.json")
    }

    fn counter_path(&self) -> PathBuf {
        self.root.join("counter.json")
    }

    fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn lock_path(&self) -> PathBuf {
        self.locks_dir().join("blackboard.lock")
    }

    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(TakError::BlackboardInvalidName);
        }
        Ok(())
    }

    fn read_notes_locked(&self) -> Result<Vec<BlackboardNote>> {
        let path = self.notes_path();
        if !path.exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content)
            .map_err(|e| TakError::BlackboardCorruptFile(path.display().to_string(), e.to_string()))
    }

    fn write_notes_locked(&self, notes: &[BlackboardNote]) -> Result<()> {
        fs::write(self.notes_path(), serde_json::to_string_pretty(notes)?)?;
        Ok(())
    }

    fn next_id_locked(&self) -> Result<u64> {
        let path = self.counter_path();
        let content = fs::read_to_string(&path)?;
        let mut counter: Counter = serde_json::from_str(&content).map_err(|e| {
            TakError::BlackboardCorruptFile(path.display().to_string(), e.to_string())
        })?;

        let id = counter.next_id;
        counter.next_id += 1;
        fs::write(&path, serde_json::to_string(&counter)?)?;
        Ok(id)
    }

    pub fn post(
        &self,
        author: &str,
        message: &str,
        tags: Vec<String>,
        task_ids: Vec<u64>,
    ) -> Result<BlackboardNote> {
        self.post_with_links(
            author,
            message,
            tags,
            task_ids,
            CoordinationLinks::default(),
        )
    }

    pub fn post_with_links(
        &self,
        author: &str,
        message: &str,
        tags: Vec<String>,
        task_ids: Vec<u64>,
        mut links: CoordinationLinks,
    ) -> Result<BlackboardNote> {
        Self::validate_name(author)?;

        let message = message.trim();
        if message.is_empty() {
            return Err(TakError::BlackboardInvalidMessage);
        }
        links.normalize();

        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path())?;

        let mut notes = self.read_notes_locked()?;

        let now = Utc::now();
        let note = BlackboardNote {
            id: self.next_id_locked()?,
            author: author.to_string(),
            message: message.to_string(),
            status: BlackboardStatus::Open,
            tags: normalize_tags(tags),
            task_ids: normalize_task_ids(task_ids),
            created_at: now,
            updated_at: now,
            closed_by: None,
            closed_reason: None,
            closed_at: None,
            links,
        };

        notes.push(note.clone());
        self.write_notes_locked(&notes)?;

        lock::release_lock(lock)?;

        Ok(note)
    }

    pub fn list(
        &self,
        status: Option<BlackboardStatus>,
        tag: Option<&str>,
        task_id: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<BlackboardNote>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }

        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path())?;
        let mut notes = self.read_notes_locked()?;
        lock::release_lock(lock)?;

        if let Some(status) = status {
            notes.retain(|n| n.status == status);
        }
        if let Some(tag) = tag {
            notes.retain(|n| n.tags.iter().any(|t| t == tag));
        }
        if let Some(task_id) = task_id {
            notes.retain(|n| n.task_ids.contains(&task_id));
        }

        notes.sort_by(|a, b| b.id.cmp(&a.id));

        if let Some(limit) = limit {
            notes.truncate(limit);
        }

        Ok(notes)
    }

    pub fn get(&self, id: u64) -> Result<BlackboardNote> {
        if !self.root.exists() {
            return Err(TakError::BlackboardNoteNotFound(id));
        }

        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path())?;
        let notes = self.read_notes_locked()?;
        lock::release_lock(lock)?;

        notes
            .into_iter()
            .find(|n| n.id == id)
            .ok_or(TakError::BlackboardNoteNotFound(id))
    }

    pub fn close(&self, id: u64, closed_by: &str, reason: Option<&str>) -> Result<BlackboardNote> {
        Self::validate_name(closed_by)?;

        if !self.root.exists() {
            return Err(TakError::BlackboardNoteNotFound(id));
        }

        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path())?;
        let mut notes = self.read_notes_locked()?;

        let now = Utc::now();

        let note = notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(TakError::BlackboardNoteNotFound(id))?;

        note.status = BlackboardStatus::Closed;
        note.updated_at = now;
        note.closed_by = Some(closed_by.to_string());
        note.closed_reason = reason.map(|s| s.to_string());
        note.closed_at = Some(now);

        let updated = note.clone();
        self.write_notes_locked(&notes)?;

        lock::release_lock(lock)?;

        Ok(updated)
    }

    pub fn reopen(&self, id: u64, reopened_by: &str) -> Result<BlackboardNote> {
        Self::validate_name(reopened_by)?;

        if !self.root.exists() {
            return Err(TakError::BlackboardNoteNotFound(id));
        }

        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path())?;
        let mut notes = self.read_notes_locked()?;

        let now = Utc::now();

        let note = notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(TakError::BlackboardNoteNotFound(id))?;

        note.status = BlackboardStatus::Open;
        note.updated_at = now;
        note.closed_by = None;
        note.closed_reason = None;
        note.closed_at = None;

        let updated = note.clone();
        self.write_notes_locked(&notes)?;

        lock::release_lock(lock)?;

        Ok(updated)
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut tags: Vec<String> = tags
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

fn normalize_task_ids(mut task_ids: Vec<u64>) -> Vec<u64> {
    task_ids.sort();
    task_ids.dedup();
    task_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_board() -> (tempfile::TempDir, BlackboardStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        let store = BlackboardStore::open(&tak_root);
        store.ensure_dirs().unwrap();
        (dir, store)
    }

    #[test]
    fn post_and_get_note() {
        let (_dir, store) = setup_board();

        let note = store
            .post(
                "agent_1",
                "Need eyes on migration ordering",
                vec!["db".into(), "review".into()],
                vec![7],
            )
            .unwrap();

        assert_eq!(note.id, 1);
        assert_eq!(note.status, BlackboardStatus::Open);

        let fetched = store.get(note.id).unwrap();
        assert_eq!(fetched.message, "Need eyes on migration ordering");
        assert_eq!(fetched.tags, vec!["db", "review"]);
    }

    #[test]
    fn post_with_links_normalizes_and_persists() {
        let (_dir, store) = setup_board();

        let note = store
            .post_with_links(
                "agent_1",
                "Need linkage",
                vec![],
                vec![],
                CoordinationLinks {
                    mesh_message_ids: vec![" m2 ".into(), "m1".into(), "m1".into()],
                    blackboard_note_ids: vec![7, 2, 7],
                    history_event_ids: vec![" h2 ".into(), "h1".into()],
                },
            )
            .unwrap();

        assert_eq!(note.links.mesh_message_ids, vec!["m1", "m2"]);
        assert_eq!(note.links.blackboard_note_ids, vec![2, 7]);
        assert_eq!(note.links.history_event_ids, vec!["h1", "h2"]);

        let fetched = store.get(note.id).unwrap();
        assert_eq!(fetched.links, note.links);
    }

    #[test]
    fn list_filters_and_limits() {
        let (_dir, store) = setup_board();

        store
            .post("a", "first", vec!["infra".into()], vec![1])
            .unwrap();
        store
            .post("a", "second", vec!["api".into()], vec![2])
            .unwrap();
        store
            .post("a", "third", vec!["api".into()], vec![2])
            .unwrap();

        let api = store.list(None, Some("api"), None, None).unwrap();
        assert_eq!(api.len(), 2);
        assert_eq!(api[0].message, "third");
        assert_eq!(api[1].message, "second");

        let for_task_2 = store.list(None, None, Some(2), Some(1)).unwrap();
        assert_eq!(for_task_2.len(), 1);
        assert_eq!(for_task_2[0].message, "third");
    }

    #[test]
    fn close_and_reopen_note() {
        let (_dir, store) = setup_board();

        let note = store
            .post("agent", "please verify", vec![], vec![])
            .unwrap();
        let closed = store.close(note.id, "reviewer", Some("done")).unwrap();

        assert_eq!(closed.status, BlackboardStatus::Closed);
        assert_eq!(closed.closed_by.as_deref(), Some("reviewer"));
        assert_eq!(closed.closed_reason.as_deref(), Some("done"));
        assert!(closed.closed_at.is_some());

        let reopened = store.reopen(note.id, "agent").unwrap();
        assert_eq!(reopened.status, BlackboardStatus::Open);
        assert!(reopened.closed_by.is_none());
        assert!(reopened.closed_reason.is_none());
        assert!(reopened.closed_at.is_none());
    }

    #[test]
    fn close_missing_note_errors() {
        let (_dir, store) = setup_board();

        let err = store.close(99, "agent", None).unwrap_err();
        assert!(matches!(err, TakError::BlackboardNoteNotFound(99)));
    }

    #[test]
    fn invalid_names_and_messages_rejected() {
        let (_dir, store) = setup_board();

        assert!(matches!(
            store.post("bad name", "hello", vec![], vec![]).unwrap_err(),
            TakError::BlackboardInvalidName
        ));
        assert!(matches!(
            store.post("agent", "   ", vec![], vec![]).unwrap_err(),
            TakError::BlackboardInvalidMessage
        ));
    }

    #[test]
    fn tags_and_task_ids_are_normalized() {
        let (_dir, store) = setup_board();

        let note = store
            .post(
                "agent",
                "normalize",
                vec![" api ".into(), "api".into(), "".into(), "ops".into()],
                vec![3, 1, 3, 2],
            )
            .unwrap();

        assert_eq!(note.tags, vec!["api", "ops"]);
        assert_eq!(note.task_ids, vec![1, 2, 3]);
    }

    #[test]
    fn corrupt_notes_file_returns_error() {
        let (_dir, store) = setup_board();

        fs::write(store.notes_path(), "not valid json").unwrap();
        let err = store.list(None, None, None, None).unwrap_err();
        assert!(matches!(err, TakError::BlackboardCorruptFile(_, _)));
    }
}
