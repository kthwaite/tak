use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{Result, TakError};
use crate::model::{Status, Task};
use crate::output::Format;
use crate::store::migration::rewrite_task_files_atomic;
use crate::store::repo::Repo;
use crate::task_id::TaskId;

const MIGRATED_CONFIG_VERSION: u64 = 3;
const RANDOM_REKEY_MAX_ATTEMPTS_PER_TASK: usize = 128;

#[derive(Debug, Serialize)]
struct PreflightReport {
    dry_run: bool,
    apply_requested: bool,
    rekey_random: bool,
    force: bool,
    task_count: usize,
    learning_count: usize,
    legacy_task_files: usize,
    hash_task_files: usize,
    config_version: Option<u64>,
    target_config_version: u64,
    invalid_task_files: Vec<String>,
    in_progress_tasks: Vec<String>,
    warnings: Vec<String>,
    issues: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ApplyReport {
    task_files_rewritten: usize,
    learnings_updated: usize,
    learning_task_links_rewritten: usize,
    context_files_renamed: usize,
    history_files_renamed: usize,
    verification_files_renamed: usize,
    artifact_dirs_renamed: usize,
    config_version_before: Option<u64>,
    config_version_after: u64,
    rekey_random: bool,
    audit_map_path: String,
    audit_entries: usize,
}

#[derive(Debug, Serialize)]
struct MigrationReport {
    preflight: PreflightReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    apply: Option<ApplyReport>,
}

#[derive(Debug, Serialize)]
struct AuditMapEntry {
    old_id: u64,
    new_id: u64,
    old_task_id: TaskId,
    new_task_id: TaskId,
}

pub fn run(
    repo_root: &Path,
    dry_run: bool,
    force: bool,
    rekey_random: bool,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let preflight = preflight(repo_root, &repo, dry_run, force, rekey_random)?;

    if !preflight.issues.is_empty() {
        let report = MigrationReport {
            preflight,
            apply: None,
        };
        print_report(&report, format)?;
        return Err(TakError::Locked(
            "migrate-ids preflight failed; resolve reported issues first".into(),
        ));
    }

    let apply = if preflight.apply_requested {
        Some(apply_migration(repo_root, &repo, rekey_random)?)
    } else {
        None
    };

    let report = MigrationReport { preflight, apply };
    print_report(&report, format)?;
    Ok(())
}

fn apply_migration(repo_root: &Path, repo: &Repo, rekey_random: bool) -> Result<ApplyReport> {
    let tasks = repo.store.list_all()?;
    let id_map = if rekey_random {
        build_random_id_map(&tasks)?
    } else {
        build_identity_id_map(&tasks)
    };

    // Validate downstream rewrites before mutating task files.
    let _ = repo.learnings.migrate_task_links(&id_map, true)?;
    let _ = repo.sidecars.migrate_task_paths(&id_map, true)?;

    let task_summary = rewrite_task_files_atomic(&repo_root.join(".tak").join("tasks"), &id_map)?;
    let learning_report = repo.learnings.migrate_task_links(&id_map, false)?;
    let sidecar_report = repo.sidecars.migrate_task_paths(&id_map, false)?;

    let config_path = repo_root.join(".tak").join("config.json");
    let config_version_before = update_config_version(&config_path, MIGRATED_CONFIG_VERSION)?;

    let audit_entries = build_audit_entries(&id_map);
    let audit_map_path = write_audit_map(
        &repo_root.join(".tak"),
        &audit_entries,
        config_version_before,
        MIGRATED_CONFIG_VERSION,
        rekey_random,
        task_summary.rewritten,
        learning_report.learnings_updated,
        learning_report.task_links_rewritten,
        sidecar_report.context_files_renamed,
        sidecar_report.history_files_renamed,
        sidecar_report.verification_files_renamed,
        sidecar_report.artifact_dirs_renamed,
    )?;

    // Refresh derived indexes/fingerprints after filesystem rewrites.
    let _ = Repo::open(repo_root)?;

    Ok(ApplyReport {
        task_files_rewritten: task_summary.rewritten,
        learnings_updated: learning_report.learnings_updated,
        learning_task_links_rewritten: learning_report.task_links_rewritten,
        context_files_renamed: sidecar_report.context_files_renamed,
        history_files_renamed: sidecar_report.history_files_renamed,
        verification_files_renamed: sidecar_report.verification_files_renamed,
        artifact_dirs_renamed: sidecar_report.artifact_dirs_renamed,
        config_version_before,
        config_version_after: MIGRATED_CONFIG_VERSION,
        rekey_random,
        audit_map_path: audit_map_path.display().to_string(),
        audit_entries: audit_entries.len(),
    })
}

fn build_identity_id_map(tasks: &[Task]) -> HashMap<u64, u64> {
    tasks.iter().map(|task| (task.id, task.id)).collect()
}

fn build_random_id_map(tasks: &[Task]) -> Result<HashMap<u64, u64>> {
    build_random_id_map_with(tasks, TaskId::generate)
}

fn build_random_id_map_with<F>(tasks: &[Task], mut generate: F) -> Result<HashMap<u64, u64>>
where
    F: FnMut() -> std::result::Result<TaskId, crate::task_id::TaskIdGenerationError>,
{
    let existing_ids: HashSet<u64> = tasks.iter().map(|task| task.id).collect();
    let mut reserved_new_ids = existing_ids.clone();
    let mut id_map = HashMap::with_capacity(tasks.len());

    for task in tasks {
        let mut allocated = None;

        for _ in 0..RANDOM_REKEY_MAX_ATTEMPTS_PER_TASK {
            let candidate = generate().map_err(|err| {
                TakError::Locked(format!(
                    "failed to generate random task id for migration: {err}"
                ))
            })?;
            let candidate_id = candidate.as_u64();

            if reserved_new_ids.insert(candidate_id) {
                allocated = Some(candidate_id);
                break;
            }
        }

        let Some(new_id) = allocated else {
            return Err(TakError::Locked(format!(
                "failed to allocate unique random migration id for task {} after {} attempts",
                TaskId::from(task.id),
                RANDOM_REKEY_MAX_ATTEMPTS_PER_TASK
            )));
        };

        id_map.insert(task.id, new_id);
    }

    Ok(id_map)
}

fn build_audit_entries(id_map: &HashMap<u64, u64>) -> Vec<AuditMapEntry> {
    let mut pairs = id_map
        .iter()
        .map(|(old, new)| (*old, *new))
        .collect::<Vec<_>>();
    pairs.sort_by_key(|(old, _)| *old);
    pairs
        .into_iter()
        .map(|(old_id, new_id)| AuditMapEntry {
            old_id,
            new_id,
            old_task_id: TaskId::from(old_id),
            new_task_id: TaskId::from(new_id),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn write_audit_map(
    tak_root: &Path,
    entries: &[AuditMapEntry],
    config_version_before: Option<u64>,
    config_version_after: u64,
    rekey_random: bool,
    task_files_rewritten: usize,
    learnings_updated: usize,
    learning_task_links_rewritten: usize,
    context_files_renamed: usize,
    history_files_renamed: usize,
    verification_files_renamed: usize,
    artifact_dirs_renamed: usize,
) -> Result<PathBuf> {
    let dir = tak_root.join("migrations");
    fs::create_dir_all(&dir)?;

    let timestamp = Utc::now();
    let path = dir.join(format!(
        "task-id-map-{}.json",
        timestamp.format("%Y%m%dT%H%M%SZ")
    ));

    let payload = json!({
        "generated_at": timestamp.to_rfc3339(),
        "config_version_before": config_version_before,
        "config_version_after": config_version_after,
        "rekey_random": rekey_random,
        "task_files_rewritten": task_files_rewritten,
        "learnings_updated": learnings_updated,
        "learning_task_links_rewritten": learning_task_links_rewritten,
        "context_files_renamed": context_files_renamed,
        "history_files_renamed": history_files_renamed,
        "verification_files_renamed": verification_files_renamed,
        "artifact_dirs_renamed": artifact_dirs_renamed,
        "id_map": entries,
    });

    fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
    Ok(path)
}

fn read_config_version(config_path: &Path) -> Result<Option<u64>> {
    let data = fs::read_to_string(config_path)?;
    let value: Value = serde_json::from_str(&data)?;
    Ok(value.get("version").and_then(Value::as_u64))
}

fn update_config_version(config_path: &Path, target_version: u64) -> Result<Option<u64>> {
    let data = fs::read_to_string(config_path)?;
    let mut value: Value = serde_json::from_str(&data)?;

    let previous = value.get("version").and_then(Value::as_u64);
    let Some(object) = value.as_object_mut() else {
        return Err(TakError::Locked(format!(
            "config file '{}' is not a JSON object",
            config_path.display()
        )));
    };

    object.insert("version".to_string(), json!(target_version));
    fs::write(config_path, serde_json::to_string_pretty(&value)?)?;

    Ok(previous)
}

fn preflight(
    repo_root: &Path,
    repo: &Repo,
    dry_run: bool,
    force: bool,
    rekey_random: bool,
) -> Result<PreflightReport> {
    let tasks = repo.store.list_all()?;
    let task_count = tasks.len();
    let learning_count = repo.learnings.list_all()?.len();

    let tak_root = repo_root.join(".tak");
    let (legacy_task_files, hash_task_files, invalid_task_files) =
        classify_task_filenames(&tak_root.join("tasks"))?;
    let config_version = read_config_version(&tak_root.join("config.json"))?;

    let in_progress_tasks: Vec<String> = tasks
        .iter()
        .filter(|task| task.status == Status::InProgress)
        .map(|task| format!("{} ({})", TaskId::from(task.id), task.title))
        .collect();

    let mut warnings = Vec::new();
    let mut issues = Vec::new();

    if task_count == 0 {
        warnings.push("no tasks found in repository".into());
    }

    if rekey_random {
        warnings.push("random re-key requested: all task IDs will be rewritten".into());
    } else if legacy_task_files == 0 {
        warnings.push("no legacy numeric task filenames detected".into());
    }

    if legacy_task_files > 0 && hash_task_files > 0 {
        warnings.push("mixed legacy/hash task filename layout detected".into());
    }

    if matches!(config_version, Some(v) if v >= MIGRATED_CONFIG_VERSION) {
        warnings.push(format!(
            "config version already at or above migration target ({MIGRATED_CONFIG_VERSION})"
        ));
    }

    if !invalid_task_files.is_empty() {
        issues.push(format!(
            "invalid task filenames present: {}",
            invalid_task_files.join(", ")
        ));
    }

    if !force && !in_progress_tasks.is_empty() {
        issues.push(format!(
            "in-progress tasks must be resolved before migration: {}",
            in_progress_tasks.join(", ")
        ));
    }

    Ok(PreflightReport {
        dry_run,
        apply_requested: !dry_run,
        rekey_random,
        force,
        task_count,
        learning_count,
        legacy_task_files,
        hash_task_files,
        config_version,
        target_config_version: MIGRATED_CONFIG_VERSION,
        invalid_task_files,
        in_progress_tasks,
        warnings,
        issues,
    })
}

fn classify_task_filenames(tasks_dir: &Path) -> Result<(usize, usize, Vec<String>)> {
    if !tasks_dir.exists() {
        return Ok((0, 0, vec!["<missing .tak/tasks directory>".into()]));
    }

    let mut legacy = 0;
    let mut hash = 0;
    let mut invalid = Vec::new();

    for entry in fs::read_dir(tasks_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };

        if is_taskid_hex_stem(stem) {
            hash += 1;
        } else if is_legacy_numeric_stem(stem) {
            legacy += 1;
        } else {
            invalid.push(name.to_string());
        }
    }

    invalid.sort();
    Ok((legacy, hash, invalid))
}

fn is_taskid_hex_stem(stem: &str) -> bool {
    stem.len() == TaskId::HEX_LEN
        && stem
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn is_legacy_numeric_stem(stem: &str) -> bool {
    !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())
}

fn print_report(report: &MigrationReport, format: Format) -> Result<()> {
    let preflight = &report.preflight;

    match format {
        Format::Json => println!("{}", serde_json::to_string(report)?),
        Format::Pretty => {
            let mode = if preflight.dry_run {
                "dry-run"
            } else {
                "apply"
            };
            println!(
                "{} {}",
                "migrate-ids preflight".bold(),
                format!("({mode})").dimmed()
            );
            println!(
                "  {} {}",
                "tasks:".dimmed(),
                format!(
                    "{} (legacy files: {}, hash files: {})",
                    preflight.task_count, preflight.legacy_task_files, preflight.hash_task_files
                )
            );
            println!("  {} {}", "learnings:".dimmed(), preflight.learning_count);
            println!(
                "  {} {} -> {}",
                "config version:".dimmed(),
                preflight
                    .config_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing>".to_string()),
                preflight.target_config_version
            );
            println!("  {} {}", "force:".dimmed(), preflight.force);
            println!("  {} {}", "rekey random:".dimmed(), preflight.rekey_random);

            if !preflight.warnings.is_empty() {
                println!("\n{}", "Warnings".yellow().bold());
                for warning in &preflight.warnings {
                    println!("  - {}", warning.yellow());
                }
            }

            if !preflight.issues.is_empty() {
                println!("\n{}", "Issues".red().bold());
                for issue in &preflight.issues {
                    println!("  - {}", issue.red());
                }
            }

            if preflight.issues.is_empty() {
                if let Some(apply) = &report.apply {
                    println!("\n{}", "Migration applied".green().bold());
                    println!(
                        "  {} {}",
                        "task files rewritten:".dimmed(),
                        apply.task_files_rewritten
                    );
                    println!(
                        "  {} {} learnings / {} links",
                        "learning links rewritten:".dimmed(),
                        apply.learnings_updated,
                        apply.learning_task_links_rewritten
                    );
                    println!(
                        "  {} {} context, {} history, {} verification, {} artifacts",
                        "sidecars renamed:".dimmed(),
                        apply.context_files_renamed,
                        apply.history_files_renamed,
                        apply.verification_files_renamed,
                        apply.artifact_dirs_renamed
                    );
                    println!(
                        "  {} {} -> {}",
                        "config version:".dimmed(),
                        apply
                            .config_version_before
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "<missing>".to_string()),
                        apply.config_version_after
                    );
                    println!("  {} {}", "rekey random:".dimmed(), apply.rekey_random);
                    println!("  {} {}", "audit map:".dimmed(), apply.audit_map_path);
                } else {
                    println!("\n{}", "Preflight passed; dry-run made no changes.".green());
                }
            }
        }
        Format::Minimal => {
            println!(
                "dry_run={} apply={} rekey_random={} tasks={} legacy={} hash={} issues={} warnings={} applied={} audit_entries={}",
                preflight.dry_run,
                preflight.apply_requested,
                preflight.rekey_random,
                preflight.task_count,
                preflight.legacy_task_files,
                preflight.hash_task_files,
                preflight.issues.len(),
                preflight.warnings.len(),
                report.apply.is_some(),
                report.apply.as_ref().map(|a| a.audit_entries).unwrap_or(0)
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Dependency, Kind, LearningCategory, Planning, Status};
    use crate::store::files::FileStore;
    use crate::store::learnings::LearningStore;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn classify_task_filenames_counts_legacy_hash_and_invalid() {
        let dir = tempdir().unwrap();
        let tasks_dir = dir.path();

        fs::write(tasks_dir.join("1.json"), "{}").unwrap();
        fs::write(tasks_dir.join("0000000000000002.json"), "{}").unwrap();
        fs::write(tasks_dir.join("BAD.json"), "{}").unwrap();
        fs::write(tasks_dir.join("note.txt"), "{}").unwrap();

        let (legacy, hash, invalid) = classify_task_filenames(tasks_dir).unwrap();
        assert_eq!(legacy, 1);
        assert_eq!(hash, 1);
        assert_eq!(invalid, vec!["BAD.json".to_string()]);
    }

    #[test]
    fn preflight_flags_in_progress_tasks_without_force() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let mut task = store
            .create(
                "in progress".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        task.status = Status::InProgress;
        task.updated_at = Utc::now();
        store.write(&task).unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let report = preflight(dir.path(), &repo, true, false, false).unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.contains("in-progress tasks must be resolved"))
        );
    }

    #[test]
    fn preflight_force_bypasses_in_progress_gate() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let mut task = store
            .create(
                "in progress".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        task.status = Status::InProgress;
        task.updated_at = Utc::now();
        store.write(&task).unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let report = preflight(dir.path(), &repo, true, true, false).unwrap();

        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.contains("in-progress tasks must be resolved"))
        );
    }

