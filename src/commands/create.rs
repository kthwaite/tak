use std::path::Path;
use crate::error::Result;
use crate::model::Kind;
use crate::output;
use crate::store::repo::Repo;

pub fn run(
    repo_root: &Path,
    title: String,
    kind_str: &str,
    description: Option<String>,
    parent: Option<u64>,
    depends_on: Vec<u64>,
    tags: Vec<String>,
    pretty: bool,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let kind = match kind_str {
        "epic" => Kind::Epic,
        "task" => Kind::Task,
        "bug" => Kind::Bug,
        other => {
            eprintln!("unknown kind: {other}, using 'task'");
            Kind::Task
        }
    };
    let task = repo.store.create(title, kind, description, parent, depends_on, tags)?;
    repo.index.upsert(&task)?;
    output::print_task(&task, pretty);
    Ok(())
}
