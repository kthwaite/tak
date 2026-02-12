use crate::error::Result;
use crate::model::{Status, Task};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn run(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let next_task = select_next_task(&repo, assignee.as_deref())?;

    if let Some(task) = next_task {
        output::print_task(&task, format)?;
    } else if format != Format::Json {
        println!("No available tasks");
    } else {
        println!("null");
    }
    Ok(())
}

fn select_next_task(repo: &Repo, assignee: Option<&str>) -> Result<Option<Task>> {
    let available = repo.index.available(assignee)?;
    if available.is_empty() {
        return Ok(None);
    }

    let tasks_by_id: HashMap<u64, Task> = repo
        .store
        .list_all()?
        .into_iter()
        .map(|task| (task.id, task))
        .collect();

    for candidate in available {
        let id: u64 = candidate.into();
        let Some(task) = tasks_by_id.get(&id) else {
            continue;
        };

        if !matches!(task.status, Status::Pending) {
            continue;
        }

        if has_open_children(task.id, &tasks_by_id) {
            continue;
        }

        if has_invalid_ancestor(task, &tasks_by_id) {
            continue;
        }

        return Ok(Some(task.clone()));
    }

    Ok(None)
}

fn has_open_children(task_id: u64, tasks_by_id: &HashMap<u64, Task>) -> bool {
    tasks_by_id.values().any(|candidate| {
        candidate.parent == Some(task_id)
            && matches!(candidate.status, Status::Pending | Status::InProgress)
    })
}

fn has_invalid_ancestor(task: &Task, tasks_by_id: &HashMap<u64, Task>) -> bool {
    let mut current_parent = task.parent;
    let mut visited = HashSet::<u64>::new();

    while let Some(parent_id) = current_parent {
        if !visited.insert(parent_id) {
            return true;
        }

        let Some(parent) = tasks_by_id.get(&parent_id) else {
            return true;
        };

        if matches!(parent.status, Status::Done | Status::Cancelled) {
            return true;
        }

        current_parent = parent.parent;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use chrono::Utc;
    use tempfile::tempdir;

    fn setup_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        crate::store::files::FileStore::init(dir.path()).unwrap();
        dir
    }

    fn create_task(repo_root: &Path, title: &str, kind: Kind, parent: Option<u64>) -> u64 {
        let repo = Repo::open(repo_root).unwrap();
        let task = repo
            .store
            .create(
                title.to_string(),
                kind,
                None,
                parent,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        repo.index.upsert(&task).unwrap();
        task.id
    }

    fn set_status(repo_root: &Path, id: u64, status: Status) {
        let repo = Repo::open(repo_root).unwrap();
        let mut task = repo.store.read(id).unwrap();
        task.status = status;
        task.updated_at = Utc::now();
        repo.store.write(&task).unwrap();
        repo.index.upsert(&task).unwrap();
    }

    #[test]
    fn select_next_skips_pending_parents_with_open_children() {
        let dir = setup_repo();
        let epic_id = create_task(dir.path(), "epic", Kind::Epic, None);
        let leaf_id = create_task(dir.path(), "leaf", Kind::Task, Some(epic_id));

        let repo = Repo::open(dir.path()).unwrap();
        let next = select_next_task(&repo, None).unwrap().unwrap();

        assert_eq!(next.id, leaf_id);
    }

    #[test]
    fn select_next_skips_leaf_when_ancestor_is_terminal() {
        let dir = setup_repo();
        let parent_id = create_task(dir.path(), "done parent", Kind::Feature, None);
        let _blocked_leaf = create_task(
            dir.path(),
            "leaf under done parent",
            Kind::Task,
            Some(parent_id),
        );
        let fallback_leaf = create_task(dir.path(), "valid leaf", Kind::Task, None);

        set_status(dir.path(), parent_id, Status::Done);

        let repo = Repo::open(dir.path()).unwrap();
        let next = select_next_task(&repo, None).unwrap().unwrap();

        assert_eq!(next.id, fallback_leaf);
    }

    #[test]
    fn select_next_returns_none_when_only_candidates_have_invalid_ancestors() {
        let dir = setup_repo();
        let parent_id = create_task(dir.path(), "cancelled parent", Kind::Feature, None);
        let _child_id = create_task(
            dir.path(),
            "leaf under cancelled",
            Kind::Task,
            Some(parent_id),
        );

        set_status(dir.path(), parent_id, Status::Cancelled);

        let repo = Repo::open(dir.path()).unwrap();
        let next = select_next_task(&repo, None).unwrap();

        assert!(next.is_none());
    }
}
