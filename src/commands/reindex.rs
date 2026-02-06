use std::path::Path;
use crate::error::Result;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn run(repo_root: &Path) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let tasks = store.list_all()?;
    let idx = Index::open(&store.root().join("index.db"))?;
    idx.rebuild(&tasks)?;
    eprintln!("Reindexed {} tasks", tasks.len());
    Ok(())
}
