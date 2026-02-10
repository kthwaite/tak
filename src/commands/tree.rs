use colored::Colorize;

use crate::error::Result;
use crate::model::{Estimate, Status, Task};
use crate::output::{Format, truncate_title};
use crate::store::repo::Repo;
use crate::task_id::TaskId;
use clap::ValueEnum;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MINIMAL_ID_WIDTH: usize = TaskId::HEX_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum TreeSort {
    Id,
    Created,
    Priority,
    Estimate,
}

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

fn estimate_rank(estimate: Option<Estimate>) -> u8 {
    match estimate {
        Some(Estimate::Xs) => 0,
        Some(Estimate::S) => 1,
        Some(Estimate::M) => 2,
        Some(Estimate::L) => 3,
        Some(Estimate::Xl) => 4,
        None => 5,
    }
}

fn compare_task_ids(left: u64, right: u64, tasks: &HashMap<u64, Task>, sort: TreeSort) -> Ordering {
    let left_task = tasks.get(&left);
    let right_task = tasks.get(&right);

    let ordering = match (left_task, right_task, sort) {
        (Some(_), Some(_), TreeSort::Id) => Ordering::Equal,
        (Some(a), Some(b), TreeSort::Created) => a.created_at.cmp(&b.created_at),
        (Some(a), Some(b), TreeSort::Priority) => {
            let a_rank = a
                .planning
                .priority
                .map(|priority| priority.rank())
                .unwrap_or(4);
            let b_rank = b
                .planning
                .priority
                .map(|priority| priority.rank())
                .unwrap_or(4);
            a_rank.cmp(&b_rank)
        }
        (Some(a), Some(b), TreeSort::Estimate) => {
            let a_rank = estimate_rank(a.planning.estimate);
            let b_rank = estimate_rank(b.planning.estimate);
            a_rank.cmp(&b_rank)
        }
        _ => Ordering::Equal,
    };

    ordering.then_with(|| left.cmp(&right))
}

fn build_children_map(
    tasks: &HashMap<u64, Task>,
    pending_only: bool,
    sort: TreeSort,
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

    // Sort roots/siblings according to requested key, with task ID as tiebreaker.
    for children in children_map.values_mut() {
        children.sort_by(|left, right| compare_task_ids(*left, *right, tasks, sort));
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

pub fn run(
    repo_root: &Path,
    id: Option<u64>,
    pending_only: bool,
    sort: TreeSort,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let blocked_ids: HashSet<u64> = repo.index.blocked()?.iter().map(u64::from).collect();

    // Pre-load all tasks into memory (one pass over files)
    let all_tasks = repo.store.list_all()?;
    let tasks = collect_tasks(all_tasks, pending_only);

    // Build parent→children index in memory
    let children_map = build_children_map(&tasks, pending_only, sort);

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
    use crate::model::{Estimate, Kind, Priority};
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
        let children_map = build_children_map(&filtered, true, TreeSort::Id);
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
        let children_map = build_children_map(&filtered, true, TreeSort::Id);

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
        let children_map = build_children_map(&filtered, true, TreeSort::Id);

        assert!(build_tree(1, &children_map, &filtered, &HashSet::new()).is_none());
    }

    #[test]
    fn build_tree_uses_hex_task_ids() {
        let tasks = vec![task(42, Status::Pending, None)];

        let filtered = collect_tasks(tasks, false);
        let children_map = build_children_map(&filtered, false, TreeSort::Id);
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
        let children_map = build_children_map(&unfiltered, false, TreeSort::Id);
        let roots = children_map.get(&None).cloned().unwrap_or_default();

        assert_eq!(roots, vec![1]);
    }

    #[test]
    fn tree_sort_priority_orders_highest_priority_first() {
        let mut low = task(1, Status::Pending, None);
        low.planning.priority = Some(Priority::Low);

        let mut high = task(2, Status::Pending, None);
        high.planning.priority = Some(Priority::High);

        let none = task(3, Status::Pending, None);

        let tasks = collect_tasks(vec![low, high, none], false);
        let children_map = build_children_map(&tasks, false, TreeSort::Priority);

        let roots = children_map.get(&None).cloned().unwrap_or_default();
        assert_eq!(roots, vec![2, 1, 3]);
    }

    #[test]
    fn tree_sort_estimate_orders_smallest_first() {
        let mut large = task(1, Status::Pending, None);
        large.planning.estimate = Some(Estimate::L);

        let mut small = task(2, Status::Pending, None);
        small.planning.estimate = Some(Estimate::Xs);

        let none = task(3, Status::Pending, None);

        let tasks = collect_tasks(vec![large, small, none], false);
        let children_map = build_children_map(&tasks, false, TreeSort::Estimate);

        let roots = children_map.get(&None).cloned().unwrap_or_default();
        assert_eq!(roots, vec![2, 1, 3]);
    }
}
