use std::collections::HashMap;
use std::fs;
use std::path::Path;

use colored::Colorize;
use serde_json::{Value, json};

use crate::error::Result;
use crate::json_ids::format_task_id;
use crate::model::Task;
use crate::output::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Error,
}

#[derive(Debug)]
struct Check {
    category: &'static str,
    level: Level,
    message: String,
}

impl Check {
    fn ok(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            category,
            level: Level::Ok,
            message: msg.into(),
        }
    }
    fn warn(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            category,
            level: Level::Warn,
            message: msg.into(),
        }
    }
    fn error(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            category,
            level: Level::Error,
            message: msg.into(),
        }
    }

    fn prefix(&self) -> String {
        match self.level {
            Level::Ok => " ok ".green().to_string(),
            Level::Warn => "warn".yellow().to_string(),
            Level::Error => " ERR".red().bold().to_string(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "category": self.category,
            "level": match self.level {
                Level::Ok => "ok",
                Level::Warn => "warn",
                Level::Error => "error",
            },
            "message": self.message,
        })
    }
}

pub fn run(fix: bool, format: Format) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut checks: Vec<Check> = Vec::new();

    let tak_dir = find_tak_dir(&cwd);

    // Core checks
    run_core_checks(&mut checks, tak_dir.as_deref());

    // Index checks (only if .tak/ exists)
    if let Some(ref tak) = tak_dir {
        run_index_checks(&mut checks, tak, fix);
    }

    // Data integrity checks (only if .tak/tasks/ exists)
    if let Some(ref tak) = tak_dir {
        run_integrity_checks(&mut checks, tak);
    }

    // Coordination checks (only if .tak/ exists)
    if let Some(ref tak) = tak_dir {
        run_coordination_checks(&mut checks, tak, fix);
    }

    // Environment checks
    run_env_checks(&mut checks, tak_dir.as_deref());

    // Output
    let passed = checks.iter().filter(|c| c.level == Level::Ok).count();
    let warnings = checks.iter().filter(|c| c.level == Level::Warn).count();
    let errors = checks.iter().filter(|c| c.level == Level::Error).count();

    match format {
        Format::Json => {
            let arr: Vec<Value> = checks.iter().map(|c| c.to_json()).collect();
            let output = json!({
                "checks": arr,
                "summary": {
                    "passed": passed,
                    "warnings": warnings,
                    "errors": errors,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            let mut current_cat = "";
            for check in &checks {
                if check.category != current_cat {
                    if !current_cat.is_empty() {
                        eprintln!();
                    }
                    eprintln!("{}", check.category.bold());
                    current_cat = check.category;
                }
                eprintln!("  {}  {}", check.prefix(), check.message);
            }
            eprintln!();
            eprintln!(
                "{} passed, {} warnings, {} errors",
                passed.to_string().green(),
                warnings.to_string().yellow(),
                if errors > 0 {
                    errors.to_string().red().bold().to_string()
                } else {
                    errors.to_string()
                },
            );
        }
    }

    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Walk up from cwd to find .tak/ directory. Returns the .tak/ path.
fn find_tak_dir(start: &Path) -> Option<std::path::PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let tak = dir.join(".tak");
        if tak.exists() && tak.is_dir() {
            return Some(tak);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn run_core_checks(checks: &mut Vec<Check>, tak_dir: Option<&Path>) {
    let Some(tak) = tak_dir else {
        checks.push(Check::error("Core", "not a tak repo (run tak init)"));
        return;
    };

    checks.push(Check::ok("Core", ".tak/ found"));

    // config.json
    let config_path = tak.join("config.json");
    if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(data) => match serde_json::from_str::<Value>(&data) {
                Ok(val) => {
                    if let Some(v) = val.get("version").and_then(|v| v.as_u64()) {
                        checks.push(Check::ok("Core", format!("config version {v}")));
                    } else {
                        checks.push(Check::warn("Core", "config.json missing version field"));
                    }
                }
                Err(_) => checks.push(Check::error("Core", "corrupt config.json")),
            },
            Err(_) => checks.push(Check::error("Core", "cannot read config.json")),
        }
    } else {
        checks.push(Check::error("Core", "missing config.json"));
    }

    // counter.json is a legacy artifact from pre-random-id allocation and is ignored now.
    let counter_path = tak.join("counter.json");
    if counter_path.exists() {
        match fs::read_to_string(&counter_path) {
            Ok(data) => match serde_json::from_str::<Value>(&data) {
                Ok(val) => {
                    if let Some(n) = val.get("next_id").and_then(|v| v.as_u64()) {
                        checks.push(Check::warn(
                            "Core",
                            format!(
                                "legacy counter.json present (ignored by random task-id allocator, next_id={n})"
                            ),
                        ));
                    } else {
                        checks.push(Check::warn(
                            "Core",
                            "legacy counter.json present (ignored by random task-id allocator)",
                        ));
                    }
                }
                Err(_) => checks.push(Check::warn(
                    "Core",
                    "corrupt legacy counter.json (ignored by random task-id allocator)",
                )),
            },
            Err(_) => checks.push(Check::warn(
                "Core",
                "cannot read legacy counter.json (ignored by random task-id allocator)",
            )),
        }
    } else {
        checks.push(Check::ok("Core", "counterless task-id allocation enabled"));
    }

    // tasks/ directory
    let tasks_dir = tak.join("tasks");
    if tasks_dir.is_dir() {
        checks.push(Check::ok("Core", "tasks/ found"));
    } else {
        checks.push(Check::error("Core", "missing tasks directory"));
    }
}

fn run_index_checks(checks: &mut Vec<Check>, tak: &Path, fix: bool) {
    let index_path = tak.join("index.db");
    let tasks_dir = tak.join("tasks");

    if !tasks_dir.is_dir() {
        return;
    }

    // Count task files
    let file_count = count_task_files(&tasks_dir);

    if !index_path.exists() {
        if fix {
            if let Err(e) = rebuild_index(tak) {
                checks.push(Check::error("Index", format!("rebuild failed: {e}")));
            } else {
                checks.push(Check::ok(
                    "Index",
                    format!("rebuilt index ({file_count} tasks)"),
                ));
            }
        } else {
            checks.push(Check::error("Index", "missing (run tak reindex)"));
        }
        return;
    }

    // Check freshness via fingerprint
    let repo_root = tak.parent().unwrap_or(Path::new("."));
    match crate::store::files::FileStore::open(repo_root) {
        Ok(store) => {
            match store.fingerprint() {
                Ok(current_fp) => {
                    let index = match crate::store::index::Index::open(&index_path) {
                        Ok(idx) => idx,
                        Err(e) => {
                            checks.push(Check::error("Index", format!("cannot open: {e}")));
                            return;
                        }
                    };

                    let stored_fp = index.get_fingerprint().ok().flatten();
                    let stale = stored_fp.as_deref() != Some(current_fp.as_str());

                    if stale {
                        if fix {
                            if let Err(e) = rebuild_index(tak) {
                                checks.push(Check::error("Index", format!("rebuild failed: {e}")));
                            } else {
                                checks.push(Check::ok(
                                    "Index",
                                    format!("rebuilt stale index ({file_count} tasks)"),
                                ));
                            }
                        } else {
                            checks.push(Check::warn("Index", "stale (run tak reindex)"));
                        }
                    } else {
                        // Check count match
                        let indexed = index_task_count(&index);
                        if indexed == file_count {
                            checks.push(Check::ok(
                                "Index",
                                format!("index up to date ({file_count} tasks)"),
                            ));
                        } else {
                            checks.push(Check::warn(
                                "Index",
                                format!("mismatch: {file_count} files, {indexed} indexed"),
                            ));
                        }
                    }
                }
                Err(e) => checks.push(Check::error("Index", format!("fingerprint error: {e}"))),
            }
        }
        Err(e) => checks.push(Check::error("Index", format!("cannot open store: {e}"))),
    }
}

fn run_integrity_checks(checks: &mut Vec<Check>, tak: &Path) {
    let tasks_dir = tak.join("tasks");
    if !tasks_dir.is_dir() {
        return;
    }

    let mut tasks: HashMap<u64, Task> = HashMap::new();
    let mut parse_errors = Vec::new();
    let mut filename_issues = Vec::new();

    // Load all task files
    if let Ok(entries) = fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };

            if !is_valid_task_filename_stem(stem) {
                filename_issues.push(format!(
                    "task file '{}' has invalid name; expected 16 lowercase hex (or legacy numeric)",
                    name
                ));
            }

            match fs::read_to_string(entry.path()) {
                Ok(data) => match serde_json::from_str::<Task>(&data) {
                    Ok(task) => {
                        if let Ok(file_id) = stem.parse::<u64>()
                            && file_id != task.id
                        {
                            filename_issues.push(format!(
                                "task file '{}' id mismatch (filename {}, payload {})",
                                name,
                                format_task_id(file_id),
                                format_task_id(task.id)
                            ));
                        }

                        let task_id = task.id;
                        if tasks.insert(task_id, task).is_some() {
                            filename_issues.push(format!(
                                "duplicate task id found in task files for id {}",
                                format_task_id(task_id)
                            ));
                        }
                    }
                    Err(e) => parse_errors.push(format!("task file '{}': {e}", name)),
                },
                Err(e) => parse_errors.push(format!("task file '{}': {e}", name)),
            }
        }
    }

    if parse_errors.is_empty() {
        checks.push(Check::ok("Data Integrity", "all task JSON files parse"));
    } else {
        for err in &parse_errors {
            checks.push(Check::error("Data Integrity", err.clone()));
        }
    }

    if filename_issues.is_empty() {
        checks.push(Check::ok("Data Integrity", "task filename conventions OK"));
    } else {
        for issue in &filename_issues {
            checks.push(Check::warn("Data Integrity", issue.clone()));
        }
    }

    // Parent ref checks
    let mut parent_issues = Vec::new();
    for (id, task) in &tasks {
        if let Some(parent) = task.parent
            && !tasks.contains_key(&parent)
        {
            parent_issues.push(format!(
                "task {}: parent {} not found",
                format_task_id(*id),
                format_task_id(parent)
            ));
        }
    }
    if parent_issues.is_empty() {
        checks.push(Check::ok("Data Integrity", "parent refs OK"));
    } else {
        for issue in &parent_issues {
            checks.push(Check::warn("Data Integrity", issue.clone()));
        }
    }

    // Dependency ref checks
    let mut dep_issues = Vec::new();
    for (id, task) in &tasks {
        for dep in &task.depends_on {
            if !tasks.contains_key(&dep.id) {
                dep_issues.push(format!(
                    "task {}: depends on {}, not found",
                    format_task_id(*id),
                    format_task_id(dep.id)
                ));
            }
        }
    }
    if dep_issues.is_empty() {
        checks.push(Check::ok("Data Integrity", "dependency refs OK"));
    } else {
        for issue in &dep_issues {
            checks.push(Check::warn("Data Integrity", issue.clone()));
        }
    }

    // Cycle detection (dependency)
    let dep_cycle = detect_dep_cycle(&tasks);
    if let Some(task_id) = dep_cycle {
        checks.push(Check::error(
            "Data Integrity",
            format!("cycle detected involving task {}", format_task_id(task_id)),
        ));
    } else {
        checks.push(Check::ok("Data Integrity", "no cycles"));
    }

    // Parent cycle detection
    let parent_cycle = detect_parent_cycle(&tasks);
    if let Some(task_id) = parent_cycle {
        checks.push(Check::error(
            "Data Integrity",
            format!("parent cycle involving task {}", format_task_id(task_id)),
        ));
    } else {
        checks.push(Check::ok("Data Integrity", "no parent cycles"));
    }
}

