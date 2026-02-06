use std::path::Path;
use crate::error::Result;
use crate::output;
use crate::store::files::FileStore;

pub fn run(repo_root: &Path, id: u64, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let task = store.read(id)?;
    output::print_task(&task, pretty);
    Ok(())
}
