use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::{self, Format};
use crate::store::repo::Repo;
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

fn set_status(
    repo_root: &Path,
    id: u64,
    target: Status,
    assignee: Option<String>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, target).map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = target;
    if let Some(a) = assignee {
        task.assignee = Some(a);
    }
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    output::print_task(&task, format)?;
    Ok(())
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
    if let Some(a) = assignee {
        task.assignee = Some(a);
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

    output::print_task(&task, format)?;
    Ok(())
}

pub fn finish(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    set_status(repo_root, id, Status::Done, None, format)
}

pub fn cancel(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    set_status(repo_root, id, Status::Cancelled, None, format)
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

    output::print_task(&task, format)?;
    Ok(())
}
