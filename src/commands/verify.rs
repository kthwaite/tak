use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::json_ids::format_task_id;
use crate::output::Format;
use crate::store::coordination_db::{CoordinationDb, DbRegistration, DbReservation};
use crate::store::paths::{
    normalize_reservation_path, normalized_paths_conflict, path_conflict_key,
};
use crate::store::repo::Repo;
use crate::store::sidecars::{CommandResult, VerificationResult};

const WAIT_HINT_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum VerifyScopeSelector {
    Unscoped,
    ExplicitPaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifyScopePlan {
    selector: VerifyScopeSelector,
    requested_paths: Vec<String>,
    effective_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifyScopeBlocker {
    owner: String,
    scope_path: String,
    held_path: String,
    reason: Option<String>,
    age_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifyScopeDiagnostics {
    blockers: Vec<VerifyScopeBlocker>,
    suggestions: Vec<String>,
}

impl VerifyScopeDiagnostics {
    fn empty() -> Self {
        Self {
            blockers: vec![],
            suggestions: vec![],
        }
    }

    fn is_blocked(&self) -> bool {
        !self.blockers.is_empty()
    }
}

#[derive(Debug, Serialize)]
struct VerifyScopeBlockerOutput {
    owner: String,
    scope_path: String,
    held_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    age_secs: i64,
}

#[derive(Debug, Serialize)]
struct VerifyScopePlanOutput {
    selector: VerifyScopeSelector,
    requested_paths: Vec<String>,
    effective_paths: Vec<String>,
    blocked: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blockers: Vec<VerifyScopeBlockerOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<String>,
}

impl VerifyScopePlan {
    fn from_scope_paths(repo_root: &Path, scope_paths: &[String]) -> Result<Self> {
        if scope_paths.is_empty() {
            return Ok(Self {
                selector: VerifyScopeSelector::Unscoped,
                requested_paths: vec![],
                effective_paths: vec![],
            });
        }

        let requested_paths = normalize_requested_scope_paths(repo_root, scope_paths)?;
        let effective_paths = collapse_redundant_scope_paths(&requested_paths);

        Ok(Self {
            selector: VerifyScopeSelector::ExplicitPaths,
            requested_paths,
            effective_paths,
        })
    }

    fn has_effective_paths(&self) -> bool {
        !self.effective_paths.is_empty()
    }

    fn as_output(&self, diagnostics: &VerifyScopeDiagnostics) -> VerifyScopePlanOutput {
        VerifyScopePlanOutput {
            selector: self.selector,
            requested_paths: self.requested_paths.clone(),
            effective_paths: self.effective_paths.clone(),
            blocked: diagnostics.is_blocked(),
            blockers: diagnostics
                .blockers
                .iter()
                .map(|blocker| VerifyScopeBlockerOutput {
                    owner: blocker.owner.clone(),
                    scope_path: blocker.scope_path.clone(),
                    held_path: blocker.held_path.clone(),
                    reason: blocker.reason.clone(),
                    age_secs: blocker.age_secs,
                })
                .collect(),
            suggestions: diagnostics.suggestions.clone(),
        }
    }
}

fn normalize_requested_scope_paths(
    repo_root: &Path,
    scope_paths: &[String],
) -> Result<Vec<String>> {
    let mut deduped_by_key: BTreeMap<String, String> = BTreeMap::new();

    for raw_path in scope_paths {
        let normalized = normalize_reservation_path(raw_path, repo_root).map_err(|err| {
            TakError::VerifyInvalidScopePath {
                path: raw_path.clone(),
                reason: err.to_string(),
            }
        })?;

        let key = path_conflict_key(&normalized);
        match deduped_by_key.get_mut(&key) {
            Some(existing) => {
                if normalized < *existing {
                    *existing = normalized;
                }
            }
            None => {
                deduped_by_key.insert(key, normalized);
            }
        }
    }

    Ok(deduped_by_key.into_values().collect())
}

fn collapse_redundant_scope_paths(paths: &[String]) -> Vec<String> {
    let mut by_depth_then_key = paths.to_vec();
    by_depth_then_key.sort_by(|a, b| {
        path_depth(a)
            .cmp(&path_depth(b))
            .then_with(|| path_conflict_key(a).cmp(&path_conflict_key(b)))
    });

    let mut effective: Vec<String> = Vec::new();
    for path in by_depth_then_key {
        if effective
            .iter()
            .any(|existing| normalized_paths_conflict(existing, &path))
        {
            continue;
        }
        effective.push(path);
    }

    effective.sort_by_key(|path| path_conflict_key(path));
    effective
}

fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

fn resolve_verify_agent_name(db: &CoordinationDb) -> Option<String> {
    if let Some(from_env) = std::env::var("TAK_AGENT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        if db.get_agent(&from_env).is_ok() {
            return Some(from_env);
        }
    }

    let agents = db.list_agents().ok()?;
    if agents.is_empty() {
        return None;
    }

    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let by_cwd: Vec<&DbRegistration> = agents
        .iter()
        .filter(|agent| !cwd.is_empty() && agent.cwd == cwd)
        .collect();

    if by_cwd.len() == 1 {
        return Some(by_cwd[0].name.clone());
    }

    if agents.len() == 1 {
        return Some(agents[0].name.clone());
    }

    None
}

fn collect_scope_blockers(
    scope_paths: &[String],
    reservations: &[DbReservation],
    current_agent: Option<&str>,
) -> Vec<VerifyScopeBlocker> {
    let now = Utc::now();
    let mut blockers = Vec::new();

    for scope_path in scope_paths {
        for reservation in reservations {
            if current_agent.is_some_and(|agent| reservation.agent == agent) {
                continue;
            }
            if !normalized_paths_conflict(scope_path, &reservation.path) {
                continue;
            }

            blockers.push(VerifyScopeBlocker {
                owner: reservation.agent.clone(),
                scope_path: scope_path.clone(),
                held_path: reservation.path.clone(),
                reason: reservation.reason.clone(),
                age_secs: (now - reservation.created_at).num_seconds().max(0),
            });
        }
    }

    blockers.sort_by(|left, right| {
        left.owner
            .cmp(&right.owner)
            .then_with(|| left.scope_path.cmp(&right.scope_path))
            .then_with(|| left.held_path.cmp(&right.held_path))
    });

    blockers.dedup_by(|left, right| {
        left.owner == right.owner
            && left.scope_path == right.scope_path
            && left.held_path == right.held_path
    });

    blockers
}

fn build_scope_suggestions(
    task_id: u64,
    current_agent: Option<&str>,
    blockers: &[VerifyScopeBlocker],
) -> Vec<String> {
    let Some(first) = blockers.first() else {
        return vec![];
    };

    let mut suggestions = vec![
        format!("tak mesh blockers --path {}", first.held_path),
        format!(
            "tak wait --path {} --timeout {}",
            first.held_path, WAIT_HINT_TIMEOUT_SECS
        ),
    ];

    let reserve_owner = current_agent.unwrap_or("<agent-name>");
    suggestions.push(format!(
        "tak mesh reserve --name {reserve_owner} --path {} --reason task-{}",
        first.scope_path,
        format_task_id(task_id)
    ));

    suggestions.push(format!(
        "tak verify {} --path {}",
        format_task_id(task_id),
        first.scope_path
    ));

    suggestions
}

fn build_scope_diagnostics(
    repo_root: &Path,
    task_id: u64,
    scope: &VerifyScopePlan,
) -> Result<VerifyScopeDiagnostics> {
    if !scope.has_effective_paths() {
        return Ok(VerifyScopeDiagnostics::empty());
    }

    let db = CoordinationDb::from_repo(repo_root)?;
    let reservations = db.list_reservations()?;
    let current_agent = resolve_verify_agent_name(&db);
    let blockers = collect_scope_blockers(
        &scope.effective_paths,
        &reservations,
        current_agent.as_deref(),
    );

    let suggestions = build_scope_suggestions(task_id, current_agent.as_deref(), &blockers);

    Ok(VerifyScopeDiagnostics {
        blockers,
        suggestions,
    })
}

fn print_scope_summary(scope: &VerifyScopePlan, format: Format) {
    if !scope.has_effective_paths() {
        return;
    }

    let joined = scope.effective_paths.join(", ");
    match format {
        Format::Json => {}
        Format::Pretty => println!("  {} {}", "Scope:".dimmed(), joined.cyan()),
        Format::Minimal => println!("scope {joined}"),
    }
}

fn print_scope_blocked_details(diagnostics: &VerifyScopeDiagnostics, format: Format) {
    if !diagnostics.is_blocked() {
        return;
    }

    match format {
        Format::Json => {}
        Format::Pretty => {
            println!(
                "  {}",
                "Scoped verification blocked by reservation overlap."
                    .red()
                    .bold()
            );
            for blocker in diagnostics.blockers.iter().take(5) {
                let reason = blocker.reason.as_deref().unwrap_or("none");
                println!(
                    "  {} owner={} scope={} held={} reason={} age={}s",
                    "-".dimmed(),
                    blocker.owner.cyan(),
                    blocker.scope_path,
                    blocker.held_path,
                    reason,
                    blocker.age_secs,
                );
            }
            for suggestion in &diagnostics.suggestions {
                println!("  {} {}", "hint:".dimmed(), suggestion);
            }
        }
        Format::Minimal => {
            for blocker in &diagnostics.blockers {
                println!(
                    "blocked owner={} scope={} held={} reason={} age={}s",
                    blocker.owner,
                    blocker.scope_path,
                    blocker.held_path,
                    blocker.reason.as_deref().unwrap_or("none"),
                    blocker.age_secs,
                );
            }
            for suggestion in &diagnostics.suggestions {
                println!("suggest {suggestion}");
            }
        }
    }
}

fn format_scope_blocked_message(task_id: u64, diagnostics: &VerifyScopeDiagnostics) -> String {
    let Some(first) = diagnostics.blockers.first() else {
        return "scoped verify blocked by reservation overlap".to_string();
    };

    let reason = first.reason.as_deref().unwrap_or("none");
    let others = diagnostics.blockers.len().saturating_sub(1);
    let overlap_summary = if others > 0 {
        format!(
            "scope '{}' overlaps '{}' held by '{}' (reason: {reason}, age: {}s) (+{} more overlap(s))",
            first.scope_path, first.held_path, first.owner, first.age_secs, others
        )
    } else {
        format!(
            "scope '{}' overlaps '{}' held by '{}' (reason: {reason}, age: {}s)",
            first.scope_path, first.held_path, first.owner, first.age_secs
        )
    };

    if diagnostics.suggestions.is_empty() {
        return format!(
            "task {} scoped verify blocked: {overlap_summary}",
            format_task_id(task_id)
        );
    }

    format!(
        "task {} scoped verify blocked: {overlap_summary}. remediation: {}",
        format_task_id(task_id),
        diagnostics
            .suggestions
            .iter()
            .map(|hint| format!("`{hint}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn print_json_result(
    result: &VerificationResult,
    scope: &VerifyScopePlan,
    diagnostics: &VerifyScopeDiagnostics,
) -> Result<()> {
    let mut payload = serde_json::to_value(result)?;
    if let serde_json::Value::Object(map) = &mut payload {
        map.insert(
            "scope".to_string(),
            serde_json::to_value(scope.as_output(diagnostics))?,
        );
    }

    println!("{}", serde_json::to_string(&payload)?);
    Ok(())
}

/// Run the verification commands from a task's contract.
///
/// Each command is executed via `sh -c`. Reports pass/fail per command.
/// Returns exit code 0 if all pass, 1 if any fail.
/// Stores the result in `.tak/verification_results/{id}.json`.
pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    run_with_scope(repo_root, id, vec![], format)
}

/// Run verification with optional explicit scope selector paths.
pub fn run_with_scope(
    repo_root: &Path,
    id: u64,
    scope_paths: Vec<String>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(id)?;
    let scope = VerifyScopePlan::from_scope_paths(repo_root, &scope_paths)?;
    let diagnostics = build_scope_diagnostics(repo_root, id, &scope)?;

    if diagnostics.is_blocked() {
        print_scope_summary(&scope, format);
        print_scope_blocked_details(&diagnostics, format);
        return Err(TakError::VerifyScopeBlocked(format_scope_blocked_message(
            id,
            &diagnostics,
        )));
    }

    let commands = &task.contract.verification;

    if commands.is_empty() {
        let vr = VerificationResult {
            timestamp: Utc::now(),
            results: vec![],
            passed: true,
        };
        let _ = repo.sidecars.write_verification(id, &vr);

        match format {
            Format::Json => print_json_result(&vr, &scope, &diagnostics)?,
            Format::Pretty => {
                print_scope_summary(&scope, format);
                eprintln!("No verification commands for task {}", format_task_id(id));
            }
            Format::Minimal => print_scope_summary(&scope, format),
        }
        return Ok(());
    }

    print_scope_summary(&scope, format);

    let mut results = Vec::new();
    let mut all_passed = true;

    for cmd in commands {
        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(repo_root)
            .output();

        let (passed, exit_code, stdout, stderr) = match output {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                (o.status.success(), code, out, err)
            }
            Err(e) => (false, -1, String::new(), e.to_string()),
        };

        if !passed {
            all_passed = false;
        }

        match format {
            Format::Pretty => {
                let icon = if passed {
                    "PASS".green().bold().to_string()
                } else {
                    "FAIL".red().bold().to_string()
                };
                println!("  [{}] {} {}", icon, "$".dimmed(), cmd.cyan());
                if !stderr.is_empty() {
                    for line in stderr.lines() {
                        println!("         {}", line.red());
                    }
                }
            }
            Format::Minimal => {
                let icon = if passed {
                    "ok".green().to_string()
                } else {
                    "FAIL".red().to_string()
                };
                println!("{icon} {cmd}");
            }
            Format::Json => {}
        }

        results.push(CommandResult {
            command: cmd.clone(),
            exit_code,
            stdout,
            stderr,
            passed,
        });
    }

    let vr = VerificationResult {
        timestamp: Utc::now(),
        results,
        passed: all_passed,
    };

    // Store the result
    let _ = repo.sidecars.write_verification(id, &vr);

    match format {
        Format::Json => print_json_result(&vr, &scope, &diagnostics)?,
        Format::Pretty => {
            if all_passed {
                println!("  {}", "All verification commands passed.".green());
            } else {
                println!("  {}", "Some verification commands failed.".red());
            }
        }
        Format::Minimal => {}
    }

    if !all_passed {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn scope_plan_defaults_to_unscoped_without_paths() {
        let dir = tempdir().unwrap();
        let plan = VerifyScopePlan::from_scope_paths(dir.path(), &[]).unwrap();

        assert_eq!(plan.selector, VerifyScopeSelector::Unscoped);
        assert!(plan.requested_paths.is_empty());
        assert!(plan.effective_paths.is_empty());
    }

    #[test]
    fn scope_plan_normalizes_deduplicates_and_collapses_descendants() {
        let dir = tempdir().unwrap();
        let raw = vec![
            "./src/store".to_string(),
            "src/store/".to_string(),
            "src/store/mesh.rs".to_string(),
            "src/model.rs".to_string(),
            "src/./model.rs".to_string(),
        ];

        let plan = VerifyScopePlan::from_scope_paths(dir.path(), &raw).unwrap();

        assert_eq!(plan.selector, VerifyScopeSelector::ExplicitPaths);
        assert_eq!(
            plan.requested_paths,
            vec![
                "src/model.rs".to_string(),
                "src/store".to_string(),
                "src/store/mesh.rs".to_string()
            ]
        );
        assert_eq!(
            plan.effective_paths,
            vec!["src/model.rs".to_string(), "src/store".to_string()]
        );
    }

    #[test]
    fn scope_plan_is_deterministic_across_input_orders() {
        let dir = tempdir().unwrap();
        let left = vec![
            "src/a".to_string(),
            "src/a/b".to_string(),
            "src/c".to_string(),
        ];
        let right = vec![
            "src/c".to_string(),
            "src/a/b".to_string(),
            "src/a".to_string(),
        ];

        let left_plan = VerifyScopePlan::from_scope_paths(dir.path(), &left).unwrap();
        let right_plan = VerifyScopePlan::from_scope_paths(dir.path(), &right).unwrap();

        assert_eq!(left_plan.requested_paths, right_plan.requested_paths);
        assert_eq!(left_plan.effective_paths, right_plan.effective_paths);
    }

    #[test]
    fn scope_plan_rejects_invalid_path_inputs() {
        let dir = tempdir().unwrap();
        let raw = vec!["../outside".to_string()];

        let err = VerifyScopePlan::from_scope_paths(dir.path(), &raw).unwrap_err();
        assert!(matches!(
            err,
            TakError::VerifyInvalidScopePath {
                path,
                reason: _
            } if path == "../outside"
        ));
    }

    fn reservation(
        agent: &str,
        path: &str,
        reason: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> DbReservation {
        DbReservation {
            id: 1,
            agent: agent.to_string(),
            generation: 1,
            path: path.to_string(),
            reason: reason.map(|value| value.to_string()),
            created_at,
            expires_at: created_at + Duration::hours(1),
        }
    }

    #[test]
    fn collect_scope_blockers_excludes_current_agent_and_reports_metadata() {
        let created_at = Utc::now() - Duration::seconds(30);
        let reservations = vec![
            reservation("owner", "src/store", Some("task-owner"), created_at),
            reservation("peer", "src/store", Some("task-peer"), created_at),
        ];

        let blockers = collect_scope_blockers(
            &["src/store/mesh.rs".to_string()],
            &reservations,
            Some("owner"),
        );

        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].owner, "peer");
        assert_eq!(blockers[0].scope_path, "src/store/mesh.rs");
        assert_eq!(blockers[0].held_path, "src/store");
        assert_eq!(blockers[0].reason.as_deref(), Some("task-peer"));
        assert!(blockers[0].age_secs >= 0);
    }

    #[test]
    fn collect_scope_blockers_ignores_non_overlapping_paths() {
        let created_at = Utc::now() - Duration::seconds(5);
        let reservations = vec![reservation("peer", "src/commands", None, created_at)];

        let blockers = collect_scope_blockers(
            &["src/store/mesh.rs".to_string()],
            &reservations,
            Some("owner"),
        );

        assert!(blockers.is_empty());
    }

    #[test]
    fn build_scope_suggestions_includes_blocker_wait_and_reserve_hints() {
        let blockers = vec![VerifyScopeBlocker {
            owner: "peer".to_string(),
            scope_path: "src/store/mesh.rs".to_string(),
            held_path: "src/store".to_string(),
            reason: Some("task-peer".to_string()),
            age_secs: 42,
        }];

        let suggestions = build_scope_suggestions(42, Some("owner"), &blockers);

        assert!(
            suggestions
                .iter()
                .any(|line| line.contains("tak mesh blockers --path src/store"))
        );
        assert!(
            suggestions
                .iter()
                .any(|line| line.contains("tak wait --path src/store --timeout 120"))
        );
        assert!(suggestions
            .iter()
            .any(|line| line.contains("tak mesh reserve --name owner --path src/store/mesh.rs")));
    }
}
