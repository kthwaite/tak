use std::path::Path;
use crate::error::Result;
use crate::output::{self, Format};
use crate::store::repo::Repo;

pub fn run(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let available = repo.index.available_for(assignee.as_deref())?;

    if let Some(&id) = available.first() {
        let task = repo.store.read(id)?;
        output::print_task(&task, format);
    } else if format != Format::Json {
        println!("No available tasks");
    } else {
        println!("null");
    }
    Ok(())
}
