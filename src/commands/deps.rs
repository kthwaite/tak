use crate::error::{Result, TakError};
use crate::model::{DepType, Dependency};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use chrono::Utc;
use std::path::Path;

pub fn depend(
    repo_root: &Path,
    id: u64,
    on: Vec<u64>,
    dep_type: Option<DepType>,
    reason: Option<String>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    // Phase 1: validate all deps exist and won't cycle
    for &dep_id in &on {
        repo.store.read(dep_id)?; // validate exists
        if repo.index.would_cycle(id, dep_id)? {
            return Err(TakError::CycleDetected(id));
        }
    }

    // Phase 2: mutate task in memory
    for &dep_id in &on {
        if !task.depends_on.iter().any(|d| d.id == dep_id) {
            task.depends_on.push(Dependency {
                id: dep_id,
                dep_type,
                reason: reason.clone(),
            });
        }
    }
    task.normalize();
    task.updated_at = Utc::now();

    // Phase 3: single commit — file then index
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    output::print_task(&task, format)?;
    Ok(())
}

pub fn undepend(repo_root: &Path, id: u64, on: Vec<u64>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.depends_on.retain(|d| !on.contains(&d.id));
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
