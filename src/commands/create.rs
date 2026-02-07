use std::path::Path;
use crate::error::Result;
use crate::model::Kind;
use crate::output::{self, Format};
use crate::store::repo::Repo;

#[allow(clippy::too_many_arguments)]
pub fn run(
    repo_root: &Path,
    title: String,
    kind: Kind,
    description: Option<String>,
    parent: Option<u64>,
    depends_on: Vec<u64>,
    tags: Vec<String>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.create(title, kind, description, parent, depends_on, tags)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}
