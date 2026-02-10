use std::path::Path;

use colored::Colorize;
use serde::{Deserialize, Serialize};

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
    Done,
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
            Self::Done => "done",
            Self::NoWork => "no_work",
            Self::LimitReached => "limit_reached",
            Self::Status => "status",
            Self::Stopped => "stopped",
        }
    }
}

fn is_false(v: &bool) -> bool {
    !v
}

#[derive(Debug, Serialize)]
struct WorkResponse {
    event: WorkEvent,
    agent: String,
    /// True when the agent identity was auto-generated (no --assignee, no TAK_AGENT).
    /// Loop state keyed to an ephemeral identity won't persist across invocations.
    #[serde(default, skip_serializing_if = "is_false")]
    ephemeral_identity: bool,
    #[serde(rename = "loop")]
    state: WorkState,
    current_task: Option<Task>,
    #[serde(default)]
    reservations: Vec<String>,
    #[serde(default)]
    blockers: Vec<String>,
    suggested_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<WorkDoneSummary>,
}

#[derive(Debug, Serialize)]
struct ReservationReleaseSummary {
    released: bool,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkDoneSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_task_id: Option<u64>,
    lifecycle_transition: String,
    reservation_release: ReservationReleaseSummary,
    paused: bool,
    loop_active: bool,
}

