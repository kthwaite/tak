use crate::error::Result;
use crate::model::Kind;
use crate::output::{self, Format};
use crate::store::repo::Repo;
use chrono::Utc;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn run(
    repo_root: &Path,
    id: u64,
    title: Option<String>,
    description: Option<String>,
    kind: Option<Kind>,
    tags: Option<Vec<String>>,
    objective: Option<String>,
    verify: Option<Vec<String>>,
    constraint: Option<Vec<String>>,
    criterion: Option<Vec<String>>,
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

    // Contract fields — each is individually optional
    if let Some(o) = objective {
        if o.is_empty() {
            task.contract.objective = None;
        } else {
            task.contract.objective = Some(o);
        }
    }
    if let Some(v) = verify {
        task.contract.verification = v;
    }
    if let Some(c) = constraint {
        task.contract.constraints = c;
    }
    if let Some(c) = criterion {
        task.contract.acceptance_criteria = c;
    }

    task.normalize();
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    output::print_task(&task, format)?;
    Ok(())
}
