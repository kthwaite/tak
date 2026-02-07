use std::path::Path;
use chrono::Utc;
use crate::error::Result;
use crate::model::Kind;
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn run(
    repo_root: &Path,
    id: u64,
    title: Option<String>,
    description: Option<String>,
    kind: Option<Kind>,
    tags: Option<Vec<String>>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    if let Some(t) = title {
        task.title = t;
    }
    if let Some(d) = description {
        if d.is_empty() {
            task.description = None;
        } else {
            task.description = Some(d);
        }
    }
    if let Some(k) = kind {
        task.kind = k;
    }
    if let Some(t) = tags {
        task.tags = t;
    }

    task.normalize();
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    output::print_task(&task, format)?;
    Ok(())
}