fn run_coordination_checks(checks: &mut Vec<Check>, tak: &Path, fix: bool) {
    let runtime_dir = tak.join("runtime");
    let db_path = runtime_dir.join("coordination.db");

    if db_path.exists() {
        match crate::store::coordination_db::CoordinationDb::open(&db_path) {
            Ok(_) => checks.push(Check::ok("Coordination", "coordination.db opens")),
            Err(e) => {
                if fix {
                    // Remove and recreate
                    let _ = fs::remove_file(&db_path);
                    match crate::store::coordination_db::CoordinationDb::open(&db_path) {
                        Ok(_) => checks.push(Check::ok(
                            "Coordination",
                            "coordination.db recreated after error",
                        )),
                        Err(e2) => checks.push(Check::error(
                            "Coordination",
                            format!("cannot recreate coordination.db: {e2}"),
                        )),
                    }
                } else {
                    checks.push(Check::error(
                        "Coordination",
                        format!("cannot open coordination.db: {e}"),
                    ));
                }
            }
        }
    } else if fix {
        let _ = fs::create_dir_all(&runtime_dir);
        match crate::store::coordination_db::CoordinationDb::open(&db_path) {
            Ok(_) => checks.push(Check::ok(
                "Coordination",
                "coordination.db created (was missing)",
            )),
            Err(e) => checks.push(Check::error(
                "Coordination",
                format!("cannot create coordination.db: {e}"),
            )),
        }
    } else {
        checks.push(Check::warn(
            "Coordination",
            "coordination.db missing (run tak doctor --fix or tak init)",
        ));
    }

    // Detect stale file-based coordination dirs
    let old_mesh_dir = runtime_dir.join("mesh");
    let old_blackboard_dir = runtime_dir.join("blackboard");

    if old_mesh_dir.exists() {
        if fix {
            let _ = fs::remove_dir_all(&old_mesh_dir);
            checks.push(Check::ok(
                "Coordination",
                "removed stale runtime/mesh/ directory",
            ));
        } else {
            checks.push(Check::warn(
                "Coordination",
                "stale runtime/mesh/ directory present (run tak doctor --fix to remove)",
            ));
        }
    }

    if old_blackboard_dir.exists() {
        if fix {
            let _ = fs::remove_dir_all(&old_blackboard_dir);
            checks.push(Check::ok(
                "Coordination",
                "removed stale runtime/blackboard/ directory",
            ));
        } else {
            checks.push(Check::warn(
                "Coordination",
                "stale runtime/blackboard/ directory present (run tak doctor --fix to remove)",
            ));
        }
    }
}

