use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::{Result, TakError};
use crate::model::{Dependency, Task};
use crate::task_id::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRewriteSummary {
    pub rewritten: usize,
}

/// Rewrite task payload IDs and task filenames based on an old->new ID map,
/// then atomically swap the `.tak/tasks` directory to avoid partial writes.
pub fn rewrite_task_files_atomic(
    tasks_dir: &Path,
    id_map: &HashMap<u64, u64>,
) -> Result<TaskRewriteSummary> {
    let source_paths = task_json_paths(tasks_dir)?;
    let source_tasks = read_tasks(&source_paths)?;
    let rewritten_tasks = rewrite_tasks_with_mapping(&source_tasks, id_map)?;

    let Some(parent_dir) = tasks_dir.parent() else {
        return Err(TakError::Locked(format!(
            "task directory '{}' has no parent directory",
            tasks_dir.display()
        )));
    };

    let nonce = Uuid::new_v4();
    let staging_dir = parent_dir.join(format!("tasks.migrate.{nonce}.staging"));
    let backup_dir = parent_dir.join(format!("tasks.migrate.{nonce}.backup"));

    fs::create_dir_all(&staging_dir)?;
    if let Err(err) = write_tasks(&staging_dir, &rewritten_tasks) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(err);
    }

    fs::rename(tasks_dir, &backup_dir)?;

    if let Err(err) = fs::rename(&staging_dir, tasks_dir) {
        let rollback_err = fs::rename(&backup_dir, tasks_dir).err();
        let _ = fs::remove_dir_all(&staging_dir);

        if let Some(rollback_err) = rollback_err {
            return Err(TakError::Locked(format!(
                "task migration swap failed and rollback failed; backup left at {}: swap error: {}; rollback error: {}",
                backup_dir.display(),
                err,
                rollback_err
            )));
        }

        return Err(err.into());
    }

    let _ = fs::remove_dir_all(&backup_dir);

    Ok(TaskRewriteSummary {
        rewritten: rewritten_tasks.len(),
    })
}

