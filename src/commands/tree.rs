use std::collections::HashSet;
use std::path::Path;
use serde::Serialize;
use crate::error::Result;
use crate::output::Format;
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

fn truncate_title(title: &str, max_len: usize) -> String {
    if title.chars().count() > max_len {
        let truncated: String = title.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    } else {
        title.to_string()
    }
}

fn print_tree_minimal(node: &TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let title = truncate_title(&node.title, 12);
    let blocked_marker = if node.blocked { " [B]" } else { "" };
    println!("{}{:>4} {:12} {:6} {:10}{}", indent, node.id, title, node.kind, node.status, blocked_marker);
    for child in &node.children {
        print_tree_minimal(child, depth + 1);
    }
}

pub fn run(repo_root: &Path, id: Option<u64>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let blocked_ids: HashSet<u64> = repo.index.blocked()?.into_iter().collect();

    if let Some(root_id) = id {
        let tree = build_tree(root_id, &repo.store, &repo.index, &blocked_ids)?;
        match format {
            Format::Json => println!("{}", serde_json::to_string(&tree)?),
            Format::Pretty => print_tree_pretty(&tree, "", true, true),
            Format::Minimal => print_tree_minimal(&tree, 0),
        }
    } else {
        let root_ids = repo.index.roots()?;
        let trees: Vec<TreeNode> = root_ids
            .into_iter()
            .map(|rid| build_tree(rid, &repo.store, &repo.index, &blocked_ids))
            .collect::<Result<Vec<_>>>()?;

        match format {
            Format::Json => println!("{}", serde_json::to_string(&trees)?),
            Format::Pretty => {
                for tree in &trees {
                    print_tree_pretty(tree, "", true, true);
                    println!();
                }
            }
            Format::Minimal => {
                for tree in &trees {
                    print_tree_minimal(&tree, 0);
                }
            }
        }
    }

    Ok(())
}