fn run_env_checks(checks: &mut Vec<Check>, tak_dir: Option<&Path>) {
    // tak binary in PATH
    let in_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("tak").exists()))
        .unwrap_or(false);

    if in_path {
        checks.push(Check::ok("Environment", "tak in PATH"));
    } else {
        checks.push(Check::warn("Environment", "tak not in PATH"));
    }

    // Claude Code hooks
    match crate::commands::setup::check_hooks_installed() {
        Some(scope) => checks.push(Check::ok(
            "Environment",
            format!("hooks installed ({scope})"),
        )),
        None => checks.push(Check::warn("Environment", "no hooks (run tak setup)")),
    }

    // .gitignore checks (only if we have a tak dir)
    if let Some(tak) = tak_dir {
        let repo_root = tak.parent().unwrap_or(Path::new("."));
        let gitignore_path = repo_root.join(".gitignore");

        if gitignore_path.exists() {
            let content = fs::read_to_string(&gitignore_path).unwrap_or_default();
            check_gitignore_entry(checks, &content, "index.db", &gitignore_path);
            check_gitignore_entry(checks, &content, "claim.lock", &gitignore_path);
        } else {
            checks.push(Check::warn("Environment", ".gitignore not found"));
        }
    }
}

fn check_gitignore_entry(
    checks: &mut Vec<Check>,
    content: &str,
    pattern: &str,
    _gitignore_path: &Path,
) {
    // Simple check: see if the pattern appears in any gitignore line
    let covered = content.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#') && line.contains(pattern)
    });

    if covered {
        checks.push(Check::ok("Environment", format!("{pattern} gitignored")));
    } else {
        checks.push(Check::warn(
            "Environment",
            format!("{pattern} not gitignored"),
        ));
    }
}

