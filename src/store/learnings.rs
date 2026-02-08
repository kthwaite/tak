use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::model::{Learning, LearningCategory};
use crate::store::lock;

/// File-based storage for learnings in `.tak/learnings/*.json`.
pub struct LearningStore {
    root: PathBuf,
}

impl LearningStore {
    /// Open an existing learnings store rooted at `.tak/`.
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.to_path_buf(),
        }
    }

    fn learnings_dir(&self) -> PathBuf {
        self.root.join("learnings")
    }

    fn learning_path(&self, id: u64) -> PathBuf {
        self.learnings_dir().join(format!("{id}.json"))
    }

    fn counter_path(&self) -> PathBuf {
        self.learnings_dir().join("counter.json")
    }

    fn next_id(&self) -> Result<u64> {
        let lock_path = self.root.join("learning_counter.lock");
        let lock_file = lock::acquire_lock(&lock_path)?;

        let counter_path = self.counter_path();
        let data = if counter_path.exists() {
            fs::read_to_string(&counter_path)?
        } else {
            r#"{"next_id": 1}"#.to_string()
        };

        #[derive(serde::Deserialize, serde::Serialize)]
        struct Counter {
            next_id: u64,
        }

        let mut counter: Counter = serde_json::from_str(&data)?;
        let id = counter.next_id;
        counter.next_id += 1;
        fs::write(&counter_path, serde_json::to_string(&counter)?)?;

        lock::release_lock(lock_file)?;

        Ok(id)
    }

    pub fn create(
        &self,
        title: String,
        description: Option<String>,
        category: LearningCategory,
        tags: Vec<String>,
        task_ids: Vec<u64>,
    ) -> Result<Learning> {
        let dir = self.learnings_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        let id = self.next_id()?;
        let now = Utc::now();
        let mut learning = Learning {
            id,
            title,
            description,
            category,
            tags,
            task_ids,
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        normalize_learning(&mut learning);

        let json = serde_json::to_string_pretty(&learning)?;
        fs::write(self.learning_path(id), json)?;
        Ok(learning)
    }

    pub fn read(&self, id: u64) -> Result<Learning> {
        let path = self.learning_path(id);
        if !path.exists() {
            return Err(TakError::LearningNotFound(id));
        }
        let data = fs::read_to_string(path)?;
        let learning: Learning = serde_json::from_str(&data)?;
        Ok(learning)
    }

    pub fn write(&self, learning: &mut Learning) -> Result<()> {
        normalize_learning(learning);
        let json = serde_json::to_string_pretty(learning)?;
        fs::write(self.learning_path(learning.id), json)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        let path = self.learning_path(id);
        if !path.exists() {
            return Err(TakError::LearningNotFound(id));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn list_ids(&self) -> Result<Vec<u64>> {
        let dir = self.learnings_dir();
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "counter.json" {
                continue;
            }
            if let Some(stem) = name.strip_suffix(".json")
                && let Ok(id) = stem.parse::<u64>()
            {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub fn list_all(&self) -> Result<Vec<Learning>> {
        self.list_ids()?
            .into_iter()
            .map(|id| self.read(id))
            .collect()
    }

    /// Compute a fingerprint from learning file metadata (id, size, mtime).
    pub fn fingerprint(&self) -> Result<String> {
        let dir = self.learnings_dir();
        if !dir.exists() {
            return Ok(String::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "counter.json" {
                continue;
            }
            if let Some(stem) = name.strip_suffix(".json")
                && let Ok(id) = stem.parse::<u64>()
            {
                let meta = entry.metadata()?;
                let mtime = meta
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let size = meta.len();
                entries.push((id, size, mtime));
            }
        }
        entries.sort();
        let fp = entries
            .iter()
            .map(|(id, size, mtime)| format!("{id}:{size}:{mtime}"))
            .collect::<Vec<_>>()
            .join(",");
        Ok(fp)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Normalize a learning: sort and deduplicate tags and task_ids.
fn normalize_learning(learning: &mut Learning) {
    learning.tags.retain(|t| !t.trim().is_empty());
    learning.tags.sort();
    learning.tags.dedup();
    learning.task_ids.sort();
    learning.task_ids.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, LearningStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(tak_root.join("learnings")).unwrap();
        let store = LearningStore::open(&tak_root);
        (dir, store)
    }

    #[test]
    fn create_and_read_learning() {
        let (_dir, store) = setup();
        let learning = store
            .create(
                "Use FTS5 for search".into(),
                Some("Full-text search in SQLite".into()),
                LearningCategory::Tool,
                vec!["sqlite".into()],
                vec![1, 2],
            )
            .unwrap();
        assert_eq!(learning.id, 1);
        assert_eq!(learning.title, "Use FTS5 for search");
        assert_eq!(learning.category, LearningCategory::Tool);

        let read = store.read(1).unwrap();
        assert_eq!(read.title, "Use FTS5 for search");
        assert_eq!(read.tags, vec!["sqlite"]);
        assert_eq!(read.task_ids, vec![1, 2]);
    }

    #[test]
    fn sequential_ids() {
        let (_dir, store) = setup();
        let l1 = store
            .create("A".into(), None, LearningCategory::Insight, vec![], vec![])
            .unwrap();
        let l2 = store
            .create("B".into(), None, LearningCategory::Pitfall, vec![], vec![])
            .unwrap();
        let l3 = store
            .create("C".into(), None, LearningCategory::Pattern, vec![], vec![])
            .unwrap();
        assert_eq!(l1.id, 1);
        assert_eq!(l2.id, 2);
        assert_eq!(l3.id, 3);
    }

    #[test]
    fn list_all_learnings() {
        let (_dir, store) = setup();
        store
            .create("A".into(), None, LearningCategory::Insight, vec![], vec![])
            .unwrap();
        store
            .create("B".into(), None, LearningCategory::Pattern, vec![], vec![])
            .unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn delete_learning() {
        let (_dir, store) = setup();
        store
            .create(
                "Doomed".into(),
                None,
                LearningCategory::Insight,
                vec![],
                vec![],
            )
            .unwrap();
        store.delete(1).unwrap();
        assert!(store.read(1).is_err());
    }

    #[test]
    fn read_nonexistent_fails() {
        let (_dir, store) = setup();
        assert!(store.read(999).is_err());
    }

    #[test]
    fn create_deduplicates_tags_and_task_ids() {
        let (_dir, store) = setup();
        let learning = store
            .create(
                "Duped".into(),
                None,
                LearningCategory::Tool,
                vec!["x".into(), "y".into(), "x".into()],
                vec![3, 1, 2, 1],
            )
            .unwrap();
        assert_eq!(learning.tags, vec!["x", "y"]);
        assert_eq!(learning.task_ids, vec![1, 2, 3]);
    }

    #[test]
    fn write_updates_learning() {
        let (_dir, store) = setup();
        let mut learning = store
            .create(
                "Original".into(),
                None,
                LearningCategory::Insight,
                vec![],
                vec![],
            )
            .unwrap();
        learning.title = "Updated".into();
        learning.updated_at = Utc::now();
        store.write(&mut learning).unwrap();

        let read = store.read(1).unwrap();
        assert_eq!(read.title, "Updated");
    }

    #[test]
    fn write_normalizes_tags_and_task_ids() {
        let (_dir, store) = setup();
        let mut learning = store
            .create(
                "Norm test".into(),
                None,
                LearningCategory::Insight,
                vec!["a".into()],
                vec![1],
            )
            .unwrap();
        // Introduce duplicates and disorder
        learning.tags = vec!["z".into(), "a".into(), "z".into(), " ".into()];
        learning.task_ids = vec![3, 1, 2, 1];
        store.write(&mut learning).unwrap();

        let read = store.read(1).unwrap();
        assert_eq!(read.tags, vec!["a", "z"]);
        assert_eq!(read.task_ids, vec![1, 2, 3]);
    }
}
