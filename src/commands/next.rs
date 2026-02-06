use std::path::Path;
use crate::error::Result;
use crate::output;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn run(repo_root: &Path, _assignee: Option<String>, pretty: bool) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;
    let available = idx.available()?;

    if let Some(&id) = available.first() {
        let task = store.read(id)?;
        output::print_task(&task, pretty);
    } else if pretty {
        println!("No available tasks");
    } else {
        println!("null");
    }
    Ok(())
}