fn is_valid_task_filename_stem(stem: &str) -> bool {
    is_taskid_hex_stem(stem) || is_legacy_numeric_stem(stem)
}

fn is_taskid_hex_stem(stem: &str) -> bool {
    stem.len() == 16
        && stem
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn is_legacy_numeric_stem(stem: &str) -> bool {
    !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())
}

fn count_task_files(tasks_dir: &Path) -> usize {
    fs::read_dir(tasks_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".json")
                        .is_some_and(is_valid_task_filename_stem)
                })
                .count()
        })
        .unwrap_or(0)
}

fn index_task_count(index: &crate::store::index::Index) -> usize {
    index
        .conn()
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap_or(0)
}

fn rebuild_index(tak: &Path) -> Result<()> {
    let repo_root = tak.parent().unwrap_or(Path::new("."));
    let repo = crate::store::repo::Repo::open(repo_root)?;
    let tasks = repo.store.list_all()?;
    repo.index.rebuild(&tasks)?;
    let fp = repo.store.fingerprint()?;
    repo.index.set_fingerprint(&fp)?;
    Ok(())
}

/// Detect dependency cycles using DFS. Returns the first task ID involved in a cycle, if any.
fn detect_dep_cycle(tasks: &HashMap<u64, Task>) -> Option<u64> {
    let mut visited = HashMap::new(); // 0 = unvisited, 1 = in-stack, 2 = done

    for &id in tasks.keys() {
        visited.entry(id).or_insert(0);
    }

    for &id in tasks.keys() {
        if visited[&id] == 0
            && let Some(cycle_id) = dfs_dep(id, tasks, &mut visited)
        {
            return Some(cycle_id);
        }
    }
    None
}