    #[test]
    fn preflight_rekey_random_skips_missing_legacy_warning() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let _task = store
            .create(
                "canonical task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let report = preflight(dir.path(), &repo, true, false, true).unwrap();

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("random re-key requested"))
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("no legacy numeric task filenames"))
        );
    }

    #[test]
    fn build_random_id_map_with_avoids_existing_ids_and_retries_collisions() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let first = store
            .create(
                "first".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let second = store
            .create(
                "second".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let tasks = store.list_all().unwrap();
        let existing: std::collections::HashSet<u64> = tasks.iter().map(|task| task.id).collect();

        let mut next_fresh = 1_u64;
        while existing.contains(&next_fresh) {
            next_fresh += 1;
        }
        let fresh_a = next_fresh;

        next_fresh += 1;
        while existing.contains(&next_fresh) || next_fresh == fresh_a {
            next_fresh += 1;
        }
        let fresh_b = next_fresh;

        let sequence = [first.id, fresh_a, second.id, fresh_b];
        let mut idx = 0;
        let map = build_random_id_map_with(&tasks, || {
            let value = sequence[idx];
            idx += 1;
            Ok(TaskId::from(value))
        })
        .unwrap();

        assert_eq!(map.len(), 2);
        let mapped_ids: std::collections::HashSet<u64> = map.values().copied().collect();
        assert_eq!(mapped_ids.len(), 2);
        assert!(mapped_ids.iter().all(|id| !existing.contains(id)));
    }

    #[test]
    fn apply_mode_with_rekey_random_rewrites_canonical_task_ids() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();

        let root = store
            .create(
                "root".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let child = store
            .create(
                "child".into(),
                Kind::Task,
                None,
                Some(root.id),
                vec![root.id],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        fs::create_dir_all(store.root().join("context")).unwrap();
        fs::write(
            store
                .root()
                .join("context")
                .join(format!("{}.md", TaskId::from(child.id))),
            "context",
        )
        .unwrap();

        let learning_store = LearningStore::open(store.root());
        let learning = learning_store
            .create(
                "child-link".into(),
                Some("linked to child task".into()),
                LearningCategory::Insight,
                vec!["migration".into()],
                vec![child.id],
            )
            .unwrap();

        run(dir.path(), false, false, true, Format::Json).unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let tasks = repo.store.list_all().unwrap();
        assert_eq!(tasks.len(), 2);

        let migrated_root = tasks.iter().find(|task| task.title == "root").unwrap();
        let migrated_child = tasks.iter().find(|task| task.title == "child").unwrap();

        assert_ne!(migrated_root.id, root.id);
        assert_ne!(migrated_child.id, child.id);
        assert_eq!(migrated_child.parent, Some(migrated_root.id));
        assert_eq!(
            migrated_child.depends_on,
            vec![Dependency::simple(migrated_root.id)]
        );

        let migrated_learning = repo.learnings.read(learning.id).unwrap();
        assert_eq!(migrated_learning.task_ids, vec![migrated_child.id]);

        let tasks_dir = store.root().join("tasks");
        assert!(
            !tasks_dir
                .join(format!("{}.json", TaskId::from(root.id)))
                .exists()
        );
        assert!(
            !tasks_dir
                .join(format!("{}.json", TaskId::from(child.id)))
                .exists()
        );
        assert!(
            tasks_dir
                .join(format!("{}.json", TaskId::from(migrated_root.id)))
                .exists()
        );
        assert!(
            tasks_dir
                .join(format!("{}.json", TaskId::from(migrated_child.id)))
                .exists()
        );

        let old_context = store
            .root()
            .join("context")
            .join(format!("{}.md", TaskId::from(child.id)));
        let new_context = store
            .root()
            .join("context")
            .join(format!("{}.md", TaskId::from(migrated_child.id)));
        assert!(!old_context.exists());
        assert!(new_context.exists());

        let config: Value =
            serde_json::from_str(&fs::read_to_string(store.root().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(
            config["version"],
            serde_json::json!(MIGRATED_CONFIG_VERSION)
        );

        let mut audit_files = fs::read_dir(store.root().join("migrations"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        audit_files.sort();
        assert_eq!(audit_files.len(), 1);

        let audit: Value =
            serde_json::from_str(&fs::read_to_string(&audit_files[0]).unwrap()).unwrap();
        assert_eq!(audit["rekey_random"], serde_json::json!(true));

        let id_map = audit["id_map"].as_array().unwrap();
        assert_eq!(id_map.len(), 2);
        assert!(id_map.iter().all(|entry| {
            entry
                .get("old_id")
                .and_then(Value::as_u64)
                .zip(entry.get("new_id").and_then(Value::as_u64))
                .map(|(old_id, new_id)| old_id != new_id)
                .unwrap_or(false)
        }));
    }

    #[test]
    fn apply_mode_writes_audit_map_and_bumps_config_version() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "legacy task".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let tasks_dir = store.root().join("tasks");
        fs::rename(
            tasks_dir.join(format!("{}.json", TaskId::from(task.id))),
            tasks_dir.join(format!("{}.json", task.id)),
        )
        .unwrap();

        fs::create_dir_all(store.root().join("context")).unwrap();
        fs::write(
            store.root().join("context").join(format!("{}.md", task.id)),
            "legacy context",
        )
        .unwrap();

        run(dir.path(), false, false, false, Format::Json).unwrap();

        let config: Value =
            serde_json::from_str(&fs::read_to_string(store.root().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(
            config["version"],
            serde_json::json!(MIGRATED_CONFIG_VERSION)
        );

        assert!(!tasks_dir.join(format!("{}.json", task.id)).exists());
        assert!(
            tasks_dir
                .join(format!("{}.json", TaskId::from(task.id)))
                .exists()
        );

        let legacy_context = store.root().join("context").join(format!("{}.md", task.id));
        let migrated_context = store
            .root()
            .join("context")
            .join(format!("{}.md", TaskId::from(task.id)));
        assert!(!legacy_context.exists());
        assert!(migrated_context.exists());

        let mut audit_files = fs::read_dir(store.root().join("migrations"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        audit_files.sort();
        assert_eq!(audit_files.len(), 1);

        let audit: Value =
            serde_json::from_str(&fs::read_to_string(&audit_files[0]).unwrap()).unwrap();
        assert_eq!(
            audit["config_version_after"],
            serde_json::json!(MIGRATED_CONFIG_VERSION)
        );
        assert_eq!(audit["id_map"].as_array().unwrap().len(), 1);
        assert_eq!(audit["id_map"][0]["old_id"], serde_json::json!(task.id));
        assert_eq!(audit["id_map"][0]["new_id"], serde_json::json!(task.id));
    }
}
