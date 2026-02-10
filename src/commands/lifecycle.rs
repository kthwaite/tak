use crate::build_info;
use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::{self, Format};
use crate::store::coordination::{CoordinationLinks, derive_links_from_text};
use crate::store::repo::Repo;
use crate::store::sidecars::HistoryEvent;
use crate::{git, model};
use chrono::Utc;
use std::path::Path;

fn transition(current: Status, target: Status) -> std::result::Result<(), (String, String)> {
    let allowed = match current {
        Status::Pending => matches!(target, Status::InProgress | Status::Cancelled),
        Status::InProgress => matches!(target, Status::Done | Status::Cancelled | Status::Pending),
        Status::Done => matches!(target, Status::Pending),
        Status::Cancelled => matches!(target, Status::Pending),
    };
    if allowed {
        Ok(())
    } else {
        Err((current.to_string(), target.to_string()))
    }
}

fn handoff_links_from_summary(summary: &str) -> CoordinationLinks {
    let mut links = derive_links_from_text(summary);
    links.normalize();
    links
}

fn is_tak_source_repo(repo_root: &Path) -> bool {
    repo_root.join("Cargo.toml").exists()
        && repo_root.join("src/main.rs").exists()
        && repo_root.join("pi-plugin/extensions/tak.ts").exists()
        && repo_root
            .join("claude-plugin/skills/task-execution/SKILL.md")
            .exists()
}

fn is_tak_functionality_path(path: &str) -> bool {
    path == "Cargo.toml"
        || path == "Cargo.lock"
        || path.starts_with("src/")
        || path.starts_with("pi-plugin/")
        || path.starts_with("claude-plugin/")
}

