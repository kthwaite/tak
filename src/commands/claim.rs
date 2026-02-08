use std::path::Path;

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::{self, Format};
use crate::store::lock;
use crate::store::repo::Repo;
use crate::{git, model};

pub fn run(repo_root: &Path, assignee: String, tag: Option<String>, format: Format) -> Result<()> {
    let lock_path = repo_root.join(".tak").join("claim.lock");
    let lock_file = lock::acquire_lock(&lock_path)?;

    let repo = Repo::open(repo_root)?;
    let available = repo.index.available(Some(&assignee))?;

    let id = if let Some(ref tg) = tag {
        // Find first available task with matching tag
        let mut found = None;
        for &aid in &available {
            if let Ok(t) = repo.store.read(aid)
                && t.tags.contains(tg)
            {
                found = Some(aid);
                break;
            }
        }
        match found {
            Some(id) => id,
            None => {
                lock::release_lock(lock_file)?;
                return Err(TakError::NoAvailableTask);
            }
        }
    } else {
        match available.first() {
            Some(&id) => id,
            None => {
                lock::release_lock(lock_file)?;
                return Err(TakError::NoAvailableTask);
            }
        }
    };

    let mut task = repo.store.read(id)?;
    task.status = Status::InProgress;
    task.assignee = Some(assignee);

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

    lock::release_lock(lock_file)?;

    output::print_task(&task, format)?;
    Ok(())
}
