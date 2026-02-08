use crate::error::Result;
use crate::model::{Contract, Kind, Planning};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use std::path::Path;

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
    let task = repo.store.create(
        title,
        kind,
        description,
        parent,
        depends_on,
        tags,
        contract,
        planning,
    )?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}
