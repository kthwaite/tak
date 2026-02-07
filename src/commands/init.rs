use crate::error::Result;
use crate::store::files::FileStore;
use crate::store::index::Index;
use std::path::Path;

pub fn run(repo_root: &Path) -> Result<()> {
    let store = FileStore::init(repo_root)?;
    Index::open(&store.root().join("index.db"))?;
    eprintln!("Initialized .tak/ in {}", repo_root.display());
    Ok(())
}
