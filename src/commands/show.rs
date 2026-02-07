use std::path::Path;
use crate::error::Result;
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(id)?;
    output::print_task(&task, format)?;
    Ok(())
}
