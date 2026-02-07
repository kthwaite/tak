use crate::error::Result;
use crate::store::repo::Repo;
use std::path::Path;

pub fn run(repo_root: &Path) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let tasks = repo.store.list_all()?;
    repo.index.rebuild(&tasks)?;
    eprintln!("Reindexed {} tasks", tasks.len());
    Ok(())
}
