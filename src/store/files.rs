use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::model::{Contract, Dependency, Execution, GitInfo, Kind, Planning, Status, Task};
use crate::store::lock;
use crate::task_id::TaskId;

const TASK_ID_ALLOCATION_MAX_ATTEMPTS: usize = 128;

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
        fs::write(
            root.join("config.json"),
            r#"{
  "version": 2,
  "mesh": {
    "registration_ttl_secs": 900,
    "reservation_ttl_secs": 1800,
    "heartbeat_interval_secs": 30
  }
}"#,
        )?;

        Ok(Self { root })
    }

    fn tasks_dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    fn task_path(&self, id: u64) -> PathBuf {
        let file_id = TaskId::from(id);
        self.tasks_dir().join(format!("{file_id}.json"))
    }

    fn legacy_task_path(&self, id: u64) -> PathBuf {
        self.tasks_dir().join(format!("{id}.json"))
    }

    fn resolve_task_path(&self, id: u64) -> Option<PathBuf> {
        let canonical = self.task_path(id);
        if canonical.exists() {
            return Some(canonical);
        }

        let legacy = self.legacy_task_path(id);
        if legacy.exists() {
            return Some(legacy);
        }

        None
    }

    fn task_files(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(self.tasks_dir())? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".json") {
                files.push((stem.to_string(), path));
            }
        }

        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    fn allocation_lock_path(&self) -> PathBuf {
        self.root.join("task-id.lock")
    }

    fn next_available_id(&self) -> Result<u64> {
        self.next_available_id_with(TaskId::generate)
    }

    fn next_available_id_with<F>(&self, mut generate: F) -> Result<u64>
    where
        F: FnMut() -> std::result::Result<TaskId, crate::task_id::TaskIdGenerationError>,
    {
        for _ in 0..TASK_ID_ALLOCATION_MAX_ATTEMPTS {
            let candidate = generate().map_err(|err| std::io::Error::other(err.to_string()))?;
            let id = candidate.as_u64();
            if self.resolve_task_path(id).is_none() {
                return Ok(id);
            }
        }

        Err(std::io::Error::other(format!(
            "failed to allocate unique task id after {TASK_ID_ALLOCATION_MAX_ATTEMPTS} attempts"
        ))
        .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        title: String,
        kind: Kind,
        description: Option<String>,
        parent: Option<u64>,
        depends_on: Vec<u64>,
        tags: Vec<String>,
        contract: Contract,
        planning: Planning,
    ) -> Result<Task> {
        if let Some(pid) = parent {
            self.read(pid)?;
        }
        for &dep in &depends_on {
            self.read(dep)?;
        }

        let _allocation_lock = lock::acquire_lock(&self.allocation_lock_path())?;
        let id = self.next_available_id()?;

        let now = Utc::now();
        let mut task = Task {
            id,
            title,
            description,
            status: Status::Pending,
            kind,
            parent,
            depends_on: depends_on.into_iter().map(Dependency::simple).collect(),
            assignee: None,
            tags,
            contract,
            planning,
            git: GitInfo::default(),
            execution: Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        task.normalize();

        let json = serde_json::to_string_pretty(&task)?;
        fs::write(self.task_path(id), json)?;
        Ok(task)
    }

    pub fn read(&self, id: u64) -> Result<Task> {
        let Some(path) = self.resolve_task_path(id) else {
            return Err(TakError::TaskNotFound(id));
        };
        let data = fs::read_to_string(path)?;
        let task: Task = serde_json::from_str(&data)?;
        Ok(task)
    }

    pub fn write(&self, task: &Task) -> Result<()> {
        let json = serde_json::to_string_pretty(task)?;
        let path = self
            .resolve_task_path(task.id)
            .unwrap_or_else(|| self.task_path(task.id));
        fs::write(path, json)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        let Some(path) = self.resolve_task_path(id) else {
            return Err(TakError::TaskNotFound(id));
        };
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn list_ids(&self) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        for (_, path) in self.task_files()? {
            let data = fs::read_to_string(path)?;
            let task: Task = serde_json::from_str(&data)?;
            ids.push(task.id);
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Compute a fingerprint from task file metadata (filename, size, mtime).
    /// Cheap (stat calls, no file reads) and detects additions, deletions,
    /// and in-place edits. Uses nanosecond mtime to catch rapid same-size edits.
    pub fn fingerprint(&self) -> Result<String> {
        let mut entries = Vec::new();
        for (file_id, path) in self.task_files()? {
            let meta = fs::metadata(path)?;
            let mtime = meta
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let size = meta.len();
            entries.push((file_id, size, mtime));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let fp = entries
            .iter()
            .map(|(file_id, size, mtime)| format!("{file_id}:{size}:{mtime}"))
            .collect::<Vec<_>>()
            .join(",");
        Ok(fp)
    }

    pub fn read_many(&self, ids: &[u64]) -> Result<Vec<Task>> {
        ids.iter().map(|&id| self.read(id)).collect()
    }

    pub fn list_all(&self) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        for (_, path) in self.task_files()? {
            let data = fs::read_to_string(path)?;
            let task: Task = serde_json::from_str(&data)?;
            tasks.push(task);
        }
        tasks.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(tasks)
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
        assert!(!store.root().join("counter.json").exists());
        assert!(store.root().join("tasks").is_dir());
    }

    #[test]
    fn init_writes_mesh_lease_defaults() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let config_path = store.root().join("config.json");
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();

        assert_eq!(config["version"], serde_json::json!(2));
        assert_eq!(
            config["mesh"]["registration_ttl_secs"],
            serde_json::json!(900)
        );
        assert_eq!(
            config["mesh"]["reservation_ttl_secs"],
            serde_json::json!(1800)
        );
        assert_eq!(
            config["mesh"]["heartbeat_interval_secs"],
            serde_json::json!(30)
        );
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
            .create(
                "First task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        assert_eq!(TaskId::from(task.id).as_str().len(), TaskId::HEX_LEN);
        assert_eq!(task.title, "First task");
        let read = store.read(task.id).unwrap();
        assert_eq!(read.title, "First task");
    }

    #[test]
    fn create_does_not_create_counter_artifacts() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let _task = store
            .create(
                "No counter".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        assert!(!store.root().join("counter.json").exists());
        assert!(!store.root().join("counter.lock").exists());
    }

    #[test]
    fn create_uses_hash_style_task_filename() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Hash path".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let hash_path = store
            .root()
            .join("tasks")
            .join(format!("{}.json", TaskId::from(task.id)));
        assert!(hash_path.exists());

        let legacy_path = store.root().join("tasks").join(format!("{}.json", task.id));
        assert!(!legacy_path.exists());
    }

    #[test]
    fn read_supports_legacy_numeric_filename_fallback() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Legacy read".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let hash_path = store
            .root()
            .join("tasks")
            .join(format!("{}.json", TaskId::from(task.id)));
        let legacy_path = store.root().join("tasks").join(format!("{}.json", task.id));
        fs::rename(&hash_path, &legacy_path).unwrap();

        let read = store.read(task.id).unwrap();
        assert_eq!(read.title, "Legacy read");
    }

    #[test]
    fn generated_ids_are_unique() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let t1 = store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let t2 = store
            .create(
                "B".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let t3 = store
            .create(
                "C".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        assert_ne!(t1.id, t2.id);
        assert_ne!(t1.id, t3.id);
        assert_ne!(t2.id, t3.id);
    }

    #[test]
    fn list_all_tasks() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        store
            .create(
                "B".into(),
                Kind::Epic,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_all_orders_by_created_at_then_id() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let t1 = store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let t2 = store
            .create(
                "B".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let t3 = store
            .create(
                "C".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let base = Utc::now();
        let mut task1 = store.read(t1.id).unwrap();
        task1.created_at = base + chrono::Duration::seconds(20);
        task1.updated_at = task1.created_at;
        store.write(&task1).unwrap();

        let mut task2 = store.read(t2.id).unwrap();
        task2.created_at = base + chrono::Duration::seconds(10);
        task2.updated_at = task2.created_at;
        store.write(&task2).unwrap();

        let mut task3 = store.read(t3.id).unwrap();
        task3.created_at = base + chrono::Duration::seconds(10);
        task3.updated_at = task3.created_at;
        store.write(&task3).unwrap();

        let all = store.list_all().unwrap();
        let ids: Vec<u64> = all.into_iter().map(|t| t.id).collect();
        let mut expected_same_timestamp = vec![t2.id, t3.id];
        expected_same_timestamp.sort_unstable();
        let mut expected = expected_same_timestamp;
        expected.push(t1.id);
        assert_eq!(ids, expected);
    }

    #[test]
    fn list_ids_uses_task_payload_not_filename() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        fs::rename(
            store
                .tasks_dir()
                .join(format!("{}.json", TaskId::from(task.id))),
            store.tasks_dir().join("deadbeefdeadbeef.json"),
        )
        .unwrap();

        assert_eq!(store.list_ids().unwrap(), vec![task.id]);
    }

    #[test]
    fn list_all_reads_tasks_from_non_numeric_filenames() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        fs::rename(
            store
                .tasks_dir()
                .join(format!("{}.json", TaskId::from(task.id))),
            store.tasks_dir().join("deadbeefdeadbeef.json"),
        )
        .unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, task.id);
        assert_eq!(all[0].title, "Task");
    }

    #[test]
    fn fingerprint_includes_non_numeric_filenames() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        fs::rename(
            store
                .tasks_dir()
                .join(format!("{}.json", TaskId::from(task.id))),
            store.tasks_dir().join("deadbeefdeadbeef.json"),
        )
        .unwrap();

        let fp = store.fingerprint().unwrap();
        assert!(fp.contains("deadbeefdeadbeef:"));
    }

    #[test]
    fn delete_task() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "Doomed".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        store.delete(task.id).unwrap();
        assert!(store.read(task.id).is_err());
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
        let dep_a = store
            .create(
                "Dep A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let dep_b = store
            .create(
                "Dep B".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        // Create a task with duplicate tags and deps
        let task = store
            .create(
                "Duped".into(),
                Kind::Task,
                None,
                None,
                vec![dep_a.id, dep_b.id, dep_a.id, dep_b.id, dep_a.id],
                vec!["x".into(), "y".into(), "x".into()],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let mut expected_dep_ids = vec![dep_a.id, dep_b.id];
        expected_dep_ids.sort_unstable();
        let expected_deps: Vec<Dependency> = expected_dep_ids
            .into_iter()
            .map(Dependency::simple)
            .collect();

        assert_eq!(task.depends_on, expected_deps);
        assert_eq!(task.tags, vec!["x", "y"]);

        // Verify the persisted file is also deduped
        let read = store.read(task.id).unwrap();
        assert_eq!(read.depends_on, expected_deps);
        assert_eq!(read.tags, vec!["x", "y"]);
    }

    #[test]
    fn next_available_id_retries_on_collision() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let existing = store
            .create(
                "Existing".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let available_id = existing.id ^ 1;
        let mut calls = 0;
        let allocated = store
            .next_available_id_with(|| {
                calls += 1;
                if calls == 1 {
                    Ok(TaskId::from(existing.id))
                } else {
                    Ok(TaskId::from(available_id))
                }
            })
            .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(allocated, available_id);
    }

    #[test]
    fn next_available_id_succeeds_on_final_retry_attempt() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let existing = store
            .create(
                "Existing".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let available_id = existing.id ^ 1;
        let mut calls = 0;
        let allocated = store
            .next_available_id_with(|| {
                calls += 1;
                if calls < TASK_ID_ALLOCATION_MAX_ATTEMPTS {
                    Ok(TaskId::from(existing.id))
                } else {
                    Ok(TaskId::from(available_id))
                }
            })
            .unwrap();

        assert_eq!(allocated, available_id);
        assert_eq!(calls, TASK_ID_ALLOCATION_MAX_ATTEMPTS);
    }

    #[test]
    fn next_available_id_fails_after_retry_limit() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let existing = store
            .create(
                "Existing".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let mut calls = 0;
        let err = store
            .next_available_id_with(|| {
                calls += 1;
                Ok(TaskId::from(existing.id))
            })
            .unwrap_err();

        assert_eq!(calls, TASK_ID_ALLOCATION_MAX_ATTEMPTS);
        assert!(
            err.to_string()
                .contains("failed to allocate unique task id"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn next_available_id_propagates_generator_errors() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let err = store
            .next_available_id_with(|| {
                Err(crate::task_id::TaskIdGenerationError::RandomSource(
                    "entropy unavailable".to_string(),
                ))
            })
            .unwrap_err();

        assert!(
            err.to_string().contains("task id generation failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn lock_file_persists_after_id_allocation() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let lock_path = dir.path().join(".tak").join("task-id.lock");

        store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        assert!(
            lock_path.exists(),
            "lock file should persist after first allocation"
        );

        store
            .create(
                "B".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        assert!(
            lock_path.exists(),
            "lock file should persist after second allocation"
        );
    }
}
