use std::path::Path;
use crate::error::Result;
use crate::output;
use crate::store::repo::Repo;

pub fn run(repo_root: &Path, _assignee: Option<String>, pretty: bool) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let available = repo.index.available()?;

    if let Some(&id) = available.first() {
        let task = repo.store.read(id)?;
        output::print_task(&task, pretty);
    } else if pretty {
        println!("No available tasks");
    } else {
        println!("null");
    }
    Ok(())
}
