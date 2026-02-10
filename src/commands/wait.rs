use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use colored::Colorize;
use serde_json::json;

use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::Format;
use crate::store::mesh::{MeshStore, Reservation};
use crate::store::repo::Repo;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathBlocker {
    agent: String,
    held_path: String,
    reason: Option<String>,
    age_secs: i64,
}

pub fn run(
    repo_root: &Path,
    path: Option<String>,
    on_task: Option<u64>,
    timeout_secs: Option<u64>,
    format: Format,
) -> Result<()> {
    match (path, on_task) {
        (Some(path), None) => wait_for_path(repo_root, &path, timeout_secs, format),
        (None, Some(task_id)) => wait_for_task(repo_root, task_id, timeout_secs, format),
        _ => Err(TakError::WaitInvalidTarget),
    }
}

fn wait_for_path(
    repo_root: &Path,
    path: &str,
    timeout_secs: Option<u64>,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let target = normalize_path(path);
    let started = Instant::now();

    loop {
        let reservations = store.list_reservations()?;
        let blockers = find_path_blockers(&target, &reservations);

        if blockers.is_empty() {
            print_path_ready(&target, started.elapsed(), format);
            return Ok(());
        }

        if timed_out(started, timeout_secs) {
            return Err(TakError::WaitTimeout(format_path_timeout(
                &target,
                started.elapsed(),
                &blockers,
            )));
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_task(
    repo_root: &Path,
    task_id: u64,
    timeout_secs: Option<u64>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    // Fail fast if the task does not exist.
    repo.store.read(task_id)?;

    let started = Instant::now();

    loop {
        if !repo.index.is_blocked(task_id)? {
            print_task_ready(task_id, started.elapsed(), format);
            return Ok(());
        }

        if timed_out(started, timeout_secs) {
            let blockers = unresolved_dependency_ids(&repo, task_id)?;
            return Err(TakError::WaitTimeout(format_task_timeout(
                task_id,
                started.elapsed(),
                &blockers,
            )));
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn unresolved_dependency_ids(repo: &Repo, task_id: u64) -> Result<Vec<u64>> {
    let task = repo.store.read(task_id)?;
    let mut blockers = Vec::new();

    for dep in task.depends_on {
        let dep_task = repo.store.read(dep.id)?;
        if !matches!(dep_task.status, Status::Done | Status::Cancelled) {
            blockers.push(dep.id);
        }
    }

    blockers.sort_unstable();
    blockers.dedup();
    Ok(blockers)
}

fn find_path_blockers(target: &str, reservations: &[Reservation]) -> Vec<PathBlocker> {
    let now = Utc::now();
    let mut blockers = Vec::new();

    for reservation in reservations {
        for held_path in &reservation.paths {
            if paths_conflict(target, held_path) {
                blockers.push(PathBlocker {
                    agent: reservation.agent.clone(),
                    held_path: held_path.clone(),
                    reason: reservation.reason.clone(),
                    age_secs: (now - reservation.since).num_seconds().max(0),
                });
            }
        }
    }

    blockers.sort_by(|a, b| a.agent.cmp(&b.agent).then(a.held_path.cmp(&b.held_path)));
    blockers
}

fn print_path_ready(path: &str, waited: Duration, format: Format) {
    let waited_ms = waited.as_millis() as u64;
    match format {
        Format::Json => println!(
            "{}",
            json!({
                "mode": "path",
                "path": path,
                "status": "ready",
                "waited_ms": waited_ms
            })
        ),
        Format::Pretty => println!(
            "{} {} {}",
            "Ready:".green().bold(),
            path.cyan(),
            format!("(waited {waited_ms}ms)").dimmed()
        ),
        Format::Minimal => println!("{path}"),
    }
}

fn print_task_ready(task_id: u64, waited: Duration, format: Format) {
    let waited_ms = waited.as_millis() as u64;
    match format {
        Format::Json => println!(
            "{}",
            json!({
                "mode": "task",
                "task_id": task_id,
                "status": "ready",
                "waited_ms": waited_ms
            })
        ),
        Format::Pretty => println!(
            "{} task {} {}",
            "Ready:".green().bold(),
            task_id.to_string().cyan(),
            format!("(waited {waited_ms}ms)").dimmed()
        ),
        Format::Minimal => println!("{task_id}"),
    }
}

fn format_path_timeout(path: &str, waited: Duration, blockers: &[PathBlocker]) -> String {
    let waited_ms = waited.as_millis();
    if let Some(blocker) = blockers.first() {
        let reason = blocker.reason.as_deref().unwrap_or("none");
        format!(
            "path '{path}' is still blocked by agent '{}' via '{}' (reason: {reason}, age: {}s) after {waited_ms}ms",
            blocker.agent, blocker.held_path, blocker.age_secs
        )
    } else {
        format!("path '{path}' is still blocked after {waited_ms}ms")
    }
}

fn format_task_timeout(task_id: u64, waited: Duration, blockers: &[u64]) -> String {
    let waited_ms = waited.as_millis();
    if blockers.is_empty() {
        format!("task {task_id} is still blocked after {waited_ms}ms")
    } else {
        let deps = blockers
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "task {task_id} is still blocked by unfinished dependencies [{deps}] after {waited_ms}ms"
        )
    }
}

fn timed_out(started: Instant, timeout_secs: Option<u64>) -> bool {
    timeout_secs
        .map(Duration::from_secs)
        .is_some_and(|timeout| started.elapsed() >= timeout)
}

/// Lexically normalize a path: resolve `.`/`..` components and collapse duplicate
/// separators. Preserves trailing slash (directory indicator).
fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            c => components.push(c),
        }
    }
    let normalized = components.join("/");
    if path.ends_with('/') && !normalized.is_empty() {
        format!("{normalized}/")
    } else {
        normalized
    }
}

/// Two paths conflict if one is a prefix of the other (directory containment)
/// or they are exactly equal.
fn paths_conflict(a: &str, b: &str) -> bool {
    let a = normalize_path(a);
    let b = normalize_path(b);
    if a == b {
        return true;
    }

    let a_trimmed = a.trim_end_matches('/');
    let b_trimmed = b.trim_end_matches('/');
    if a_trimmed == b_trimmed {
        return true;
    }

    let a_dir = format!("{a_trimmed}/");
    let b_dir = format!("{b_trimmed}/");
    b_trimmed.starts_with(&a_dir) || a_trimmed.starts_with(&b_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_cases() {
        assert_eq!(normalize_path("src/./lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src/../src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src//lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("./src/store/"), "src/store/");
        assert_eq!(normalize_path("src/store"), "src/store");
    }

    #[test]
    fn paths_conflict_cases() {
        assert!(paths_conflict("src/store/", "src/store/mesh.rs"));
        assert!(paths_conflict("src/store/mesh.rs", "src/store/"));
        assert!(paths_conflict("src/store", "src/store/"));
        assert!(!paths_conflict("src/store/", "src/model.rs"));
        assert!(paths_conflict("./src/store/", "src/store/mesh.rs"));
    }

    #[test]
    fn find_path_blockers_includes_metadata() {
        let reservations = vec![Reservation {
            agent: "A".into(),
            paths: vec!["src/store/".into()],
            reason: Some("task-1".into()),
            since: Utc::now(),
            ttl_secs: None,
            last_heartbeat_at: None,
            expires_at: None,
        }];

        let blockers = find_path_blockers("src/store/mesh.rs", &reservations);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].agent, "A");
        assert_eq!(blockers[0].held_path, "src/store/");
        assert_eq!(blockers[0].reason.as_deref(), Some("task-1"));
    }
}
