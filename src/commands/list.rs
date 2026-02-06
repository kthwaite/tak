use std::path::Path;
use crate::error::Result;
use crate::output;
use crate::store::files::FileStore;
use crate::store::index::Index;

pub fn run(
    repo_root: &Path,
    status: Option<String>,
    kind: Option<String>,
    tag: Option<String>,
    assignee: Option<String>,
    available: bool,
    blocked: bool,
    children_of: Option<u64>,
    pretty: bool,
) -> Result<()> {
    let store = FileStore::open(repo_root)?;
    let idx = Index::open(&store.root().join("index.db"))?;

    let tasks = if available {
        let ids = idx.available()?;
        ids.into_iter().map(|id| store.read(id)).collect::<Result<Vec<_>>>()?
    } else if blocked {
        let ids = idx.blocked()?;
        ids.into_iter().map(|id| store.read(id)).collect::<Result<Vec<_>>>()?
    } else if let Some(parent_id) = children_of {
        let ids = idx.children_of(parent_id)?;
        ids.into_iter().map(|id| store.read(id)).collect::<Result<Vec<_>>>()?
    } else {
        let mut all = store.list_all()?;

        if let Some(ref s) = status {
            all.retain(|t| t.status.to_string() == *s);
        }
        if let Some(ref k) = kind {
            all.retain(|t| t.kind.to_string() == *k);
        }
        if let Some(ref tg) = tag {
            all.retain(|t| t.tags.contains(tg));
        }
        if let Some(ref a) = assignee {
            all.retain(|t| t.assignee.as_deref() == Some(a.as_str()));
        }
        all
    };

    output::print_tasks(&tasks, pretty);
    Ok(())
}
