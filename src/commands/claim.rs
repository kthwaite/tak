use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::model::{Kind, Status, Task};
use crate::output::{self, Format};
use crate::store::coordination::CoordinationLinks;
use crate::store::lock;
use crate::store::repo::Repo;
use crate::store::sidecars::HistoryEvent;
use crate::store::work::WorkClaimStrategy;
use crate::{git, model};

pub fn run(repo_root: &Path, assignee: String, tag: Option<String>, format: Format) -> Result<()> {
    let task = claim_next(
        repo_root,
        &assignee,
        tag.as_deref(),
        WorkClaimStrategy::default(),
    )?
    .ok_or(TakError::NoAvailableTask)?;
    output::print_task(&task, format)?;
    Ok(())
}

/// Atomically claim and start the next available task for an assignee.
///
/// Returns `Ok(None)` when no matching task is available.
pub fn claim_next(
    repo_root: &Path,
    assignee: &str,
    tag: Option<&str>,
    strategy: WorkClaimStrategy,
) -> Result<Option<Task>> {
    let lock_path = repo_root.join(".tak").join("claim.lock");
    let lock_file = lock::acquire_lock(&lock_path)?;

    let result = claim_next_locked(repo_root, assignee, tag, strategy);
    lock::release_lock(lock_file)?;
    result
}

