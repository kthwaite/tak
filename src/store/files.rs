use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::model::{Kind, Status, Task};
use crate::store::lock;

/// Root of the .tak directory for a repository.
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    /// Open an existing .tak directory.
    pub fn open(repo_root: &Path) -> Result<Self> {
        let root = repo_root.join(".tak");
        if !root.join("config.json").exists() {
            return Err(TakError::NotInitialized);
        }
        Ok(Self { root })
    }

    /// Initialize a new .tak directory.
    pub fn init(repo_root: &Path) -> Result<Self> {
        let root = repo_root.join(".tak");
        if root.join("config.json").exists() {
            return Err(TakError::AlreadyInitialized);
        }

        fs::create_dir_all(root.join("tasks"))?;
        fs::write(root.join("counter.json"), r#"{"next_id": 1}"#)?;
        fs::write(root.join("config.json"), r#"{"version": 1}"#)?;

        Ok(Self { root })
    }

    fn tasks_dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    fn task_path(&self, id: u64) -> PathBuf {
        self.tasks_dir().join(format!("{}.json", id))
    }

    fn counter_path(&self) -> PathBuf {
        self.root.join("counter.json")
    }

    fn next_id(&self) -> Result<u64> {
        let lock_path = self.root.join("counter.lock");
        let lock_file = lock::acquire_lock(&lock_path)?;

        let data = fs::read_to_string(self.counter_path())?;

        #[derive(serde::Deserialize, serde::Serialize)]
        struct Counter {
            next_id: u64,
        }

        let mut counter: Counter = serde_json::from_str(&data)?;
        let id = counter.next_id;
        counter.next_id += 1;
        fs::write(self.counter_path(), serde_json::to_string(&counter)?)?;

        lock::release_lock(lock_file)?;

        Ok(id)
    }

    pub fn create(
        &self,
        title: String,
        kind: Kind,
        description: Option<String>,
        parent: Option<u64>,
        depends_on: Vec<u64>,
        tags: Vec<String>,
    ) -> Result<Task> {
        if let Some(pid) = parent {
            self.read(pid)?;
        }
        for &dep in &depends_on {
            self.read(dep)?;
        }

        let id = self.next_id()?;
        let now = Utc::now();
        let mut task = Task {
            id,
            title,
            description,
            status: Status::Pending,
            kind,
            parent,
            depends_on,
            assignee: None,
            tags,
            created_at: now,
            updated_at: now,
        };
        task.normalize();

        let json = serde_json::to_string_pretty(&task)?;
        fs::write(self.task_path(id), json)?;
        Ok(task)
    }

    pub fn read(&self, id: u64) -> Result<Task> {
        let path = self.task_path(id);
        if !path.exists() {
            return Err(TakError::TaskNotFound(id));
        }
        let data = fs::read_to_string(path)?;
        let task: Task = serde_json::from_str(&data)?;
        Ok(task)
    }

    pub fn write(&self, task: &Task) -> Result<()> {
        let json = serde_json::to_string_pretty(task)?;
        fs::write(self.task_path(task.id), json)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        let path = self.task_path(id);
        if !path.exists() {
            return Err(TakError::TaskNotFound(id));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn list_ids(&self) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(self.tasks_dir())? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".json")
                && let Ok(id) = stem.parse::<u64>()
            {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Compute a fingerprint from task file metadata (id, size, mtime).
    /// Cheap (stat calls, no file reads) and detects additions, deletions,
    /// and in-place edits. Uses nanosecond mtime to catch rapid same-size edits.
    pub fn fingerprint(&self) -> Result<String> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(self.tasks_dir())? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
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

    pub fn read_many(&self, ids: &[u64]) -> Result<Vec<Task>> {
        ids.iter().map(|&id| self.read(id)).collect()
    }

    pub fn list_all(&self) -> Result<Vec<Task>> {
        self.list_ids()?
            .into_iter()
            .map(|id| self.read(id))
            .collect()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_creates_directory_structure() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        assert!(store.root().join("config.json").exists());
        assert!(store.root().join("counter.json").exists());
        assert!(store.root().join("tasks").is_dir());
    }

    #[test]
    fn init_twice_fails() {
        let dir = tempdir().unwrap();
        FileStore::init(dir.path()).unwrap();
        assert!(FileStore::init(dir.path()).is_err());
    }

    #[test]
    fn create_and_read_task() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create("First task".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        assert_eq!(task.id, 1);
        assert_eq!(task.title, "First task");
        let read = store.read(1).unwrap();
        assert_eq!(read.title, "First task");
    }

    #[test]
    fn sequential_ids() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let t1 = store
            .create("A".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        let t2 = store
            .create("B".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        let t3 = store
            .create("C".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        assert_eq!(t1.id, 1);
        assert_eq!(t2.id, 2);
        assert_eq!(t3.id, 3);
    }

    #[test]
    fn list_all_tasks() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        store
            .create("A".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        store
            .create("B".into(), Kind::Epic, None, None, vec![], vec![])
            .unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn delete_task() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        store
            .create("Doomed".into(), Kind::Task, None, None, vec![], vec![])
            .unwrap();
        store.delete(1).unwrap();
        assert!(store.read(1).is_err());
    }

    #[test]
    fn read_nonexistent_fails() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        assert!(store.read(999).is_err());
    }

    #[test]
    fn create_deduplicates_tags_and_deps() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        // Create two tasks so dep references are valid
        store.create("Dep A".into(), Kind::Task, None, None, vec![], vec![]).unwrap();
        store.create("Dep B".into(), Kind::Task, None, None, vec![], vec![]).unwrap();

        // Create a task with duplicate tags and deps
        let task = store.create(
            "Duped".into(),
            Kind::Task,
            None,
            None,
            vec![1, 2, 1, 2, 1],
            vec!["x".into(), "y".into(), "x".into()],
        ).unwrap();

        assert_eq!(task.depends_on, vec![1, 2]);
        assert_eq!(task.tags, vec!["x", "y"]);

        // Verify the persisted file is also deduped
        let read = store.read(task.id).unwrap();
        assert_eq!(read.depends_on, vec![1, 2]);
        assert_eq!(read.tags, vec!["x", "y"]);
    }

    #[test]
    fn lock_file_persists_after_id_allocation() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let lock_path = dir.path().join(".tak").join("counter.lock");

        store.create("A".into(), Kind::Task, None, None, vec![], vec![]).unwrap();
        assert!(lock_path.exists(), "lock file should persist after first allocation");

        store.create("B".into(), Kind::Task, None, None, vec![], vec![]).unwrap();
        assert!(lock_path.exists(), "lock file should persist after second allocation");
    }
}