fn is_docs_path(path: &str) -> bool {
    path == "README.md"
        || path == "CLAUDE.md"
        || path.starts_with("docs/")
        || path.starts_with("claude-plugin/skills/")
        || path == "pi-plugin/README.md"
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn collect_epic_finish_hygiene_issues(
    changed_paths: &[String],
    head_sha: &str,
    binary_git_sha: Option<&str>,
    embedded_assets_match_repo: Option<bool>,
    project_pi_status: Option<&str>,
) -> Vec<String> {
    if !changed_paths
        .iter()
        .any(|path| is_tak_functionality_path(path))
    {
        return vec![];
    }

    let mut issues = Vec::new();

    if !changed_paths.iter().any(|path| is_docs_path(path)) {
        issues.push(
            "docs are not updated in this epic commit range (expected README.md, CLAUDE.md, docs/, or skill docs changes)".to_string(),
        );
    }

    match binary_git_sha {
        Some(binary_sha) if binary_sha == head_sha => {}
        Some(binary_sha) => issues.push(format!(
            "tak binary is out of sync with HEAD (binary={}, head={}); run `cargo install --path .`",
            short_sha(binary_sha),
            short_sha(head_sha)
        )),
        None => issues.push(
            "tak binary lacks build git metadata; reinstall with `cargo install --path .`"
                .to_string(),
        ),
    }

    match embedded_assets_match_repo {
        Some(true) => {}
        Some(false) => issues.push(
            "this tak binary embeds outdated pi assets relative to `pi-plugin/`; rebuild with `cargo install --path .`".to_string(),
        ),
        None => issues.push(
            "unable to compare embedded pi assets against repository source".to_string(),
        ),
    }

    if project_pi_status.is_some_and(|status| status != "installed") {
        issues.push("project .pi integration is not synced; run `tak setup --pi`".to_string());
    }

    issues
}

fn enforce_epic_finish_hygiene(repo_root: &Path, task: &model::Task) -> Result<()> {
    if !matches!(task.kind, model::Kind::Epic) || !is_tak_source_repo(repo_root) {
        return Ok(());
    }

    let Some(start_commit) = task.git.start_commit.as_deref() else {
        return Ok(());
    };
    let Some(head_info) = git::current_head_info(repo_root) else {
        return Ok(());
    };

    let changed_paths = git::changed_files_since(repo_root, start_commit, &head_info.sha);

    let embedded_assets_match_repo =
        crate::commands::setup::embedded_pi_assets_match_repo_source(repo_root).ok();
    let project_pi_status = Some(crate::commands::setup::check_project_pi_installed(
        repo_root,
    ));

    let issues = collect_epic_finish_hygiene_issues(
        &changed_paths,
        &head_info.sha,
        build_info::git_sha(),
        embedded_assets_match_repo,
        project_pi_status,
    );

    if issues.is_empty() {
        Ok(())
    } else {
        Err(TakError::EpicFinishHygiene(issues.join("; ")))
    }
}

pub fn start(repo_root: &Path, id: u64, assignee: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::InProgress)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    if repo.index.is_blocked(id)? {
        return Err(TakError::TaskBlocked(id));
    }

    task.status = Status::InProgress;
    task.execution.attempt_count += 1;
    if let Some(ref a) = assignee {
        task.assignee = Some(a.clone());
    }

    // Capture git HEAD on first start (only if not already set)
    if task.git.start_commit.is_none()
        && let Some(info) = git::current_head_info(repo_root)
    {
        task.git = model::GitInfo {
            branch: info.branch,
            start_commit: Some(info.sha),
            ..task.git
        };
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "started".into(),
        agent: task.assignee.clone(),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn finish(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Done)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    enforce_epic_finish_hygiene(repo_root, &task)?;

    task.status = Status::Done;

    // Capture end commit and collect commit range if start_commit exists
    if let Some(info) = git::current_head_info(repo_root) {
        task.git.end_commit = Some(info.sha.clone());

        if let Some(ref start) = task.git.start_commit {
            task.git.commits = git::commits_since(repo_root, start, &info.sha);
        }
    }

    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "finished".into(),
        agent: task.assignee.clone(),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn cancel(repo_root: &Path, id: u64, reason: Option<String>, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Cancelled)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Cancelled;
    if let Some(ref r) = reason {
        task.execution.last_error = Some(r.clone());
    }
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let mut detail = serde_json::Map::new();
    if let Some(ref r) = reason {
        detail.insert("reason".into(), serde_json::Value::String(r.clone()));
    }
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "cancelled".into(),
        agent: task.assignee.clone(),
        detail,
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn handoff(repo_root: &Path, id: u64, summary: String, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Pending)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Pending;
    task.assignee = None;
    task.execution.handoff_summary = Some(summary.clone());
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let mut detail = serde_json::Map::new();
    let links = handoff_links_from_summary(&summary);
    detail.insert("summary".into(), serde_json::Value::String(summary));

    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "handoff".into(),
        agent: None,
        detail,
        links,
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn reopen(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    transition(task.status, Status::Pending)
        .map_err(|(from, to)| TakError::InvalidTransition(from, to))?;

    task.status = Status::Pending;
    task.assignee = None;
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "reopened".into(),
        agent: None,
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

pub fn unassign(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(id)?;

    task.assignee = None;
    task.updated_at = Utc::now();
    repo.store.write(&task)?;
    repo.index.upsert(&task)?;

    // Best-effort history logging
    let evt = HistoryEvent {
        id: None,
        timestamp: Utc::now(),
        event: "unassigned".into(),
        agent: None,
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    let _ = repo.sidecars.append_history(id, &evt);

    output::print_task(&task, format)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_links_extracts_blackboard_and_mesh_refs() {
        let links = handoff_links_from_summary(
            "handoff via B17 after mesh ping 550e8400-e29b-41d4-a716-446655440000",
        );

        assert_eq!(links.blackboard_note_ids, vec![17]);
        assert_eq!(
            links.mesh_message_ids,
            vec!["550e8400-e29b-41d4-a716-446655440000"]
        );
    }

    #[test]
    fn hygiene_issues_are_empty_when_non_functional_changes_only() {
        let changed = vec!["docs/how/channel-contract.md".to_string()];
        let issues = collect_epic_finish_hygiene_issues(
            &changed,
            "abcdef0123456789",
            Some("abcdef0123456789"),
            Some(true),
            Some("installed"),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn hygiene_issues_include_docs_binary_and_pi_actions_when_missing() {
        let changed = vec!["src/commands/work.rs".to_string()];
        let issues = collect_epic_finish_hygiene_issues(
            &changed,
            "abcdef0123456789",
            Some("1234567890abcdef"),
            Some(false),
            Some("outdated"),
        );

        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("docs are not updated"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("tak binary is out of sync"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("embeds outdated pi assets"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("run `tak setup --pi`"))
        );
    }

    #[test]
    fn docs_change_satisfies_docs_check_for_functional_epic() {
        let changed = vec!["src/commands/work.rs".to_string(), "README.md".to_string()];
        let issues = collect_epic_finish_hygiene_issues(
            &changed,
            "abcdef0123456789",
            Some("abcdef0123456789"),
            Some(true),
            Some("installed"),
        );

        assert!(issues.is_empty());
    }
}
