use std::path::Path;
use chrono::Utc;
use crate::error::Result;
use crate::model::Kind;
use crate::output;
use crate::store::repo::Repo;

pub fn run(
    repo_root: &Path,
    id: u64,
    title: Option<String>,
    description: Option<String>,
    kind: Option<String>,
    tags: Option<Vec<String>>,
    pretty: bool,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    if let Some(t) = title {
        task.title = t;
    }
    if let Some(d) = description {
        task.description = Some(d);
    }
    if let Some(k) = kind {
        task.kind = match k.as_str() {
            "epic" => Kind::Epic,
            "task" => Kind::Task,
            "bug" => Kind::Bug,
            other => {
                eprintln!("unknown kind: {other}, keeping current");
                task.kind
            }
        };
    }
    if let Some(t) = tags {
        task.tags = t;
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    output::print_task(&task, pretty);
    Ok(())
}
