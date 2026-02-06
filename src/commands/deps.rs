use std::path::Path;
use chrono::Utc;
use crate::error::{Result, TakError};
use crate::output;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn depend(repo_root: &Path, id: u64, on: Vec<u64>, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;
    let mut task = store.read(id)?;

    for dep_id in &on {
        store.read(*dep_id)?; // validate exists
        if idx.would_cycle(id, *dep_id)? {
            return Err(TakError::CycleDetected(id));
        }
        if !task.depends_on.contains(dep_id) {
            task.depends_on.push(*dep_id);
        }
    }

    task.updated_at = Utc::now();
    store.write(&task)?;
    idx.upsert(&task)?;
    output::print_task(&task, pretty);
    Ok(())
}

pub fn undepend(repo_root: &Path, id: u64, on: Vec<u64>, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;
    let mut task = store.read(id)?;

    task.depends_on.retain(|d| !on.contains(d));
    task.updated_at = Utc::now();
    store.write(&task)?;
    idx.upsert(&task)?;
    output::print_task(&task, pretty);
    Ok(())
}

pub fn reparent(repo_root: &Path, id: u64, to: u64, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;
    store.read(to)?; // validate parent exists
    let mut task = store.read(id)?;

    task.parent = Some(to);
    task.updated_at = Utc::now();
    store.write(&task)?;
    idx.upsert(&task)?;
    output::print_task(&task, pretty);
    Ok(())
}

pub fn orphan(repo_root: &Path, id: u64, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;
    let mut task = store.read(id)?;

    task.parent = None;
    task.updated_at = Utc::now();
    store.write(&task)?;
    idx.upsert(&task)?;
    output::print_task(&task, pretty);
    Ok(())
}
