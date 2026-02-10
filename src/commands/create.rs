use crate::error::Result;
use crate::model::{Contract, Kind, Planning, Task};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use std::path::Path;

fn merge_traceability_from_task(
    task: &Task,
    origin_idea_id: &mut Option<u64>,
    refinement_task_ids: &mut Vec<u64>,
) {
    match task.kind {
        Kind::Idea => {
            if origin_idea_id.is_none() {
                *origin_idea_id = Some(task.id);
            }
        }
        Kind::Meta => {
            refinement_task_ids.push(task.id);
            if origin_idea_id.is_none() {
                *origin_idea_id = task.origin_idea_id();
            }
        }
        _ => {
            if origin_idea_id.is_none() {
                *origin_idea_id = task.origin_idea_id();
            }
            refinement_task_ids.extend(task.refinement_task_ids());
        }
    }
}

fn derive_traceability(
    repo: &Repo,
    kind: Kind,
    parent: Option<u64>,
    depends_on: &[u64],
) -> Result<(Option<u64>, Vec<u64>)> {
    if matches!(kind, Kind::Idea) {
        return Ok((None, vec![]));
    }

    let mut origin_idea_id = None;
    let mut refinement_task_ids = Vec::new();

    if let Some(parent_id) = parent {
        let parent_task = repo.store.read(parent_id)?;
        merge_traceability_from_task(&parent_task, &mut origin_idea_id, &mut refinement_task_ids);
    }

    for dep_id in depends_on {
        let dep_task = repo.store.read(*dep_id)?;
        merge_traceability_from_task(&dep_task, &mut origin_idea_id, &mut refinement_task_ids);
    }

    refinement_task_ids.sort_unstable();
    refinement_task_ids.dedup();

    Ok((origin_idea_id, refinement_task_ids))
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    repo_root: &Path,
    title: String,
    kind: Kind,
    description: Option<String>,
    parent: Option<u64>,
    depends_on: Vec<u64>,
    tags: Vec<String>,
    contract: Contract,
    planning: Planning,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let (origin_idea_id, refinement_task_ids) =
        derive_traceability(&repo, kind, parent, &depends_on)?;

    let mut task = repo.store.create(
        title,
        kind,
        description,
        parent,
        depends_on,
        tags,
        contract,
        planning,
    )?;

    let mut traceability_changed = false;
    if origin_idea_id.is_some() {
        task.set_origin_idea_id(origin_idea_id);
        traceability_changed = true;
    }
    if !refinement_task_ids.is_empty() {
        task.set_refinement_task_ids(refinement_task_ids);
        traceability_changed = true;
    }

    if traceability_changed {
        task.normalize();
        repo.store.write(&task)?;
    }

    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}
