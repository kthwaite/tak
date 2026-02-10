use std::path::Path;

use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::model::{Status, Task};
use crate::output::Format;
use crate::store::mesh::MeshStore;
use crate::store::repo::Repo;
use crate::store::work::{
    WorkClaimStrategy, WorkCoordinationVerbosity, WorkState, WorkStore, WorkVerifyMode,
};
use crate::task_id::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkEvent {
    Continued,
    Attached,
    Claimed,
    NoWork,
    LimitReached,
    Status,
    Stopped,
}

impl WorkEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Continued => "continued",
            Self::Attached => "attached",
            Self::Claimed => "claimed",
            Self::NoWork => "no_work",
            Self::LimitReached => "limit_reached",
            Self::Status => "status",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Serialize)]
struct WorkResponse {
    event: WorkEvent,
    agent: String,
    #[serde(rename = "loop")]
    state: WorkState,
    current_task: Option<Task>,
}

pub fn start_or_resume(
    repo_root: &Path,
    assignee: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    format: Format,
) -> Result<()> {
    start_or_resume_with_strategy(
        repo_root,
        assignee,
        tag,
        limit,
        verify_mode,
        None,
        None,
        format,
    )
}

pub fn start_or_resume_with_strategy(
    repo_root: &Path,
    assignee: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    claim_strategy: Option<WorkClaimStrategy>,
    coordination_verbosity: Option<WorkCoordinationVerbosity>,
    format: Format,
) -> Result<()> {
    let agent = resolve_agent_identity(assignee)?;
    let response = reconcile_start_or_resume(
        repo_root,
        agent,
        tag,
        limit,
        verify_mode,
        claim_strategy,
        coordination_verbosity,
    )?;
    print_response(response, format)
}

pub fn status(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let agent = resolve_agent_identity(assignee)?;
    let response = status_response(repo_root, agent)?;
    print_response(response, format)
}

pub fn stop(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let agent = resolve_agent_identity(assignee)?;
    let response = stop_response(repo_root, agent);
    print_response(response?, format)
}

fn reconcile_start_or_resume(
    repo_root: &Path,
    agent: String,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    claim_strategy: Option<WorkClaimStrategy>,
    coordination_verbosity: Option<WorkCoordinationVerbosity>,
) -> Result<WorkResponse> {
    let work_store = WorkStore::open(&repo_root.join(".tak"));
    let mut state = work_store
        .activate(
            &agent,
            tag,
            limit,
            verify_mode,
            claim_strategy,
            coordination_verbosity,
        )?
        .state;
    let repo = Repo::open(repo_root)?;

    if let Some(task) = load_current_owned_task(&repo, &agent, state.current_task_id)? {
        let state = work_store.save(&state)?;
        return Ok(WorkResponse {
            event: WorkEvent::Continued,
            agent,
            state,
            current_task: Some(task),
        });
    }

    if state.current_task_id.is_some() {
        state.current_task_id = None;
        mark_previous_unit_processed(&mut state);
        release_reservations_best_effort(repo_root, &agent);
    }

    if state.remaining == Some(0) {
        state.active = false;
        state.current_task_id = None;
        let state = work_store.save(&state)?;
        return Ok(WorkResponse {
            event: WorkEvent::LimitReached,
            agent,
            state,
            current_task: None,
        });
    }

    if let Some(task) = find_owned_in_progress_task(&repo, &agent)? {
        state.active = true;
        state.current_task_id = Some(task.id);
        let state = work_store.save(&state)?;
        return Ok(WorkResponse {
            event: WorkEvent::Attached,
            agent,
            state,
            current_task: Some(task),
        });
    }

    if let Some(task) = crate::commands::claim::claim_next(
        repo_root,
        &agent,
        state.tag.as_deref(),
        state.claim_strategy,
    )? {
        state.active = true;
        state.current_task_id = Some(task.id);
        let state = work_store.save(&state)?;
        return Ok(WorkResponse {
            event: WorkEvent::Claimed,
            agent,
            state,
            current_task: Some(task),
        });
    }

    state.active = false;
    state.current_task_id = None;
    let state = work_store.save(&state)?;
    Ok(WorkResponse {
        event: WorkEvent::NoWork,
        agent,
        state,
        current_task: None,
    })
}

fn status_response(repo_root: &Path, agent: String) -> Result<WorkResponse> {
    let store = WorkStore::open(&repo_root.join(".tak"));
    let state = store.status(&agent)?;
    let current_task = load_task_if_exists(repo_root, state.current_task_id)?;

    Ok(WorkResponse {
        event: WorkEvent::Status,
        agent,
        state,
        current_task,
    })
}

fn stop_response(repo_root: &Path, agent: String) -> Result<WorkResponse> {
    let store = WorkStore::open(&repo_root.join(".tak"));
    let state = store.deactivate(&agent)?;
    release_reservations_best_effort(repo_root, &agent);

    Ok(WorkResponse {
        event: WorkEvent::Stopped,
        agent,
        state,
        current_task: None,
    })
}

