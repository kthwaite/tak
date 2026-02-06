use std::collections::HashSet;
use std::path::Path;
use serde::Serialize;
use crate::error::Result;
use crate::store::files::FileStore;
use crate::store::index::Index;
use crate::store::repo::Repo;

#[derive(Serialize)]
struct TreeNode {
    id: u64,
    title: String,
    kind: String,
    status: String,
    blocked: bool,
    children: Vec<TreeNode>,
}

fn build_tree(
    id: u64,
    store: &FileStore,
    idx: &Index,
    blocked_set: &HashSet<u64>,
) -> Result<TreeNode> {
    let task = store.read(id)?;
    let child_ids = idx.children_of(id)?;
    let children = child_ids
        .into_iter()
        .map(|cid| build_tree(cid, store, idx, blocked_set))
        .collect::<Result<Vec<_>>>()?;

    Ok(TreeNode {
        id: task.id,
        title: task.title,
        kind: task.kind.to_string(),
        status: task.status.to_string(),
        blocked: blocked_set.contains(&task.id),
        children,
    })
}

fn print_tree_pretty(node: &TreeNode, prefix: &str, is_last: bool, is_root: bool) {
    let connector = if is_root {
        ""
    } else if is_last {
        "\u{2514}\u{2500}\u{2500} "
    } else {
        "\u{251c}\u{2500}\u{2500} "
    };

    let blocked_marker = if node.blocked { " [BLOCKED]" } else { "" };
    println!(
        "{}{}[{}] {} ({}, {}){}",
        prefix, connector,
        node.id, node.title, node.kind, node.status, blocked_marker
    );

    let child_prefix = if is_root {
        prefix.to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}\u{2502}   ", prefix)
    };

    for (i, child) in node.children.iter().enumerate() {
        let last = i == node.children.len() - 1;
        print_tree_pretty(child, &child_prefix, last, false);
    }
}

pub fn run(repo_root: &Path, id: Option<u64>, pretty: bool) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let blocked_ids: HashSet<u64> = repo.index.blocked()?.into_iter().collect();

    if let Some(root_id) = id {
        let tree = build_tree(root_id, &repo.store, &repo.index, &blocked_ids)?;
        if pretty {
            print_tree_pretty(&tree, "", true, true);
        } else {
            println!("{}", serde_json::to_string(&tree).unwrap());
        }
    } else {
        let root_ids = repo.index.roots()?;
        let trees: Vec<TreeNode> = root_ids
            .into_iter()
            .map(|rid| build_tree(rid, &repo.store, &repo.index, &blocked_ids))
            .collect::<Result<Vec<_>>>()?;

        if pretty {
            for tree in &trees {
                print_tree_pretty(tree, "", true, true);
                println!();
            }
        } else {
            println!("{}", serde_json::to_string(&trees).unwrap());
        }
    }

    Ok(())
}
