use std::path::Path;
use crate::error::Result;
use crate::store::files::FileStore;

pub fn run(repo_root: &Path) -> Result<()> {
    FileStore::init(repo_root)?;
    eprintln!("Initialized .tak/ in {}", repo_root.display());
    Ok(())
}
