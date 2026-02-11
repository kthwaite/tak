use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::store::coordination_db::CoordinationDb;
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
    fs::create_dir_all(tak.join("therapist"))?;
    fs::write(tak.join("therapist").join("observations.jsonl"), "")?;

    // Create coordination database (creates runtime/ dir and coordination.db)
    let runtime_dir = tak.join("runtime");
    fs::create_dir_all(&runtime_dir)?;
    CoordinationDb::open(&runtime_dir.join("coordination.db"))?;

    // Create work-loop state directory
    fs::create_dir_all(runtime_dir.join("work").join("states"))?;

    // Write .gitignore for derived/ephemeral data
    fs::write(
        tak.join(".gitignore"),
        "index.db\n*.lock\nartifacts/\nverification_results/\nruntime/\n",
    )?;

    eprintln!("Initialized .tak/ in {}", repo_root.display());
    Ok(())
}
