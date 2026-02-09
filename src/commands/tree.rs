use colored::Colorize;

use crate::error::Result;
use crate::model::Task;
use crate::output::{Format, truncate_title};
use crate::store::repo::Repo;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

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
    children_map: &HashMap<Option<u64>, Vec<u64>>,
    tasks: &HashMap<u64, Task>,
    blocked_set: &HashSet<u64>,
) -> Option<TreeNode> {
    let task = tasks.get(&id)?;
    let child_ids = children_map.get(&Some(id)).cloned().unwrap_or_default();
    let children = child_ids
        .into_iter()
        .filter_map(|cid| build_tree(cid, children_map, tasks, blocked_set))
        .collect();

    Some(TreeNode {
        id: task.id,
        title: task.title.clone(),
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

    let blocked_marker = if node.blocked {
        format!(" {}", "[BLOCKED]".red().bold())
    } else {
        String::new()
    };
    let status_colored = match node.status.as_str() {
        "pending" => node.status.yellow().to_string(),
        "in_progress" => node.status.blue().to_string(),
        "done" => node.status.green().to_string(),
        "cancelled" => node.status.red().to_string(),
        other => other.to_string(),
    };
    println!(
        "{}{}{} {} ({}, {}){}",
        prefix,
        connector.dimmed(),
        format!("[{}]", node.id).cyan().bold(),
        node.title.bold(),
        node.kind,
        status_colored,
        blocked_marker,
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

fn print_tree_minimal(node: &TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let title = truncate_title(&node.title, 12);
    let blocked_marker = if node.blocked { " [B]" } else { "" };
    println!(
        "{}{:>4} {:12} {:6} {:10}{}",
        indent, node.id, title, node.kind, node.status, blocked_marker
    );
    for child in &node.children {
        print_tree_minimal(child, depth + 1);
    }
}

pub fn run(repo_root: &Path, id: Option<u64>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let blocked_ids: HashSet<u64> = repo.index.blocked()?.iter().map(u64::from).collect();

    // Pre-load all tasks into memory (one pass over files)
    let all_tasks = repo.store.list_all()?;
    let tasks: HashMap<u64, Task> = all_tasks.into_iter().map(|t| (t.id, t)).collect();

    // Build parent→children index in memory
    let mut children_map: HashMap<Option<u64>, Vec<u64>> = HashMap::new();
    for task in tasks.values() {
        children_map.entry(task.parent).or_default().push(task.id);
    }
    // Sort children by ID for deterministic output
    for children in children_map.values_mut() {
        children.sort();
    }

    if let Some(root_id) = id {
        if let Some(tree) = build_tree(root_id, &children_map, &tasks, &blocked_ids) {
            match format {
                Format::Json => println!("{}", serde_json::to_string(&tree)?),
                Format::Pretty => print_tree_pretty(&tree, "", true, true),
                Format::Minimal => print_tree_minimal(&tree, 0),
            }
        }
    } else {
        let root_ids = children_map.get(&None).cloned().unwrap_or_default();
        let trees: Vec<TreeNode> = root_ids
            .into_iter()
            .filter_map(|rid| build_tree(rid, &children_map, &tasks, &blocked_ids))
            .collect();

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
                    print_tree_minimal(tree, 0);
                }
            }
        }
    }

    Ok(())
}
