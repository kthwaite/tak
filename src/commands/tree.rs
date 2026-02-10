use colored::Colorize;

use crate::error::Result;
use crate::model::{Status, Task};
use crate::output::{Format, truncate_title};
use crate::store::repo::Repo;
use crate::task_id::TaskId;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MINIMAL_ID_WIDTH: usize = TaskId::HEX_LEN;

#[derive(Debug, Serialize)]
struct TreeNode {
    id: String,
    title: String,
    kind: String,
    status: String,
    blocked: bool,
    children: Vec<TreeNode>,
}

fn collect_tasks(all_tasks: Vec<Task>, pending_only: bool) -> HashMap<u64, Task> {
    all_tasks
        .into_iter()
        .filter(|task| !pending_only || task.status == Status::Pending)
        .map(|task| (task.id, task))
        .collect()
}

fn build_children_map(
    tasks: &HashMap<u64, Task>,
    pending_only: bool,
) -> HashMap<Option<u64>, Vec<u64>> {
    let mut children_map: HashMap<Option<u64>, Vec<u64>> = HashMap::new();
    for task in tasks.values() {
        let parent = if pending_only {
            task.parent.filter(|pid| tasks.contains_key(pid))
        } else {
            task.parent
        };
        children_map.entry(parent).or_default().push(task.id);
    }

    // Sort children by ID for deterministic output
    for children in children_map.values_mut() {
        children.sort();
    }

    children_map
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
        id: format_tree_id(task.id),
        title: task.title.clone(),
        kind: task.kind.to_string(),
        status: task.status.to_string(),
        blocked: blocked_set.contains(&task.id),
        children,
    })
}

fn format_tree_id(id: u64) -> String {
    TaskId::from(id).to_string()
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
        "{}{:>id_width$} {:12} {:6} {:10}{}",
        indent,
        node.id,
        title,
        node.kind,
        node.status,
        blocked_marker,
        id_width = MINIMAL_ID_WIDTH,
    );
    for child in &node.children {
        print_tree_minimal(child, depth + 1);
    }
}

pub fn run(repo_root: &Path, id: Option<u64>, pending_only: bool, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let blocked_ids: HashSet<u64> = repo.index.blocked()?.iter().map(u64::from).collect();

    // Pre-load all tasks into memory (one pass over files)
    let all_tasks = repo.store.list_all()?;
    let tasks = collect_tasks(all_tasks, pending_only);

    // Build parent→children index in memory
    let children_map = build_children_map(&tasks, pending_only);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use chrono::Utc;

    fn task(id: u64, status: Status, parent: Option<u64>) -> Task {
        let now = Utc::now();
        Task {
            id,
            title: format!("task-{id}"),
            description: None,
            status,
            kind: Kind::Task,
            parent,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: crate::model::Contract::default(),
            planning: crate::model::Planning::default(),
            git: crate::model::GitInfo::default(),
            execution: crate::model::Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        }
    }

    #[test]
    fn pending_filter_promotes_children_of_non_pending_parents_to_roots() {
        let tasks = vec![
            task(1, Status::Done, None),
            task(2, Status::Pending, Some(1)),
            task(3, Status::Pending, Some(2)),
            task(4, Status::Pending, None),
        ];

        let filtered = collect_tasks(tasks, true);
        let children_map = build_children_map(&filtered, true);
        let roots = children_map.get(&None).cloned().unwrap_or_default();
        assert_eq!(roots, vec![2, 4]);

        let tree = build_tree(2, &children_map, &filtered, &HashSet::new()).unwrap();
        let child_ids: Vec<String> = tree.children.iter().map(|child| child.id.clone()).collect();
        assert_eq!(child_ids, vec![format_tree_id(3)]);
    }

    #[test]
    fn rooted_pending_tree_omits_non_pending_children() {
        let tasks = vec![
            task(10, Status::Pending, None),
            task(11, Status::Done, Some(10)),
            task(12, Status::Pending, Some(10)),
        ];

        let filtered = collect_tasks(tasks, true);
        let children_map = build_children_map(&filtered, true);

        let tree = build_tree(10, &children_map, &filtered, &HashSet::new()).unwrap();
        let child_ids: Vec<String> = tree.children.iter().map(|child| child.id.clone()).collect();
        assert_eq!(child_ids, vec![format_tree_id(12)]);
    }

    #[test]
    fn rooted_pending_tree_for_non_pending_root_is_empty() {
        let tasks = vec![
            task(1, Status::Done, None),
            task(2, Status::Pending, Some(1)),
        ];

        let filtered = collect_tasks(tasks, true);
        let children_map = build_children_map(&filtered, true);

        assert!(build_tree(1, &children_map, &filtered, &HashSet::new()).is_none());
    }

    #[test]
    fn build_tree_uses_hex_task_ids() {
        let tasks = vec![task(42, Status::Pending, None)];

        let filtered = collect_tasks(tasks, false);
        let children_map = build_children_map(&filtered, false);
        let tree = build_tree(42, &children_map, &filtered, &HashSet::new()).unwrap();

        assert_eq!(tree.id, format_tree_id(42));
    }

    #[test]
    fn non_pending_tree_keeps_original_parent_links() {
        let tasks = vec![
            task(1, Status::Done, None),
            task(2, Status::Pending, Some(1)),
        ];

        let unfiltered = collect_tasks(tasks, false);
        let children_map = build_children_map(&unfiltered, false);
        let roots = children_map.get(&None).cloned().unwrap_or_default();

        assert_eq!(roots, vec![1]);
    }
}
