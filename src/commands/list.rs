use crate::error::Result;
use crate::model::{Kind, Priority, Status};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn run(
    repo_root: &Path,
    status: Option<Status>,
    kind: Option<Kind>,
    tag: Option<String>,
    assignee: Option<String>,
    available: bool,
    blocked: bool,
    children_of: Option<u64>,
    priority: Option<Priority>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    let mut tasks = if available {
        let ids: Vec<u64> = repo.index.available(None)?.iter().map(u64::from).collect();
        repo.store.read_many(&ids)?
    } else if blocked {
        let ids: Vec<u64> = repo.index.blocked()?.iter().map(u64::from).collect();
        repo.store.read_many(&ids)?
    } else if let Some(parent_id) = children_of {
        let ids: Vec<u64> = repo
            .index
            .children_of(parent_id)?
            .iter()
            .map(u64::from)
            .collect();
        repo.store.read_many(&ids)?
    } else {
        repo.store.list_all()?
    };

    if let Some(s) = status {
        tasks.retain(|t| t.status == s);
    }
    if let Some(k) = kind {
        tasks.retain(|t| t.kind == k);
    }
    if let Some(ref tg) = tag {
        tasks.retain(|t| t.tags.contains(tg));
    }
    if let Some(ref a) = assignee {
        tasks.retain(|t| t.assignee.as_deref() == Some(a.as_str()));
    }
    if let Some(p) = priority {
        tasks.retain(|t| t.planning.priority == Some(p));
    }

    output::print_tasks(&tasks, format)?;
    Ok(())
}
