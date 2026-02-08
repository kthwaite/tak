use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn run(repo_root: &Path) -> Result<()> {
    let store = FileStore::init(repo_root)?;
    Index::open(&store.root().join("index.db"))?;

    // Create sidecar and learnings directories
    let tak = store.root();
    fs::create_dir_all(tak.join("context"))?;
    fs::create_dir_all(tak.join("history"))?;
    fs::create_dir_all(tak.join("artifacts"))?;
    fs::create_dir_all(tak.join("verification_results"))?;
    fs::create_dir_all(tak.join("learnings"))?;

    // Write .gitignore for derived/ephemeral data
    fs::write(
        tak.join(".gitignore"),
        "index.db\n*.lock\nartifacts/\nverification_results/\n",
    )?;

    eprintln!("Initialized .tak/ in {}", repo_root.display());
    Ok(())
}
