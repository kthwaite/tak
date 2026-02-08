use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn run(repo_root: &Path) -> Result<()> {
    let store = FileStore::init(repo_root)?;
    Index::open(&store.root().join("index.db"))?;

    // Create sidecar directories
    fs::create_dir_all(store.root().join("context"))?;
    fs::create_dir_all(store.root().join("history"))?;

    eprintln!("Initialized .tak/ in {}", repo_root.display());
    Ok(())
}