const RESUME_GATE_EXTENSION_KEY: &str = "resume_gate";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResumeGateReason {
    Handoff,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResumeGate {
    task_id: u64,
    reason: ResumeGateReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_dep_ids: Vec<u64>,
    #[serde(default)]
    handoff_skip_remaining: u8,
}

enum ResumeGateDecision {
    Hold {
        gate: ResumeGate,
        blocker: String,
        suggested_action: String,
    },
    Clear,
}

fn load_resume_gate(state: &WorkState) -> Option<ResumeGate> {
    state
        .extensions
        .get(RESUME_GATE_EXTENSION_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn store_resume_gate(state: &mut WorkState, gate: Option<ResumeGate>) {
    if let Some(gate) = gate {
        state.extensions.insert(
            RESUME_GATE_EXTENSION_KEY.to_string(),
            serde_json::to_value(gate).unwrap_or(serde_json::Value::Null),
        );
    } else {
        state.extensions.remove(RESUME_GATE_EXTENSION_KEY);
    }
}

fn unresolved_dependency_ids(repo: &Repo, task: &Task) -> Result<Vec<u64>> {
    let mut unresolved = Vec::new();

    for dep in &task.depends_on {
        match repo.store.read(dep.id) {
            Ok(dep_task) => {
                if !matches!(dep_task.status, Status::Done | Status::Cancelled) {
                    unresolved.push(dep.id);
                }
            }
            Err(TakError::TaskNotFound(_)) => unresolved.push(dep.id),
            Err(err) => return Err(err),
        }
    }

    unresolved.sort_unstable();
    unresolved.dedup();
    Ok(unresolved)
}

fn evaluate_resume_gate(repo: &Repo, gate: ResumeGate) -> Result<ResumeGateDecision> {
    let task = match repo.store.read(gate.task_id) {
        Ok(task) => task,
        Err(TakError::TaskNotFound(_)) => return Ok(ResumeGateDecision::Clear),
        Err(err) => return Err(err),
    };

    match gate.reason {
        ResumeGateReason::Blocked => {
            if !matches!(task.status, Status::Pending) {
                return Ok(ResumeGateDecision::Clear);
            }

            let current_unresolved = unresolved_dependency_ids(repo, &task)?;
            if current_unresolved.is_empty() || current_unresolved != gate.blocked_dep_ids {
                return Ok(ResumeGateDecision::Clear);
            }

            let blocker = format!(
                "task {} still blocked by deps: {}",
                TaskId::from(task.id),
                current_unresolved
                    .iter()
                    .map(|id| TaskId::from(*id).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(ResumeGateDecision::Hold {
                gate,
                blocker,
                suggested_action: "wait for dependency change or use --force-reclaim".into(),
            })
        }
        ResumeGateReason::Handoff => {
            if gate.handoff_skip_remaining == 0 {
                return Ok(ResumeGateDecision::Clear);
            }

            let mut updated = gate;
            updated.handoff_skip_remaining = updated.handoff_skip_remaining.saturating_sub(1);
            Ok(ResumeGateDecision::Hold {
                gate: updated,
                blocker: format!(
                    "recent handoff from task {}; skipping one reclaim cycle",
                    TaskId::from(task.id)
                ),
                suggested_action:
                    "rerun `tak work` after updating context/blackboard or use --force-reclaim"
                        .into(),
            })
        }
    }
}

fn list_agent_reservations_best_effort(repo_root: &Path, agent: &str) -> Vec<String> {
    let mesh = MeshStore::open(&repo_root.join(".tak"));
    if !mesh.exists() {
        return vec![];
    }

    let Ok(reservations) = mesh.list_reservations() else {
        return vec![];
    };

    let mut paths = reservations
        .into_iter()
        .filter(|reservation| reservation.agent == agent)
        .flat_map(|reservation| reservation.paths)
        .collect::<Vec<_>>();

    paths.sort();
    paths.dedup();
    paths
}

fn build_response(
    repo_root: &Path,
    event: WorkEvent,
    agent: String,
    state: WorkState,
    current_task: Option<Task>,
    blockers: Vec<String>,
    suggested_action: String,
) -> WorkResponse {
    WorkResponse {
        event,
        reservations: list_agent_reservations_best_effort(repo_root, &agent),
        blockers,
        suggested_action,
        agent,
        ephemeral_identity: false,
        state,
        current_task,
        done: None,
    }
}

pub fn start_or_resume(
    repo_root: &Path,
    assignee: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    format: Format,
) -> Result<()> {
    start_or_resume_with_strategy_force(
        repo_root,
        assignee,
        tag,
        limit,
        verify_mode,
        None,
        None,
        false,
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
    start_or_resume_with_strategy_force(
        repo_root,
        assignee,
        tag,
        limit,
        verify_mode,
        claim_strategy,
        coordination_verbosity,
        false,
        format,
    )
}

pub fn start_or_resume_with_strategy_force(
    repo_root: &Path,
    assignee: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    claim_strategy: Option<WorkClaimStrategy>,
    coordination_verbosity: Option<WorkCoordinationVerbosity>,
    force_reclaim: bool,
    format: Format,
) -> Result<()> {
    let resolved = resolve_agent_identity(assignee)?;
    let mut response = reconcile_start_or_resume_with_force(
        repo_root,
        resolved.name,
        tag,
        limit,
        verify_mode,
        claim_strategy,
        coordination_verbosity,
        force_reclaim,
    )?;
    response.ephemeral_identity = resolved.ephemeral;
    print_response(response, format)
}

pub fn status(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let resolved = resolve_agent_identity(assignee)?;
    let mut response = status_response(repo_root, resolved.name)?;
    response.ephemeral_identity = resolved.ephemeral;
    print_response(response, format)
}

pub fn stop(repo_root: &Path, assignee: Option<String>, format: Format) -> Result<()> {
    let resolved = resolve_agent_identity(assignee)?;
    let mut response = stop_response(repo_root, resolved.name)?;
    response.ephemeral_identity = resolved.ephemeral;
    print_response(response, format)
}

pub fn done(repo_root: &Path, assignee: Option<String>, pause: bool, format: Format) -> Result<()> {
    let resolved = resolve_agent_identity(assignee)?;
    let mut response = done_response(repo_root, resolved.name, pause)?;
    response.ephemeral_identity = resolved.ephemeral;
    print_response(response, format)
}

#[cfg(test)]
fn reconcile_start_or_resume(
    repo_root: &Path,
    agent: String,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    claim_strategy: Option<WorkClaimStrategy>,
    coordination_verbosity: Option<WorkCoordinationVerbosity>,
) -> Result<WorkResponse> {
    reconcile_start_or_resume_with_force(
        repo_root,
        agent,
        tag,
        limit,
        verify_mode,
        claim_strategy,
        coordination_verbosity,
        false,
    )
}

fn reconcile_start_or_resume_with_force(
    repo_root: &Path,
    agent: String,
    tag: Option<String>,
    limit: Option<u32>,
    verify_mode: Option<WorkVerifyMode>,
    claim_strategy: Option<WorkClaimStrategy>,
    coordination_verbosity: Option<WorkCoordinationVerbosity>,
    force_reclaim: bool,
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
        store_resume_gate(&mut state, None);
        let state = work_store.save(&state)?;
        let task_id = TaskId::from(task.id);
        return Ok(build_response(
            repo_root,
            WorkEvent::Continued,
            agent,
            state,
            Some(task),
            vec![],
            format!("continue working on {task_id}"),
        ));
    }

    if let Some(previous_task_id) = state.current_task_id {
        state.current_task_id = None;
        mark_previous_unit_processed(&mut state);
        release_reservations_best_effort(repo_root, &agent);

        let previous_task = match repo.store.read(previous_task_id) {
            Ok(task) => Some(task),
            Err(TakError::TaskNotFound(_)) => None,
            Err(err) => return Err(err),
        };

        let gate = if let Some(task) = previous_task {
            if matches!(task.status, Status::Pending) {
                let unresolved = unresolved_dependency_ids(&repo, &task)?;
                if !unresolved.is_empty() {
                    Some(ResumeGate {
                        task_id: task.id,
                        reason: ResumeGateReason::Blocked,
                        blocked_dep_ids: unresolved,
                        handoff_skip_remaining: 0,
                    })
                } else if task.assignee.is_none() {
                    Some(ResumeGate {
                        task_id: task.id,
                        reason: ResumeGateReason::Handoff,
                        blocked_dep_ids: vec![],
                        handoff_skip_remaining: 1,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        store_resume_gate(&mut state, gate);
    }

    if state.remaining == Some(0) {
        state.active = false;
        state.current_task_id = None;
        store_resume_gate(&mut state, None);
        let state = work_store.save(&state)?;
        return Ok(build_response(
            repo_root,
            WorkEvent::LimitReached,
            agent,
            state,
            None,
            vec![],
            "work-loop limit reached; run `tak work start --limit <n>` to continue".into(),
        ));
    }

    if let Some(task) = find_owned_in_progress_task(&repo, &agent)? {
        state.active = true;
        state.current_task_id = Some(task.id);
        store_resume_gate(&mut state, None);
        let state = work_store.save(&state)?;
        let task_id = TaskId::from(task.id);
        return Ok(build_response(
            repo_root,
            WorkEvent::Attached,
            agent,
            state,
            Some(task),
            vec![],
            format!("resume attached task {task_id}"),
        ));
    }

    if force_reclaim {
        store_resume_gate(&mut state, None);
    } else if let Some(gate) = load_resume_gate(&state) {
        match evaluate_resume_gate(&repo, gate)? {
            ResumeGateDecision::Hold {
                gate,
                blocker,
                suggested_action,
            } => {
                store_resume_gate(&mut state, Some(gate));
                state.active = true;
                state.current_task_id = None;
                let state = work_store.save(&state)?;
                return Ok(build_response(
                    repo_root,
                    WorkEvent::NoWork,
                    agent,
                    state,
                    None,
                    vec![blocker],
                    suggested_action,
                ));
            }
            ResumeGateDecision::Clear => store_resume_gate(&mut state, None),
        }
    }

    if let Some(task) = crate::commands::claim::claim_next(
        repo_root,
        &agent,
        state.tag.as_deref(),
        state.claim_strategy,
    )? {
        state.active = true;
        state.current_task_id = Some(task.id);
        store_resume_gate(&mut state, None);
        let state = work_store.save(&state)?;
        let task_id = TaskId::from(task.id);
        return Ok(build_response(
            repo_root,
            WorkEvent::Claimed,
            agent,
            state,
            Some(task),
            vec![],
            format!("start claimed task {task_id}"),
        ));
    }

    state.active = false;
    state.current_task_id = None;
    let state = work_store.save(&state)?;
    Ok(build_response(
        repo_root,
        WorkEvent::NoWork,
        agent,
        state,
        None,
        vec![],
        "no available work; run `tak next` or adjust filters".into(),
    ))
}

fn status_response(repo_root: &Path, agent: String) -> Result<WorkResponse> {
    let store = WorkStore::open(&repo_root.join(".tak"));
    let state = store.status(&agent)?;
    let current_task = load_task_if_exists(repo_root, state.current_task_id)?;
    let repo = Repo::open(repo_root)?;

    let mut blockers = vec![];
    let suggested_action = if let Some(task) = current_task.as_ref() {
        format!("continue working on {}", TaskId::from(task.id))
    } else if let Some(gate) = load_resume_gate(&state) {
        match evaluate_resume_gate(&repo, gate)? {
            ResumeGateDecision::Hold {
                blocker,
                suggested_action,
                ..
            } => {
                blockers.push(blocker);
                suggested_action
            }
            ResumeGateDecision::Clear => {
                if state.active {
                    "run `tak work` to attach or claim available work".into()
                } else {
                    "run `tak work start` to activate loop".into()
                }
            }
        }
    } else if state.active {
        "run `tak work` to attach or claim available work".into()
    } else {
        "run `tak work start` to activate loop".into()
    };

    Ok(build_response(
        repo_root,
        WorkEvent::Status,
        agent,
        state,
        current_task,
        blockers,
        suggested_action,
    ))
}

fn stop_response(repo_root: &Path, agent: String) -> Result<WorkResponse> {
    let store = WorkStore::open(&repo_root.join(".tak"));
    let state = store.deactivate(&agent)?;
    release_reservations_best_effort(repo_root, &agent);

    Ok(build_response(
        repo_root,
        WorkEvent::Stopped,
        agent,
        state,
        None,
        vec![],
        "run `tak work start` to resume loop".into(),
    ))
}

fn done_response(repo_root: &Path, agent: String, pause: bool) -> Result<WorkResponse> {
    let work_store = WorkStore::open(&repo_root.join(".tak"));
    let mut state = work_store.status(&agent)?;
    let repo = Repo::open(repo_root)?;

    let mut finished_task_id = None;
    let lifecycle_transition =
        if let Some(task) = load_current_owned_task(&repo, &agent, state.current_task_id)? {
            crate::commands::lifecycle::finish_task(repo_root, task.id)?;
            state.current_task_id = None;
            mark_previous_unit_processed(&mut state);
            finished_task_id = Some(task.id);
            "finished".to_string()
        } else if state.current_task_id.is_some() {
            state.current_task_id = None;
            "detached_without_finish".to_string()
        } else {
            "no_current_task".to_string()
        };

    store_resume_gate(&mut state, None);

    let release_summary = release_agent_reservations(repo_root, &agent);

    if pause {
        state.active = false;
    } else if finished_task_id.is_some() {
        state.active = true;
    }

    if !state.active {
        state.current_task_id = None;
    }

    let state = work_store.save(&state)?;

    let suggested_action = if pause {
        "loop paused; run `tak work start` when ready".to_string()
    } else if state.active {
        "run `tak work` to claim the next task".to_string()
    } else {
        "run `tak work start` to activate loop".to_string()
    };

    let mut response = build_response(
        repo_root,
        WorkEvent::Done,
        agent,
        state,
        None,
        vec![],
        suggested_action,
    );
    response.done = Some(WorkDoneSummary {
        finished_task_id,
        lifecycle_transition,
        reservation_release: release_summary,
        paused: pause,
        loop_active: response.state.active,
    });

    Ok(response)
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
    let ids = repo.index.tasks_by_status_assignee("in_progress", agent)?;
    match ids.first() {
        Some(id) => Ok(Some(repo.store.read(u64::from(id))?)),
        None => Ok(None),
    }
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

fn release_agent_reservations(repo_root: &Path, agent: &str) -> ReservationReleaseSummary {
    let mesh = MeshStore::open(&repo_root.join(".tak"));
    if !mesh.exists() {
        return ReservationReleaseSummary {
            released: true,
            paths: vec![],
            error: None,
        };
    }

    let paths = list_agent_reservations_best_effort(repo_root, agent);
    match mesh.release(agent, vec![]) {
        Ok(_) => ReservationReleaseSummary {
            released: true,
            paths,
            error: None,
        },
        Err(TakError::MeshAgentNotFound(_)) => ReservationReleaseSummary {
            released: true,
            paths: vec![],
            error: None,
        },
        Err(err) => ReservationReleaseSummary {
            released: false,
            paths,
            error: Some(err.to_string()),
        },
    }
}

fn release_reservations_best_effort(repo_root: &Path, agent: &str) {
    let _ = release_agent_reservations(repo_root, agent);
}

/// Resolved agent identity with provenance metadata.
#[derive(Debug)]
pub(crate) struct ResolvedAgent {
    pub name: String,
    /// True when the identity was generated on-the-fly (no `--assignee`, no `TAK_AGENT`).
    /// Loop state keyed to an ephemeral identity won't survive across CLI invocations.
    pub ephemeral: bool,
}

pub(crate) fn resolve_agent_identity(explicit_assignee: Option<String>) -> Result<ResolvedAgent> {
    if let Some(explicit) = explicit_assignee.and_then(normalize_identity) {
        return Ok(ResolvedAgent {
            name: validate_identity(explicit)?,
            ephemeral: false,
        });
    }

    if let Some(from_env) = std::env::var("TAK_AGENT").ok().and_then(normalize_identity) {
        return Ok(ResolvedAgent {
            name: validate_identity(from_env)?,
            ephemeral: false,
        });
    }

    Ok(ResolvedAgent {
        name: validate_identity(crate::agent::generated_fallback())?,
        ephemeral: true,
    })
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

fn render_response(response: &WorkResponse, format: Format) -> Result<String> {
    let rendered = match format {
        Format::Json => serde_json::to_string(response)?,
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

            let reservations = if response.reservations.is_empty() {
                "-".to_string()
            } else {
                response.reservations.join(", ")
            };
            let blockers = if response.blockers.is_empty() {
                "-".to_string()
            } else {
                response.blockers.join(" | ")
            };

            let mut out = format!(
                "{} {} ({})\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}",
                "work".cyan().bold(),
                response.event.as_str().bold(),
                response.agent.cyan(),
                "state:".dimmed(),
                active,
                "task:".dimmed(),
                task_label,
                "tag:".dimmed(),
                tag,
                "remaining:".dimmed(),
                remaining,
                "processed:".dimmed(),
                response.state.processed,
                "verify:".dimmed(),
                response.state.verify_mode,
                "strategy:".dimmed(),
                response.state.claim_strategy,
                "verbosity:".dimmed(),
                response.state.coordination_verbosity,
                "reservations:".dimmed(),
                reservations,
                "blockers:".dimmed(),
                blockers,
                "next:".dimmed(),
                response.suggested_action
            );
            if let Some(done) = response.done.as_ref() {
                let finished = done
                    .finished_task_id
                    .map(TaskId::from)
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let released = if done.reservation_release.paths.is_empty() {
                    "-".to_string()
                } else {
                    done.reservation_release.paths.join(", ")
                };
                let release_status = if done.reservation_release.released {
                    "ok".green().to_string()
                } else {
                    "failed".red().to_string()
                };
                out.push_str(&format!(
                    "\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}",
                    "lifecycle:".dimmed(),
                    done.lifecycle_transition,
                    "finished task:".dimmed(),
                    finished,
                    "release:".dimmed(),
                    release_status,
                    "released paths:".dimmed(),
                    released,
                    "paused:".dimmed(),
                    done.paused
                ));
                if let Some(error) = done.reservation_release.error.as_ref() {
                    out.push_str(&format!("\n  {} {}", "release error:".red().bold(), error));
                }
            }
            if response.ephemeral_identity {
                out.push_str(&format!(
                    "\n  {} {}",
                    "warning:".yellow().bold(),
                    "ephemeral identity; set TAK_AGENT or --assignee for durable loop state"
                ));
            }
            out
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
            format!(
                "{}\t{}\t{}\t{}",
                response.event.as_str(),
                response.agent,
                state,
                task_id
            )
        }
    };

    Ok(rendered)
}

fn print_response(response: WorkResponse, format: Format) -> Result<()> {
    println!("{}", render_response(&response, format)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use crate::store::mesh::MeshStore;
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

    fn create_task_with_deps(repo_root: &Path, title: &str, depends_on: Vec<u64>) -> u64 {
        let repo = Repo::open(repo_root).unwrap();
        let task = repo
            .store
            .create(
                title.to_string(),
                Kind::Task,
                None,
                None,
                depends_on,
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
        assert_eq!(resolved.name, "explicit-agent");
        assert!(!resolved.ephemeral);

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
        assert_eq!(resolved.name, "env-agent");
        assert!(!resolved.ephemeral);

        unsafe {
            std::env::remove_var("TAK_AGENT");
        }
    }

    #[test]
    fn resolve_agent_falls_back_to_generated_identity_and_marks_ephemeral() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TAK_AGENT");
        }

        let resolved = resolve_agent_identity(None).unwrap();
        assert!(!resolved.name.is_empty());
        assert_eq!(resolved.name.split('-').count(), 3);
        assert!(resolved.ephemeral);
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
    fn reconcile_handoff_gate_skips_one_immediate_reclaim_cycle() {
        let dir = setup_repo();
        let handed_off_task_id = create_task(dir.path(), "handoff-ready");
        let _other_task_id = create_task(dir.path(), "next-task");

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(handed_off_task_id);
        store.save(&state).unwrap();

        let first =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();
        assert_eq!(first.event, WorkEvent::NoWork);
        assert!(first.current_task.is_none());
        assert!(
            first
                .blockers
                .iter()
                .any(|line| line.contains("recent handoff"))
        );
        assert!(first.suggested_action.contains("--force-reclaim"));

        let second =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();
        assert_eq!(second.event, WorkEvent::Claimed);
        assert!(second.current_task.is_some());
    }

    #[test]
    fn reconcile_blocked_gate_waits_until_dependency_predicate_changes() {
        let dir = setup_repo();
        let dep_id = create_task(dir.path(), "dep");
        let blocked_id = create_task_with_deps(dir.path(), "blocked", vec![dep_id]);

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(blocked_id);
        store.save(&state).unwrap();

        let first =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();
        assert_eq!(first.event, WorkEvent::NoWork);
        assert!(first.current_task.is_none());
        assert!(
            first
                .blockers
                .iter()
                .any(|line| line.contains("still blocked by deps"))
        );

        mutate_task(dir.path(), dep_id, |task| {
            task.status = Status::Done;
        });

        let second =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();
        assert_eq!(second.event, WorkEvent::Claimed);
        assert_eq!(
            second.current_task.as_ref().map(|task| task.id),
            Some(blocked_id)
        );
    }

    #[test]
    fn reconcile_force_reclaim_bypasses_resume_gate() {
        let dir = setup_repo();
        let handed_off_task_id = create_task(dir.path(), "handoff-ready");

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(handed_off_task_id);
        store.save(&state).unwrap();

        let response = reconcile_start_or_resume_with_force(
            dir.path(),
            "agent-1".into(),
            None,
            None,
            None,
            None,
            None,
            true,
        )
        .unwrap();

        assert_eq!(response.event, WorkEvent::Claimed);
        assert_eq!(
            response.current_task.as_ref().map(|task| task.id),
            Some(handed_off_task_id)
        );
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

    #[test]
    fn status_reports_active_state_and_current_task_payload() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "status-current-task");
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

        let response = status_response(dir.path(), "agent-1".into()).unwrap();

        assert_eq!(response.event, WorkEvent::Status);
        assert!(response.state.active);
        assert_eq!(response.state.current_task_id, Some(task_id));
        assert_eq!(
            response.current_task.as_ref().map(|task| task.id),
            Some(task_id)
        );
    }

    #[test]
    fn status_includes_reservations_and_next_action_snapshot() {
        let dir = setup_repo();

        let store = WorkStore::open(&dir.path().join(".tak"));
        store
            .activate("agent-1", None, None, None, None, None)
            .unwrap();

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
        mesh.reserve(
            "agent-1",
            vec!["src/commands/work.rs".into()],
            Some("status-snapshot"),
        )
        .unwrap();

        let response = status_response(dir.path(), "agent-1".into()).unwrap();

        assert_eq!(response.event, WorkEvent::Status);
        assert_eq!(response.reservations, vec!["src/commands/work.rs"]);
        assert!(response.suggested_action.contains("run `tak work`"));
    }

    #[test]
    fn done_finishes_current_task_releases_reservations_and_reports_subactions() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "done-current-task");
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

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
        mesh.reserve(
            "agent-1",
            vec!["src/commands/work.rs".into()],
            Some("test-work-done"),
        )
        .unwrap();

        let response = done_response(dir.path(), "agent-1".into(), false).unwrap();

        assert_eq!(response.event, WorkEvent::Done);
        assert!(response.state.active);
        assert!(response.state.current_task_id.is_none());
        assert_eq!(response.state.processed, 1);

        let done = response.done.as_ref().unwrap();
        assert_eq!(done.finished_task_id, Some(task_id));
        assert_eq!(done.lifecycle_transition, "finished");
        assert!(done.reservation_release.released);
        assert_eq!(done.reservation_release.paths, vec!["src/commands/work.rs"]);
        assert!(!done.paused);
        assert!(done.loop_active);

        let repo = Repo::open(dir.path()).unwrap();
        let finished_task = repo.store.read(task_id).unwrap();
        assert_eq!(finished_task.status, Status::Done);

        let reservations = mesh.list_reservations().unwrap();
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation.agent != "agent-1")
        );
    }

    #[test]
    fn done_with_pause_deactivates_loop() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "done-pause");
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

        let response = done_response(dir.path(), "agent-1".into(), true).unwrap();

        assert_eq!(response.event, WorkEvent::Done);
        assert!(!response.state.active);
        let done = response.done.as_ref().unwrap();
        assert!(done.paused);
        assert!(!done.loop_active);
    }

    #[test]
    fn done_is_idempotent_when_no_current_task_is_attached() {
        let dir = setup_repo();

        let store = WorkStore::open(&dir.path().join(".tak"));
        store
            .activate("agent-1", None, None, None, None, None)
            .unwrap();

        let first = done_response(dir.path(), "agent-1".into(), false).unwrap();
        let second = done_response(dir.path(), "agent-1".into(), false).unwrap();

        for response in [first, second] {
            assert_eq!(response.event, WorkEvent::Done);
            let done = response.done.as_ref().unwrap();
            assert_eq!(done.lifecycle_transition, "no_current_task");
            assert!(done.finished_task_id.is_none());
        }
    }

    #[test]
    fn done_reports_detached_transition_when_current_task_is_not_owned_in_progress() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "detached-current-task");

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(task_id);
        store.save(&state).unwrap();

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
        mesh.reserve(
            "agent-1",
            vec!["src/commands/work.rs".into()],
            Some("test-work-done-detached"),
        )
        .unwrap();

        let response = done_response(dir.path(), "agent-1".into(), false).unwrap();

        assert_eq!(response.event, WorkEvent::Done);
        let done = response.done.as_ref().unwrap();
        assert_eq!(done.lifecycle_transition, "detached_without_finish");
        assert!(done.finished_task_id.is_none());
        assert!(done.reservation_release.released);
        assert_eq!(done.reservation_release.paths, vec!["src/commands/work.rs"]);

        let repo = Repo::open(dir.path()).unwrap();
        let task = repo.store.read(task_id).unwrap();
        assert_eq!(task.status, Status::Pending);

        let reservations = mesh.list_reservations().unwrap();
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation.agent != "agent-1")
        );
    }

    #[test]
    fn stop_is_idempotent_and_releases_agent_reservations() {
        let dir = setup_repo();
        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
        mesh.reserve(
            "agent-1",
            vec!["src/commands/work.rs".into()],
            Some("test-stop"),
        )
        .unwrap();

        let first = stop_response(dir.path(), "agent-1".into()).unwrap();
        assert_eq!(first.event, WorkEvent::Stopped);
        assert!(!first.state.active);

        let reservations = mesh.list_reservations().unwrap();
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation.agent != "agent-1")
        );

        let second = stop_response(dir.path(), "agent-1".into()).unwrap();
        assert_eq!(second.event, WorkEvent::Stopped);
        assert!(!second.state.active);
    }

    #[test]
    fn reconcile_releases_reservations_when_current_task_is_completed() {
        let dir = setup_repo();
        let finished_task_id = create_task(dir.path(), "completed-current-task");
        mutate_task(dir.path(), finished_task_id, |task| {
            task.status = Status::Done;
            task.assignee = Some("agent-1".into());
        });

        let store = WorkStore::open(&dir.path().join(".tak"));
        let mut state = store
            .activate("agent-1", None, None, None, None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(finished_task_id);
        store.save(&state).unwrap();

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
        mesh.reserve(
            "agent-1",
            vec!["src/commands/work.rs".into()],
            Some("test-reconcile-release"),
        )
        .unwrap();

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        assert!(matches!(
            response.event,
            WorkEvent::NoWork | WorkEvent::Claimed
        ));
        assert!(response.state.current_task_id != Some(finished_task_id));

        let reservations = mesh.list_reservations().unwrap();
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation.agent != "agent-1")
        );
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\u{1b}'
                && let Some('[') = chars.peek()
            {
                chars.next();
                for code in chars.by_ref() {
                    if code.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }

            out.push(ch);
        }

        out
    }

    #[test]
    fn render_json_output_contract_contains_expected_fields() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "render-json");

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        let rendered = render_response(&response, Format::Json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(value.get("event").and_then(|v| v.as_str()), Some("claimed"));
        assert_eq!(value.get("agent").and_then(|v| v.as_str()), Some("agent-1"));

        let loop_state = value.get("loop").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            loop_state.get("active").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            loop_state.get("current_task_id").and_then(|v| v.as_u64()),
            Some(task_id)
        );

        let current_task = value
            .get("current_task")
            .and_then(|v| v.as_object())
            .unwrap();
        assert_eq!(
            current_task.get("id").and_then(|v| v.as_u64()),
            Some(task_id)
        );

        assert!(
            value
                .get("reservations")
                .and_then(|v| v.as_array())
                .is_some()
        );
        assert!(value.get("blockers").and_then(|v| v.as_array()).is_some());
        let expected = format!("start claimed task {}", TaskId::from(task_id));
        assert_eq!(
            value.get("suggested_action").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );
    }

    #[test]
    fn render_json_done_output_includes_subaction_report() {
        let now = Utc::now();
        let response = WorkResponse {
            event: WorkEvent::Done,
            agent: "agent-1".into(),
            ephemeral_identity: false,
            state: WorkState::inactive("agent-1", now),
            current_task: None,
            reservations: vec![],
            blockers: vec![],
            suggested_action: "run `tak work` to claim the next task".into(),
            done: Some(WorkDoneSummary {
                finished_task_id: Some(42),
                lifecycle_transition: "finished".into(),
                reservation_release: ReservationReleaseSummary {
                    released: true,
                    paths: vec!["src/commands/work.rs".into()],
                    error: None,
                },
                paused: false,
                loop_active: true,
            }),
        };

        let rendered = render_response(&response, Format::Json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        let done = value.get("done").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            done.get("lifecycle_transition").and_then(|v| v.as_str()),
            Some("finished")
        );
        assert_eq!(
            done.get("finished_task_id").and_then(|v| v.as_u64()),
            Some(42)
        );
        assert_eq!(done.get("paused").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            done.get("loop_active").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn render_pretty_output_is_action_oriented_and_stable() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "render-pretty");

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        let rendered = render_response(&response, Format::Pretty).unwrap();
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("work claimed (agent-1)"));
        assert!(plain.contains("state: active"));
        assert!(plain.contains(&format!("task: {} render-pretty", TaskId::from(task_id))));
        assert!(plain.contains("tag: -"));
        assert!(plain.contains("verify: isolated"));
        assert!(plain.contains("strategy: priority_then_age"));
        assert!(plain.contains("verbosity: medium"));
        assert!(plain.contains("reservations: -"));
        assert!(plain.contains("blockers: -"));
        assert!(plain.contains("next: start claimed task"));
    }

    #[test]
    fn render_minimal_output_is_tab_separated_and_script_friendly() {
        let dir = setup_repo();
        let task_id = create_task(dir.path(), "render-minimal");

        let response =
            reconcile_start_or_resume(dir.path(), "agent-1".into(), None, None, None, None, None)
                .unwrap();

        let rendered = render_response(&response, Format::Minimal).unwrap();
        let parts = rendered.split('\t').collect::<Vec<_>>();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "claimed");
        assert_eq!(parts[1], "agent-1");
        assert_eq!(parts[2], "active");
        assert_eq!(parts[3], TaskId::from(task_id).to_string());
    }
}
