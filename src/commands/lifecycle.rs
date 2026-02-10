use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::{self, Format};
use crate::store::coordination::{CoordinationLinks, derive_links_from_text};
use crate::store::repo::Repo;
use crate::store::sidecars::HistoryEvent;
use crate::{git, model};
use chrono::Utc;
use std::path::Path;

fn transition(current: Status, target: Status) -> std::result::Result<(), (String, String)> {
    let allowed = match current {
        Status::Pending => matches!(target, Status::InProgress | Status::Cancelled),
        Status::InProgress => matches!(target, Status::Done | Status::Cancelled | Status::Pending),
        Status::Done => matches!(target, Status::Pending),
        Status::Cancelled => matches!(target, Status::Pending),
    };
    if allowed {
        Ok(())
    } else {
        Err((current.to_string(), target.to_string()))
    }
}

fn handoff_links_from_summary(summary: &str) -> CoordinationLinks {
    let mut links = derive_links_from_text(summary);
    links.normalize();
    links
}

pub fn start(repo_root: &Path, id: u64, assignee: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::InProgress)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    if repo.index.is_blocked(id)? {
        return Err(TakError::TaskBlocked(id));
    }

    task.status = Status::InProgress;
    task.execution.attempt_count += 1;
    if let Some(ref a) = assignee {
        task.assignee = Some(a.clone());
    }

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
        event: "started".into(),
        agent: task.assignee.clone(),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn finish(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Done)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Done;

    // Capture end commit and collect commit range if start_commit exists
    if let Some(info) = git::current_head_info(repo_root) {
        task.git.end_commit = Some(info.sha.clone());

        if let Some(ref start) = task.git.start_commit {
            task.git.commits = git::commits_since(repo_root, start, &info.sha);
        }
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "finished".into(),
        agent: task.assignee.clone(),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn cancel(repo_root: &Path, id: u64, reason: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Cancelled)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Cancelled;
    if let Some(ref r) = reason {
        task.execution.last_error = Some(r.clone());
    }
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let mut detail = serde_json::Map::new();
    if let Some(ref r) = reason {
        detail.insert("reason".into(), serde_json::Value::String(r.clone()));
    }
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "cancelled".into(),
        agent: task.assignee.clone(),
        detail,
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn handoff(repo_root: &Path, id: u64, summary: String, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Pending)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Pending;
    task.assignee = None;
    task.execution.handoff_summary = Some(summary.clone());
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let mut detail = serde_json::Map::new();
    let links = handoff_links_from_summary(&summary);
    detail.insert("summary".into(), serde_json::Value::String(summary));

    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "handoff".into(),
        agent: None,
        detail,
        links,
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn reopen(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Pending)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Pending;
    task.assignee = None;
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "reopened".into(),
        agent: None,
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn unassign(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.assignee = None;
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "unassigned".into(),
        agent: None,
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_links_extracts_blackboard_and_mesh_refs() {
        let links = handoff_links_from_summary(
            "handoff via B17 after mesh ping 550e8400-e29b-41d4-a716-446655440000",
        );

        assert_eq!(links.blackboard_note_ids, vec![17]);
        assert_eq!(
            links.mesh_message_ids,
            vec!["550e8400-e29b-41d4-a716-446655440000"]
        );
    }
}
