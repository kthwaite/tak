use std::path::Path;

use chrono::Utc;

use crate::error::{Result, TakError};
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn run(repo_root: &Path, id: u64, force: bool, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(id)?;

    let children = repo.index.children_of(id)?;
    let dependents = repo.index.dependents_of(id)?;

    if !force && (!children.is_empty() || !dependents.is_empty()) {
        return Err(TakError::TaskInUse(id));
    }

    // --force: cascade — orphan children, remove incoming deps
    for child_id in &children {
        let child_id: u64 = child_id.into();
        let mut child = repo.store.read(child_id)?;
        child.parent = None;
        child.updated_at = Utc::now();
        repo.store.write(&child)?;
        repo.index.upsert(&child)?;
    }
    for dep_id in &dependents {
        let dep_id: u64 = dep_id.into();
        let mut dep_task = repo.store.read(dep_id)?;
        dep_task.depends_on.retain(|d| d.id != id);
        dep_task.updated_at = Utc::now();
        repo.store.write(&dep_task)?;
        repo.index.upsert(&dep_task)?;
    }

    // Index first, then file (self-healing if file delete fails)
    repo.index.remove(id)?;
    repo.store.delete(id)?;

    // Best-effort sidecar cleanup
    let _ = repo.sidecars.delete(id);

    output::print_task(&task, format)?;
    Ok(())
}