/// Apply an old->new ID map to task payloads (id, parent, dependencies).
pub fn rewrite_tasks_with_mapping(tasks: &[Task], id_map: &HashMap<u64, u64>) -> Result<Vec<Task>> {
    let mut seen_old_ids = HashSet::new();
    let mut seen_new_ids = HashSet::new();
    let mut rewritten = Vec::with_capacity(tasks.len());

    for task in tasks {
        if !seen_old_ids.insert(task.id) {
            return Err(TakError::Locked(format!(
                "duplicate source task id in input: {}",
                TaskId::from(task.id)
            )));
        }

        let new_id = remap_id(task.id, id_map, "task")?;
        if !seen_new_ids.insert(new_id) {
            return Err(TakError::Locked(format!(
                "id mapping is not one-to-one: multiple tasks map to {}",
                TaskId::from(new_id)
            )));
        }

        let mut next = task.clone();
        next.id = new_id;
        next.parent = next
            .parent
            .map(|parent| remap_id(parent, id_map, "parent"))
            .transpose()?;
        next.depends_on = next
            .depends_on
            .iter()
            .map(|dep| {
                Ok(Dependency {
                    id: remap_id(dep.id, id_map, "dependency")?,
                    dep_type: dep.dep_type,
                    reason: dep.reason.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        next.normalize();

        rewritten.push(next);
    }

    Ok(rewritten)
}

fn remap_id(old_id: u64, id_map: &HashMap<u64, u64>, field: &str) -> Result<u64> {
    id_map.get(&old_id).copied().ok_or_else(|| {
        TakError::Locked(format!(
            "missing id mapping for {field} reference {}",
            TaskId::from(old_id)
        ))
    })
}

fn task_json_paths(tasks_dir: &Path) -> Result<Vec<PathBuf>> {
    if !tasks_dir.exists() {
        return Err(TakError::Locked(format!(
            "task directory '{}' does not exist",
            tasks_dir.display()
        )));
    }

    let mut paths = Vec::new();

    for entry in fs::read_dir(tasks_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }

    paths.sort();
    Ok(paths)
}

fn read_tasks(paths: &[PathBuf]) -> Result<Vec<Task>> {
    let mut tasks = Vec::with_capacity(paths.len());

    for path in paths {
        let data = fs::read_to_string(path)?;
        let task: Task = serde_json::from_str(&data)?;
        tasks.push(task);
    }

    tasks.sort_by_key(|task| task.id);
    Ok(tasks)
}

fn write_tasks(dir: &Path, tasks: &[Task]) -> Result<()> {
    for task in tasks {
        let filename = format!("{}.json", TaskId::from(task.id));
        let json = serde_json::to_string_pretty(task)?;
        fs::write(dir.join(filename), json)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use crate::store::files::FileStore;
    use tempfile::tempdir;

    fn setup_two_tasks_with_parent_dependency() -> (tempfile::TempDir, FileStore, u64, u64) {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let parent = store
            .create(
                "Parent".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let child = store
            .create(
                "Child".into(),
                Kind::Task,
                None,
                Some(parent.id),
                vec![parent.id],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        (dir, store, parent.id, child.id)
    }

    #[test]
    fn rewrite_task_files_atomic_updates_ids_references_and_filenames() {
        let (_dir, store, parent_id, child_id) = setup_two_tasks_with_parent_dependency();

        let mut map = HashMap::new();
        map.insert(parent_id, 42);
        map.insert(child_id, 99);

        let summary = rewrite_task_files_atomic(&store.root().join("tasks"), &map).unwrap();
        assert_eq!(summary.rewritten, 2);

        let tasks_dir = store.root().join("tasks");
        let new_parent_path = tasks_dir.join(format!("{}.json", TaskId::from(42)));
        let new_child_path = tasks_dir.join(format!("{}.json", TaskId::from(99)));
        assert!(new_parent_path.exists());
        assert!(new_child_path.exists());

        assert!(
            !tasks_dir
                .join(format!("{}.json", TaskId::from(parent_id)))
                .exists()
        );
        assert!(
            !tasks_dir
                .join(format!("{}.json", TaskId::from(child_id)))
                .exists()
        );

        let rewritten_parent: Task =
            serde_json::from_str(&fs::read_to_string(new_parent_path).unwrap()).unwrap();
        let rewritten_child: Task =
            serde_json::from_str(&fs::read_to_string(new_child_path).unwrap()).unwrap();

        assert_eq!(rewritten_parent.id, 42);
        assert_eq!(rewritten_child.id, 99);
        assert_eq!(rewritten_child.parent, Some(42));
        assert_eq!(rewritten_child.depends_on, vec![Dependency::simple(42)]);
    }

    #[test]
    fn rewrite_task_files_atomic_errors_when_mapping_missing() {
        let (_dir, store, parent_id, child_id) = setup_two_tasks_with_parent_dependency();

        let mut map = HashMap::new();
        map.insert(parent_id, 42);
        // Intentionally missing mapping for child_id.

        let err = rewrite_task_files_atomic(&store.root().join("tasks"), &map).unwrap_err();
        assert!(matches!(err, TakError::Locked(_)));

        let tasks_dir = store.root().join("tasks");
        assert!(
            tasks_dir
                .join(format!("{}.json", TaskId::from(parent_id)))
                .exists()
        );
        assert!(
            tasks_dir
                .join(format!("{}.json", TaskId::from(child_id)))
                .exists()
        );
        assert!(
            !tasks_dir
                .join(format!("{}.json", TaskId::from(42)))
                .exists()
        );
    }

    #[test]
    fn rewrite_tasks_with_mapping_rejects_duplicate_targets() {
        let (_dir, store, parent_id, child_id) = setup_two_tasks_with_parent_dependency();
        let tasks = store.list_all().unwrap();

        let mut map = HashMap::new();
        map.insert(parent_id, 7);
        map.insert(child_id, 7);

        let err = rewrite_tasks_with_mapping(&tasks, &map).unwrap_err();
        assert!(matches!(err, TakError::Locked(_)));
        assert!(err.to_string().contains("id mapping is not one-to-one"));
    }
}
