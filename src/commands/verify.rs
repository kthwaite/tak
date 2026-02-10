use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::paths::{
    normalize_reservation_path, normalized_paths_conflict, path_conflict_key,
};
use crate::store::repo::Repo;
use crate::store::sidecars::{CommandResult, VerificationResult};

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

#[derive(Debug, Serialize)]
struct VerifyScopePlanOutput {
    selector: VerifyScopeSelector,
    requested_paths: Vec<String>,
    effective_paths: Vec<String>,
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

    fn as_output(&self) -> VerifyScopePlanOutput {
        VerifyScopePlanOutput {
            selector: self.selector,
            requested_paths: self.requested_paths.clone(),
            effective_paths: self.effective_paths.clone(),
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

fn print_json_result(result: &VerificationResult, scope: &VerifyScopePlan) -> Result<()> {
    let mut payload = serde_json::to_value(result)?;
    if let serde_json::Value::Object(map) = &mut payload {
        map.insert(
            "scope".to_string(),
            serde_json::to_value(scope.as_output())?,
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

    let commands = &task.contract.verification;

    if commands.is_empty() {
        let vr = VerificationResult {
            timestamp: Utc::now(),
            results: vec![],
            passed: true,
        };
        let _ = repo.sidecars.write_verification(id, &vr);

        match format {
            Format::Json => print_json_result(&vr, &scope)?,
            Format::Pretty => {
                print_scope_summary(&scope, format);
                eprintln!("No verification commands for task {id}");
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
        Format::Json => print_json_result(&vr, &scope)?,
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
}
