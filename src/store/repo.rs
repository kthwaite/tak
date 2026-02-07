use std::path::{Path, PathBuf};

use crate::error::{Result, TakError};
use crate::store::files::FileStore;
use crate::store::index::Index;

pub struct Repo {
    pub store: FileStore,
    pub index: Index,
}

impl Repo {
    /// Open an existing .tak repository, auto-rebuilding the index if stale or missing.
    pub fn open(repo_root: &Path) -> Result<Self> {
        let store = FileStore::open(repo_root)?;
        let index_path = store.root().join("index.db");
        let needs_rebuild = !index_path.exists();
        let index = Index::open(&index_path)?;

        let current_fp = store.fingerprint()?;

        let needs_rebuild = if needs_rebuild {
            true
        } else {
            let stored_fp = index.get_fingerprint()?;
            stored_fp.as_deref() != Some(current_fp.as_str())
        };

        if needs_rebuild {
            let tasks = store.list_all()?;
            index.rebuild(&tasks)?;
        }

        index.set_fingerprint(&current_fp)?;
        Ok(Self { store, index })
    }
}

/// Walk up from current directory to find the .tak root.
pub fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().map_err(TakError::Io)?;
    loop {
        if dir.join(".tak").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(TakError::NotInitialized);
        }
    }
}