fn dfs_dep(id: u64, tasks: &HashMap<u64, Task>, visited: &mut HashMap<u64, u8>) -> Option<u64> {
    visited.insert(id, 1); // in-stack
    if let Some(task) = tasks.get(&id) {
        for dep in &task.depends_on {
            match visited.get(&dep.id).copied() {
                Some(1) => return Some(dep.id),
                Some(0) | None => {
                    if tasks.contains_key(&dep.id)
                        && let Some(c) = dfs_dep(dep.id, tasks, visited)
                    {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
    }
    visited.insert(id, 2); // done
    None
}

/// Detect parent cycles. Returns the first task ID involved in a cycle, if any.
fn detect_parent_cycle(tasks: &HashMap<u64, Task>) -> Option<u64> {
    for (&id, task) in tasks {
        if task.parent.is_some() {
            // Follow parent chain; if we revisit id, there's a cycle
            let mut seen = std::collections::HashSet::new();
            seen.insert(id);
            let mut current = task.parent;
            while let Some(pid) = current {
                if !seen.insert(pid) {
                    return Some(id);
                }
                current = tasks.get(&pid).and_then(|t| t.parent);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Dependency, Execution, GitInfo, Kind, Planning, Status};
    use chrono::Utc;

    fn make_task(id: u64, parent: Option<u64>, deps: Vec<u64>) -> Task {
        let now = Utc::now();
        Task {
            id,
            title: format!("Task {id}"),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent,
            depends_on: deps.into_iter().map(Dependency::simple).collect(),
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        }
    }

    #[test]
    fn validates_task_filename_stems() {
        assert!(is_valid_task_filename_stem("0000000000000001"));
        assert!(is_valid_task_filename_stem("deadbeefcafefeed"));
        assert!(is_valid_task_filename_stem("123")); // legacy numeric

        assert!(!is_valid_task_filename_stem(""));
        assert!(!is_valid_task_filename_stem("DEADBEEFCAFEBABE"));
        assert!(!is_valid_task_filename_stem("not-a-task-id"));
    }

    #[test]
    fn count_task_files_ignores_invalid_stems() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let tasks = dir.path();
        std::fs::write(tasks.join("0000000000000001.json"), "{}").unwrap();
        std::fs::write(tasks.join("2.json"), "{}").unwrap();
        std::fs::write(tasks.join("BAD.json"), "{}").unwrap();
        std::fs::write(tasks.join("notes.txt"), "{}").unwrap();

        assert_eq!(count_task_files(tasks), 2);
    }

    #[test]
    fn no_dep_cycle_in_dag() {
        let mut tasks = HashMap::new();
        tasks.insert(1, make_task(1, None, vec![]));
        tasks.insert(2, make_task(2, None, vec![1]));
        tasks.insert(3, make_task(3, None, vec![2]));
        assert!(detect_dep_cycle(&tasks).is_none());
    }

    #[test]
    fn detects_dep_cycle() {
        let mut tasks = HashMap::new();
        tasks.insert(1, make_task(1, None, vec![3]));
        tasks.insert(2, make_task(2, None, vec![1]));
        tasks.insert(3, make_task(3, None, vec![2]));
        assert!(detect_dep_cycle(&tasks).is_some());
    }

    #[test]
    fn no_parent_cycle_in_tree() {
        let mut tasks = HashMap::new();
        tasks.insert(1, make_task(1, None, vec![]));
        tasks.insert(2, make_task(2, Some(1), vec![]));
        tasks.insert(3, make_task(3, Some(2), vec![]));
        assert!(detect_parent_cycle(&tasks).is_none());
    }

    #[test]
    fn detects_parent_cycle() {
        let mut tasks = HashMap::new();
        tasks.insert(1, make_task(1, Some(3), vec![]));
        tasks.insert(2, make_task(2, Some(1), vec![]));
        tasks.insert(3, make_task(3, Some(2), vec![]));
        assert!(detect_parent_cycle(&tasks).is_some());
    }
}