fn claim_next_locked(
    repo_root: &Path,
    assignee: &str,
    tag: Option<&str>,
    strategy: WorkClaimStrategy,
) -> Result<Option<Task>> {
    let repo = Repo::open(repo_root)?;
    let available = repo.index.available(Some(assignee))?;
    let Some(id) = select_available_task_id(&repo, &available, tag, strategy)? else {
        return Ok(None);
    };

    let mut task = repo.store.read(id)?;
    task.status = Status::InProgress;
    task.execution.attempt_count += 1;
    task.assignee = Some(assignee.to_string());

    // Capture git HEAD on first start (only if not already set)
    if task.git.start_commit.is_none()
        && let Some(info) = git::current_head_info(repo_root)
    {
        task.git = model::GitInfo {
            branch: info.branch,
            start_commit: Some(info.sha),
            ..task.git
        };
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "claimed".into(),
        agent: task.assignee.clone(),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    Ok(Some(task))
}

fn select_available_task_id(
    repo: &Repo,
    available: &[crate::task_id::TaskId],
    tag: Option<&str>,
    strategy: WorkClaimStrategy,
) -> Result<Option<u64>> {
    match strategy {
        WorkClaimStrategy::PriorityThenAge => {
            select_available_task_id_default(repo, available, tag)
        }
        WorkClaimStrategy::EpicCloseout => {
            select_available_task_id_epic_closeout(repo, available, tag)
        }
    }
}

fn select_available_task_id_default(
    repo: &Repo,
    available: &[crate::task_id::TaskId],
    tag: Option<&str>,
) -> Result<Option<u64>> {
    if let Some(tag) = tag {
        for aid in available {
            let aid: u64 = aid.clone().into();
            if let Ok(task) = repo.store.read(aid)
                && task.tags.iter().any(|t| t == tag)
            {
                return Ok(Some(aid));
            }
        }
        Ok(None)
    } else {
        Ok(available.first().map(|id| id.clone().into()))
    }
}

fn select_available_task_id_epic_closeout(
    repo: &Repo,
    available: &[crate::task_id::TaskId],
    tag: Option<&str>,
) -> Result<Option<u64>> {
    if available.is_empty() {
        return Ok(None);
    }

    let tasks_by_id: HashMap<u64, Task> = repo
        .store
        .list_all()?
        .into_iter()
        .map(|task| (task.id, task))
        .collect();

    let mut epic_created_at_ms: HashMap<u64, i64> = HashMap::new();
    for task in tasks_by_id.values() {
        if matches!(task.kind, Kind::Epic) {
            epic_created_at_ms.insert(task.id, task.created_at.timestamp_millis());
        }
    }

    let mut remaining_open_by_epic: HashMap<u64, usize> = HashMap::new();
    for task in tasks_by_id.values() {
        if matches!(task.status, Status::Done | Status::Cancelled) {
            continue;
        }

        if let Some(epic_id) = root_epic_id(task.id, &tasks_by_id)
            && task.id != epic_id
        {
            *remaining_open_by_epic.entry(epic_id).or_insert(0) += 1;
        }
    }

    struct Candidate {
        id: u64,
        epic_id: Option<u64>,
        epic_created_at_ms: Option<i64>,
        epic_remaining_open: usize,
        base_rank: usize,
    }

    let mut candidates = Vec::<Candidate>::new();
    for (base_rank, aid) in available.iter().enumerate() {
        let id: u64 = aid.clone().into();
        let Some(task) = tasks_by_id.get(&id) else {
            continue;
        };

        if let Some(tag) = tag
            && !task.tags.iter().any(|task_tag| task_tag == tag)
        {
            continue;
        }

        let epic_id = root_epic_id(id, &tasks_by_id);
        let epic_created_at_ms = epic_id.and_then(|eid| epic_created_at_ms.get(&eid).copied());
        let epic_remaining_open = epic_id
            .and_then(|eid| remaining_open_by_epic.get(&eid).copied())
            .unwrap_or(0);

        if matches!(task.kind, Kind::Epic) && epic_remaining_open > 0 {
            continue;
        }

        candidates.push(Candidate {
            id,
            epic_id,
            epic_created_at_ms,
            epic_remaining_open,
            base_rank,
        });
    }

    candidates.sort_by(|a, b| {
        epic_priority_rank(a.epic_id)
            .cmp(&epic_priority_rank(b.epic_id))
            .then_with(|| compare_opt_i64(a.epic_created_at_ms, b.epic_created_at_ms))
            .then_with(|| a.epic_remaining_open.cmp(&b.epic_remaining_open))
            .then_with(|| a.base_rank.cmp(&b.base_rank))
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(candidates.first().map(|candidate| candidate.id))
}

fn epic_priority_rank(epic_id: Option<u64>) -> u8 {
    if epic_id.is_some() { 0 } else { 1 }
}

fn compare_opt_i64(a: Option<i64>, b: Option<i64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => Ordering::Equal,
    }
}

fn root_epic_id(task_id: u64, tasks_by_id: &HashMap<u64, Task>) -> Option<u64> {
    let mut current = Some(task_id);
    let mut visited = HashSet::<u64>::new();
    let mut root_epic = None;

    while let Some(current_id) = current {
        if !visited.insert(current_id) {
            break;
        }

        let Some(task) = tasks_by_id.get(&current_id) else {
            break;
        };

        if matches!(task.kind, Kind::Epic) {
            root_epic = Some(task.id);
        }

        current = task.parent;
    }

    root_epic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use chrono::Duration;
    use tempfile::tempdir;

    fn setup_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        crate::store::files::FileStore::init(dir.path()).unwrap();
        dir
    }

    fn create_task(
        repo_root: &Path,
        title: &str,
        kind: Kind,
        parent: Option<u64>,
        tags: Vec<String>,
    ) -> u64 {
        let repo = Repo::open(repo_root).unwrap();
        let task = repo
            .store
            .create(
                title.to_string(),
                kind,
                None,
                parent,
                vec![],
                tags,
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        repo.index.upsert(&task).unwrap();
        task.id
    }

    fn set_created_at(repo_root: &Path, task_id: u64, created_at: chrono::DateTime<Utc>) {
        let repo = Repo::open(repo_root).unwrap();
        let mut task = repo.store.read(task_id).unwrap();
        task.created_at = created_at;
        task.updated_at = created_at;
        repo.store.write(&task).unwrap();
        repo.index.upsert(&task).unwrap();
    }

    #[test]
    fn claim_next_returns_none_when_no_work_is_available() {
        let dir = setup_repo();
        let claimed = claim_next(
            dir.path(),
            "agent-1",
            None,
            WorkClaimStrategy::PriorityThenAge,
        )
        .unwrap();
        assert!(claimed.is_none());
    }

    #[test]
    fn claim_next_marks_task_in_progress_and_assigns_agent() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "todo", Kind::Task, None, vec![]);

        let claimed = claim_next(
            dir.path(),
            "agent-1",
            None,
            WorkClaimStrategy::PriorityThenAge,
        )
        .unwrap()
        .unwrap();
        assert_eq!(claimed.id, task_id);
        assert_eq!(claimed.status, Status::InProgress);
        assert_eq!(claimed.assignee.as_deref(), Some("agent-1"));
        assert_eq!(claimed.execution.attempt_count, 1);
    }

    #[test]
    fn claim_next_respects_tag_filter() {
        let dir = setup_repo();
        let _ = create_task(dir.path(), "task-a", Kind::Task, None, vec!["alpha".into()]);
        let tagged_id = create_task(dir.path(), "task-b", Kind::Task, None, vec!["beta".into()]);

        let claimed = claim_next(
            dir.path(),
            "agent-1",
            Some("beta"),
            WorkClaimStrategy::PriorityThenAge,
        )
        .unwrap()
        .unwrap();
        assert_eq!(claimed.id, tagged_id);
    }

    #[test]
    fn epic_closeout_prefers_oldest_epic_even_if_it_has_more_remaining_work() {
        let dir = setup_repo();

        let old_epic = create_task(dir.path(), "old epic", Kind::Epic, None, vec![]);
        let old_child = create_task(dir.path(), "old task", Kind::Task, Some(old_epic), vec![]);
        let _old_child_extra =
            create_task(dir.path(), "old extra", Kind::Task, Some(old_epic), vec![]);

        let new_epic = create_task(dir.path(), "new epic", Kind::Epic, None, vec![]);
        let _new_child = create_task(dir.path(), "new task", Kind::Task, Some(new_epic), vec![]);

        let claimed = claim_next(dir.path(), "agent-1", None, WorkClaimStrategy::EpicCloseout)
            .unwrap()
            .unwrap();

        assert_eq!(claimed.id, old_child);
    }

    #[test]
    fn epic_closeout_breaks_epic_age_ties_by_fewest_remaining_open_tasks() {
        let dir = setup_repo();

        let epic_a = create_task(dir.path(), "epic a", Kind::Epic, None, vec![]);
        let a_child_1 = create_task(dir.path(), "a1", Kind::Task, Some(epic_a), vec![]);
        let _a_child_2 = create_task(dir.path(), "a2", Kind::Task, Some(epic_a), vec![]);

        let epic_b = create_task(dir.path(), "epic b", Kind::Epic, None, vec![]);
        let b_child_1 = create_task(dir.path(), "b1", Kind::Task, Some(epic_b), vec![]);

        let tie_ts = Utc::now() - Duration::hours(1);
        set_created_at(dir.path(), epic_a, tie_ts);
        set_created_at(dir.path(), epic_b, tie_ts);

        let claimed = claim_next(dir.path(), "agent-1", None, WorkClaimStrategy::EpicCloseout)
            .unwrap()
            .unwrap();

        assert_ne!(claimed.id, a_child_1);
        assert_eq!(claimed.id, b_child_1);
    }
}
