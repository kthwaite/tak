use std::path::Path;
use crate::error::Result;
use crate::model::Kind;
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn run(
    repo_root: &Path,
    title: String,
    kind_str: &str,
    description: Option<String>,
    parent: Option<u64>,
    depends_on: Vec<u64>,
    tags: Vec<String>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let kind: Kind = kind_str.parse()?;
    let task = repo.store.create(title, kind, description, parent, depends_on, tags)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, format)?;
    Ok(())
}
