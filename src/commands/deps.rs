use std::path::Path;
use chrono::Utc;
use crate::error::{Result, TakError};
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn depend(repo_root: &Path, id: u64, on: Vec<u64>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    for dep_id in &on {
        repo.store.read(*dep_id)?; // validate exists
        if repo.index.would_cycle(id, *dep_id)? {
            return Err(TakError::CycleDetected(id));
        }
        if !task.depends_on.contains(dep_id) {
            task.depends_on.push(*dep_id);
            // Update index immediately so subsequent cycle checks see this edge
            repo.index.upsert(&task)?;
        }
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}

pub fn undepend(repo_root: &Path, id: u64, on: Vec<u64>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.depends_on.retain(|d| !on.contains(d));
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}

pub fn reparent(repo_root: &Path, id: u64, to: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    repo.store.read(to)?; // validate parent exists
    if repo.index.would_parent_cycle(id, to)? {
        return Err(TakError::CycleDetected(id));
    }
    let mut task = repo.store.read(id)?;

    task.parent = Some(to);
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}

pub fn orphan(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.parent = None;
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}
