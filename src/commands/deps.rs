use crate::error::{Result, TakError};
use crate::model::{DepType, Dependency, Task};
use crate::output::{self, Format};
use crate::store::repo::Repo;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn normalize_ids(mut ids: Vec<u64>) -> Vec<u64> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn build_dependency_adjacency(tasks: &[Task]) -> HashMap<u64, HashSet<u64>> {
    let mut adjacency = HashMap::new();

    for task in tasks {
        adjacency.entry(task.id).or_insert_with(HashSet::new);
        for dep in &task.depends_on {
            adjacency
                .entry(task.id)
                .or_insert_with(HashSet::new)
                .insert(dep.id);
            adjacency.entry(dep.id).or_insert_with(HashSet::new);
        }
    }

    adjacency
}

fn has_path(
    adjacency: &HashMap<u64, HashSet<u64>>,
    start: u64,
    target: u64,
    visited: &mut HashSet<u64>,
) -> bool {
    if start == target {
        return true;
    }

    if !visited.insert(start) {
        return false;
    }

    adjacency.get(&start).is_some_and(|deps| {
        deps.iter()
            .copied()
            .any(|next| has_path(adjacency, next, target, visited))
    })
}

fn validate_dependency_plan(
    target_ids: &[u64],
    dep_ids: &[u64],
    adjacency: &mut HashMap<u64, HashSet<u64>>,
) -> Result<()> {
    for &target_id in target_ids {
        for &dep_id in dep_ids {
            if target_id == dep_id {
                return Err(TakError::CycleDetected(target_id));
            }

            let already_present = adjacency
                .get(&target_id)
                .is_some_and(|existing| existing.contains(&dep_id));
            if already_present {
                continue;
            }

            let mut visited = HashSet::new();
            if has_path(adjacency, dep_id, target_id, &mut visited) {
                return Err(TakError::CycleDetected(target_id));
            }

            adjacency
                .entry(target_id)
                .or_insert_with(HashSet::new)
                .insert(dep_id);
        }
    }

    Ok(())
}

fn print_updated_tasks(tasks: &[Task], format: Format, quiet: bool) -> Result<()> {
    if quiet {
        return Ok(());
    }

    if tasks.len() == 1 {
        output::print_task(&tasks[0], format)
    } else {
        output::print_tasks(tasks, format)
    }
}

pub fn depend(
    repo_root: &Path,
    ids: Vec<u64>,
    on: Vec<u64>,
    dep_type: Option<DepType>,
    reason: Option<String>,
    format: Format,
    quiet: bool,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let target_ids = normalize_ids(ids);
    let dep_ids = normalize_ids(on);

    // Validate all referenced tasks exist before any mutation.
    for &id in &target_ids {
        repo.store.read(id)?;
    }
    for &dep_id in &dep_ids {
        repo.store.read(dep_id)?;
    }

    // Validate planned edges against existing + planned graph to avoid
    // introducing cycles across multi-target edits.
    let all_tasks = repo.store.list_all()?;
    let mut adjacency = build_dependency_adjacency(&all_tasks);
    validate_dependency_plan(&target_ids, &dep_ids, &mut adjacency)?;

    let mut updated_tasks = Vec::with_capacity(target_ids.len());
    for target_id in target_ids {
        let mut task = repo.store.read(target_id)?;

        for &dep_id in &dep_ids {
            if let Some(existing) = task.depends_on.iter_mut().find(|d| d.id == dep_id) {
                if dep_type.is_some() {
                    existing.dep_type = dep_type;
                }
                if reason.is_some() {
                    existing.reason = reason.clone();
                }
            } else {
                task.depends_on.push(Dependency {
                    id: dep_id,
                    dep_type,
                    reason: reason.clone(),
                });
            }
        }

        task.normalize();
        task.updated_at = Utc::now();
        repo.store.write(&task)?;
        repo.index.upsert(&task)?;
        updated_tasks.push(task);
    }

    updated_tasks.sort_by_key(|task| task.id);
    print_updated_tasks(&updated_tasks, format, quiet)
}

pub fn undepend(
    repo_root: &Path,
    ids: Vec<u64>,
    on: Vec<u64>,
    format: Format,
    quiet: bool,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let target_ids = normalize_ids(ids);
    let dep_ids: HashSet<u64> = normalize_ids(on).into_iter().collect();

    let mut updated_tasks = Vec::with_capacity(target_ids.len());
    for target_id in target_ids {
        let mut task = repo.store.read(target_id)?;
        task.depends_on.retain(|dep| !dep_ids.contains(&dep.id));
        task.updated_at = Utc::now();

        repo.store.write(&task)?;
        repo.index.upsert(&task)?;
        updated_tasks.push(task);
    }

    updated_tasks.sort_by_key(|task| task.id);
    print_updated_tasks(&updated_tasks, format, quiet)
}

pub fn reparent(repo_root: &Path, id: u64, to: u64, format: Format, quiet: bool) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    repo.store.read(to)?; // validate parent exists
    if repo.index.would_parent_cycle(id, to)? {
        return Err(TakError::CycleDetected(id));
    }

    let mut task = repo.store.read(id)?;
    task.parent = Some(to);
    task.updated_at = Utc::now();

    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    print_updated_tasks(&[task], format, quiet)
}

pub fn orphan(repo_root: &Path, id: u64, format: Format, quiet: bool) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.parent = None;
    task.updated_at = Utc::now();

    repo.store.write(&task)?;
    repo.index.upsert(&task)?;
    print_updated_tasks(&[task], format, quiet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adjacency(edges: &[(u64, u64)]) -> HashMap<u64, HashSet<u64>> {
        let mut graph = HashMap::new();
        for (from, to) in edges {
            graph.entry(*from).or_insert_with(HashSet::new).insert(*to);
            graph.entry(*to).or_insert_with(HashSet::new);
        }
        graph
    }

    #[test]
    fn validate_dependency_plan_rejects_cycle_against_existing_path() {
        let mut graph = adjacency(&[(2, 1)]);
        let err = validate_dependency_plan(&[1], &[2], &mut graph).unwrap_err();
        assert!(matches!(err, TakError::CycleDetected(1)));
    }

    #[test]
    fn validate_dependency_plan_allows_non_cyclic_edges() {
        let mut graph = adjacency(&[(3, 2)]);
        validate_dependency_plan(&[1], &[3], &mut graph).unwrap();
        assert!(graph.get(&1).is_some_and(|deps| deps.contains(&3)));
    }
}