fn load_current_owned_task(
    repo: &Repo,
    agent: &str,
    current_task_id: Option<u64>,
) -> Result<Option<Task>> {
    let Some(task_id) = current_task_id else {
        return Ok(None);
    };

    let task = match repo.store.read(task_id) {
        Ok(task) => task,
        Err(TakError::TaskNotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };

    if matches!(task.status, Status::InProgress) && task.assignee.as_deref() == Some(agent) {
        Ok(Some(task))
    } else {
        Ok(None)
    }
}

fn find_owned_in_progress_task(repo: &Repo, agent: &str) -> Result<Option<Task>> {
    let mut mine = repo
        .store
        .list_all()?
        .into_iter()
        .filter(|task| {
            matches!(task.status, Status::InProgress) && task.assignee.as_deref() == Some(agent)
        })
        .collect::<Vec<_>>();

    mine.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Ok(mine.into_iter().next())
}

fn mark_previous_unit_processed(state: &mut WorkState) {
    state.processed = state.processed.saturating_add(1);
    if let Some(remaining) = state.remaining.as_mut() {
        *remaining = remaining.saturating_sub(1);
    }
}

fn load_task_if_exists(repo_root: &Path, task_id: Option<u64>) -> Result<Option<Task>> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };

    let repo = Repo::open(repo_root)?;
    match repo.store.read(task_id) {
        Ok(task) => Ok(Some(task)),
        Err(TakError::TaskNotFound(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

fn release_reservations_best_effort(repo_root: &Path, agent: &str) {
    let mesh = MeshStore::open(&repo_root.join(".tak"));
    if !mesh.exists() {
        return;
    }

    if let Err(err) = mesh.release(agent, vec![])
        && !matches!(err, TakError::MeshAgentNotFound(_))
    {
        // best-effort cleanup only
    }
}

pub(crate) fn resolve_agent_identity(explicit_assignee: Option<String>) -> Result<String> {
    if let Some(explicit) = explicit_assignee.and_then(normalize_identity) {
        return validate_identity(explicit);
    }

    if let Some(from_env) = std::env::var("TAK_AGENT").ok().and_then(normalize_identity) {
        return validate_identity(from_env);
    }

    validate_identity(crate::agent::generated_fallback())
}

fn normalize_identity(raw: String) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn validate_identity(agent: String) -> Result<String> {
    WorkStore::validate_agent_name(&agent)?;
    Ok(agent)
}

fn print_response(response: WorkResponse, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(&response)?),
        Format::Pretty => {
            let active = if response.state.active {
                "active".green().to_string()
            } else {
                "inactive".yellow().to_string()
            };
            let task_label = response
                .current_task
                .as_ref()
                .map(|task| format!("{} {}", TaskId::from(task.id), task.title))
                .unwrap_or_else(|| "-".to_string());
            let tag = response.state.tag.as_deref().unwrap_or("-");
            let remaining = response
                .state
                .remaining
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());

            println!(
                "{} {} ({})",
                "work".cyan().bold(),
                response.event.as_str().bold(),
                response.agent.cyan()
            );
            println!("  {} {}", "state:".dimmed(), active);
            println!("  {} {}", "task:".dimmed(), task_label);
            println!("  {} {}", "tag:".dimmed(), tag);
            println!("  {} {}", "remaining:".dimmed(), remaining);
            println!("  {} {}", "processed:".dimmed(), response.state.processed);
            println!(
                "  {} {}",
                "verify:".dimmed(),
                response.state.verify_mode.to_string()
            );
            println!(
                "  {} {}",
                "strategy:".dimmed(),
                response.state.claim_strategy.to_string()
            );
            println!(
                "  {} {}",
                "verbosity:".dimmed(),
                response.state.coordination_verbosity.to_string()
            );
        }
        Format::Minimal => {
            let state = if response.state.active {
                "active"
            } else {
                "inactive"
            };
            let task_id = response
                .current_task
                .as_ref()
                .map(|task| TaskId::from(task.id).to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{}\t{}\t{}\t{}",
                response.event.as_str(),
                response.agent,
                state,
                task_id
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use chrono::Utc;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn setup_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        crate::store::files::FileStore::init(dir.path()).unwrap();
        dir
    }

    fn create_task(repo_root: &Path, title: &str) -> u64 {
        let repo = Repo::open(repo_root).unwrap();
        let task = repo
            .store
            .create(
                title.to_string(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        repo.index.upsert(&task).unwrap();
        task.id
    }

    fn mutate_task(repo_root: &Path, task_id: u64, mutator: impl FnOnce(&mut Task)) {
        let repo = Repo::open(repo_root).unwrap();
        let mut task = repo.store.read(task_id).unwrap();
        mutator(&mut task);
        task.updated_at = Utc::now();
        repo.store.write(&task).unwrap();
        repo.index.upsert(&task).unwrap();
    }

    #[test]
    fn resolve_agent_prefers_explicit_assignee_over_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TAK_AGENT", "env-agent");
        }

        let resolved = resolve_agent_identity(Some("explicit-agent".into())).unwrap();
        assert_eq!(resolved, "explicit-agent");

        unsafe {
            std::env::remove_var("TAK_AGENT");
        }
    }

    #[test]
    fn resolve_agent_uses_tak_agent_env_when_explicit_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TAK_AGENT", "env-agent");
        }

        let resolved = resolve_agent_identity(None).unwrap();
        assert_eq!(resolved, "env-agent");

        unsafe {
            std::env::remove_var("TAK_AGENT");
        }
    }

    #[test]
    fn resolve_agent_falls_back_to_generated_identity() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TAK_AGENT");
        }

        let resolved = resolve_agent_identity(None).unwrap();
        assert!(!resolved.is_empty());
        assert_eq!(resolved.split('-').count(), 3);
    }

    #[test]
    fn resolve_agent_rejects_invalid_explicit_assignee() {
        let err = resolve_agent_identity(Some("bad name".into())).unwrap_err();
        assert!(matches!(
            err,
            crate::error::TakError::WorkInvalidAgentName(name) if name == "bad name"
        ));
    }

    #[test]
    fn reconcile_claims_when_idle_and_work_is_available() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "pending");

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert_eq!(response.event, WorkEvent::Claimed);
        assert_eq!(response.state.current_task_id, Some(task_id));
        assert!(response.state.active);
        assert_eq!(
            response.current_task.as_ref().map(|task| task.id),
            Some(task_id)
        );
    }

    #[test]
    fn reconcile_continues_owned_current_task() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "active");
        mutate_task(dir.path(), task_id, |task| {
            task.status = Status::InProgress;
            task.assignee = Some("agent-1".into());
        });

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(task_id);
        store.save(&state).unwrap();

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert_eq!(response.event, WorkEvent::Continued);
        assert_eq!(response.state.current_task_id, Some(task_id));
        assert_eq!(
            response.current_task.as_ref().map(|task| task.id),
            Some(task_id)
        );
    }

    #[test]
    fn reconcile_attaches_existing_owned_in_progress_task() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "owned");
        mutate_task(dir.path(), task_id, |task| {
            task.status = Status::InProgress;
            task.assignee = Some("agent-1".into());
        });

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert_eq!(response.event, WorkEvent::Attached);
        assert_eq!(response.state.current_task_id, Some(task_id));
        assert_eq!(
            response.current_task.as_ref().map(|task| task.id),
            Some(task_id)
        );
    }

    #[test]
    fn reconcile_deactivates_when_no_work_is_available() {
        let dir = setup_repo();
        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert_eq!(response.event, WorkEvent::NoWork);
        assert!(!response.state.active);
        assert!(response.state.current_task_id.is_none());
        assert!(response.current_task.is_none());
    }

    #[test]
    fn reconcile_hits_limit_after_previous_unit_processed() {
        let dir = setup_repo();
        let finished_task_id = create_task(dir.path(), "finished");
        mutate_task(dir.path(), finished_task_id, |task| {
            task.status = Status::Done;
            task.assignee = Some("agent-1".into());
        });

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, Some(1), None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(finished_task_id);
        store.save(&state).unwrap();

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert_eq!(response.event, WorkEvent::LimitReached);
        assert!(!response.state.active);
        assert_eq!(response.state.remaining, Some(0));
        assert_eq!(response.state.processed, 1);
        assert!(response.current_task.is_none());
    }

    #[test]
    fn repeated_start_without_assignee_uses_same_tak_agent_key() {
        let _guard = ENV_LOCK.lock().unwrap();

        let dir = setup_repo();

        unsafe {
            std::env::set_var("TAK_AGENT", "stable-agent");
        }

        start_or_resume_with_strategy(
            dir.path(),
            None,
            Some("cli".into()),
            Some(2),
            None,
            Some(WorkClaimStrategy::EpicCloseout),
            Some(WorkCoordinationVerbosity::High),
            Format::Minimal,
        )
        .unwrap();
        start_or_resume(dir.path(), None, None, None, None, Format::Minimal).unwrap();

        let state_path = dir
            .path()
            .join(".tak")
            .join("runtime")
            .join("work")
            .join("states")
            .join("stable-agent.json");

        assert!(state_path.exists());

        let raw = fs::read_to_string(&state_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value.get("agent").and_then(|v| v.as_str()),
            Some("stable-agent")
        );
        assert_eq!(
            value.get("claim_strategy").and_then(|v| v.as_str()),
            Some("priority_then_age")
        );
        assert_eq!(
            value.get("coordination_verbosity").and_then(|v| v.as_str()),
            Some("medium")
        );

        unsafe {
            std::env::remove_var("TAK_AGENT");
        }
    }
}
