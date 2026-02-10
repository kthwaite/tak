use std::path::Path;

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::model::Status;
use crate::output::Format;
use crate::store::coordination::CoordinationLinks;
use crate::store::lock;
use crate::store::mesh::MeshStore;
use crate::store::repo::Repo;
use crate::store::sidecars::HistoryEvent;
use crate::store::work::WorkStore;
use crate::task_id::TaskId;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DecisionPath {
    AlreadyOwner,
    Forced,
    OwnerInactive,
    OwnerNotRegistered,
    MeshUnavailable,
}

impl DecisionPath {
    fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyOwner => "already_owner",
            Self::Forced => "forced",
            Self::OwnerInactive => "owner_inactive",
            Self::OwnerNotRegistered => "owner_not_registered",
            Self::MeshUnavailable => "mesh_unavailable",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TakeoverResponse {
    event: &'static str,
    task_id: u64,
    previous_owner: String,
    new_owner: String,
    decision: DecisionPath,
    forced: bool,
    threshold_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_inactive_secs: Option<i64>,
    resulting_status: Status,
    resulting_assignee: Option<String>,
}

enum OwnerActivity {
    InactiveFor(i64),
    OwnerNotRegistered,
    MeshUnavailable,
}

pub fn run(
    repo_root: &Path,
    task_id: u64,
    assignee: String,
    inactive_secs: Option<u64>,
    force: bool,
    format: Format,
) -> Result<()> {
    let lock_path = repo_root.join(".tak").join("takeover.lock");
    let lock_file = lock::acquire_lock(&lock_path)?;
    let result = takeover_response(repo_root, task_id, assignee, inactive_secs, force);
    lock::release_lock(lock_file)?;

    let response = result?;
    print_response(&response, format)
}

fn takeover_response(
    repo_root: &Path,
    task_id: u64,
    assignee: String,
    inactive_secs: Option<u64>,
    force: bool,
) -> Result<TakeoverResponse> {
    WorkStore::validate_agent_name(&assignee)?;

    let repo = Repo::open(repo_root)?;
    let mut task = repo.store.read(task_id)?;

    if !matches!(task.status, Status::InProgress) {
        return Err(TakError::InvalidTransition(
            task.status.to_string(),
            Status::InProgress.to_string(),
        ));
    }

    let Some(previous_owner) = task.assignee.clone() else {
        return Err(TakError::Locked(format!(
            "takeover rejected: task {} has no assignee",
            TaskId::from(task.id)
        )));
    };

    let threshold_secs = inactive_secs.unwrap_or_else(|| {
        MeshStore::open(&repo_root.join(".tak"))
            .lease_config()
            .registration_ttl_secs
    });

    let activity = owner_activity(repo_root, &previous_owner)?;
    let owner_inactive_secs = match activity {
        OwnerActivity::InactiveFor(secs) => Some(secs),
        OwnerActivity::OwnerNotRegistered | OwnerActivity::MeshUnavailable => None,
    };

    let decision = if assignee == previous_owner {
        DecisionPath::AlreadyOwner
    } else if force {
        DecisionPath::Forced
    } else {
        match activity {
            OwnerActivity::InactiveFor(secs) if secs >= threshold_secs as i64 => {
                DecisionPath::OwnerInactive
            }
            OwnerActivity::InactiveFor(secs) => {
                return Err(TakError::Locked(format!(
                    "takeover rejected: owner '{previous_owner}' is active (inactive {secs}s < threshold {threshold_secs}s); retry later or pass --force"
                )));
            }
            OwnerActivity::OwnerNotRegistered => DecisionPath::OwnerNotRegistered,
            OwnerActivity::MeshUnavailable => DecisionPath::MeshUnavailable,
        }
    };

    if !matches!(decision, DecisionPath::AlreadyOwner) {
        task.assignee = Some(assignee.clone());
        task.execution.attempt_count += 1;
        task.updated_at = Utc::now();
        repo.store.write(&task)?;
        repo.index.upsert(&task)?;

        let mut detail = serde_json::Map::new();
        detail.insert(
            "previous_owner".into(),
            serde_json::Value::String(previous_owner.clone()),
        );
        detail.insert(
            "new_owner".into(),
            serde_json::Value::String(assignee.clone()),
        );
        detail.insert(
            "decision".into(),
            serde_json::Value::String(decision.as_str().to_string()),
        );
        detail.insert("forced".into(), serde_json::Value::Bool(force));
        detail.insert(
            "threshold_secs".into(),
            serde_json::Value::Number(serde_json::Number::from(threshold_secs)),
        );
        if let Some(secs) = owner_inactive_secs {
            detail.insert(
                "owner_inactive_secs".into(),
                serde_json::Value::Number(serde_json::Number::from(secs)),
            );
        }

        let evt = HistoryEvent {
            id: None,
            timestamp: Utc::now(),
            event: "takeover".into(),
            agent: Some(assignee.clone()),
            detail,
            links: CoordinationLinks::default(),
        };
        let _ = repo.sidecars.append_history(task_id, &evt);
    }

    Ok(TakeoverResponse {
        event: "takeover",
        task_id,
        previous_owner,
        new_owner: assignee,
        decision,
        forced: force,
        threshold_secs,
        owner_inactive_secs,
        resulting_status: task.status,
        resulting_assignee: task.assignee,
    })
}

fn owner_activity(repo_root: &Path, owner: &str) -> Result<OwnerActivity> {
    let mesh = MeshStore::open(&repo_root.join(".tak"));
    if !mesh.exists() {
        return Ok(OwnerActivity::MeshUnavailable);
    }

    let agents = mesh.list_agents()?;
    let now = Utc::now();
    if let Some(reg) = agents.into_iter().find(|reg| reg.name == owner) {
        let last_seen = reg.last_seen_at.unwrap_or(reg.updated_at);
        let inactive = now.signed_duration_since(last_seen).num_seconds().max(0);
        Ok(OwnerActivity::InactiveFor(inactive))
    } else {
        Ok(OwnerActivity::OwnerNotRegistered)
    }
}

fn render_response(response: &TakeoverResponse, format: Format) -> Result<String> {
    let rendered = match format {
        Format::Json => serde_json::to_string(response)?,
        Format::Pretty => {
            let task_id = TaskId::from(response.task_id);
            let inactivity = response
                .owner_inactive_secs
                .map(|secs| format!("{secs}s"))
                .unwrap_or_else(|| "-".to_string());
            format!(
                "{} {}\n  {} {}\n  {} {} -> {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}\n  {} {}",
                "takeover".cyan().bold(),
                task_id,
                "event:".dimmed(),
                response.event,
                "owners:".dimmed(),
                response.previous_owner,
                response.new_owner,
                "decision:".dimmed(),
                response.decision.as_str(),
                "forced:".dimmed(),
                response.forced,
                "inactive:".dimmed(),
                inactivity,
                "threshold:".dimmed(),
                format!("{}s", response.threshold_secs),
                "state:".dimmed(),
                format!(
                    "status={}, assignee={}",
                    response.resulting_status,
                    response.resulting_assignee.as_deref().unwrap_or("-")
                )
            )
        }
        Format::Minimal => format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            response.event,
            TaskId::from(response.task_id),
            response.previous_owner,
            response.new_owner,
            response.decision.as_str(),
            response.resulting_status
        ),
    };

    Ok(rendered)
}

fn print_response(response: &TakeoverResponse, format: Format) -> Result<()> {
    println!("{}", render_response(response, format)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Kind, Planning};
    use crate::store::files::FileStore;
    use crate::store::mesh::Registration;
    use chrono::Duration;
    use std::fs;
    use tempfile::tempdir;

    fn setup_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        FileStore::init(dir.path()).unwrap();
        dir
    }

    fn create_in_progress_task(repo_root: &Path, title: &str, assignee: &str) -> u64 {
        let repo = Repo::open(repo_root).unwrap();
        let mut task = repo
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
        task.status = Status::InProgress;
        task.assignee = Some(assignee.to_string());
        task.updated_at = Utc::now();
        repo.store.write(&task).unwrap();
        repo.index.upsert(&task).unwrap();
        task.id
    }

    fn set_registration_last_seen(repo_root: &Path, owner: &str, secs_ago: i64) {
        let path = repo_root
            .join(".tak")
            .join("runtime")
            .join("mesh")
            .join("registry")
            .join(format!("{owner}.json"));
        let mut reg: Registration =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let ts = Utc::now() - Duration::seconds(secs_ago);
        reg.updated_at = ts;
        reg.last_seen_at = Some(ts);
        fs::write(path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();
    }

    #[test]
    fn takeover_rejects_active_owner_without_force() {
        let dir = setup_repo();
        let task_id = create_in_progress_task(dir.path(), "takeover-active-owner", "owner-1");

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("owner-1"), Some("sid-owner")).unwrap();

        let err =
            takeover_response(dir.path(), task_id, "agent-2".into(), Some(600), false).unwrap_err();
        match err {
            TakError::Locked(message) => assert!(message.contains("owner 'owner-1' is active")),
            other => panic!("unexpected error: {other:?}"),
        }

        let repo = Repo::open(dir.path()).unwrap();
        let task = repo.store.read(task_id).unwrap();
        assert_eq!(task.assignee.as_deref(), Some("owner-1"));
        assert_eq!(task.status, Status::InProgress);
    }

    #[test]
    fn takeover_allows_inactive_owner_when_threshold_met() {
        let dir = setup_repo();
        let task_id = create_in_progress_task(dir.path(), "takeover-inactive-owner", "owner-1");

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("owner-1"), Some("sid-owner")).unwrap();
        set_registration_last_seen(dir.path(), "owner-1", 3600);

        let response =
            takeover_response(dir.path(), task_id, "agent-2".into(), Some(300), false).unwrap();

        assert_eq!(response.decision, DecisionPath::OwnerInactive);
        assert!(response.owner_inactive_secs.unwrap_or_default() >= 300);
        assert_eq!(response.resulting_assignee.as_deref(), Some("agent-2"));

        let repo = Repo::open(dir.path()).unwrap();
        let events = repo.sidecars.read_history(task_id).unwrap();
        assert!(events.iter().any(|event| event.event == "takeover"));
    }

    #[test]
    fn takeover_allows_force_override_for_active_owner() {
        let dir = setup_repo();
        let task_id = create_in_progress_task(dir.path(), "takeover-force", "owner-1");

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("owner-1"), Some("sid-owner")).unwrap();

        let response =
            takeover_response(dir.path(), task_id, "agent-2".into(), Some(600), true).unwrap();

        assert_eq!(response.decision, DecisionPath::Forced);
        assert!(response.forced);
        assert_eq!(response.resulting_assignee.as_deref(), Some("agent-2"));
    }

    #[test]
    fn takeover_allows_when_owner_not_registered() {
        let dir = setup_repo();
        let task_id =
            create_in_progress_task(dir.path(), "takeover-owner-not-registered", "owner-1");

        let mesh = MeshStore::open(&dir.path().join(".tak"));
        mesh.join(Some("someone-else"), Some("sid-other")).unwrap();

        let response =
            takeover_response(dir.path(), task_id, "agent-2".into(), Some(600), false).unwrap();

        assert_eq!(response.decision, DecisionPath::OwnerNotRegistered);
        assert_eq!(response.resulting_assignee.as_deref(), Some("agent-2"));
    }

    #[test]
    fn takeover_is_idempotent_when_requester_already_owns_task() {
        let dir = setup_repo();
        let task_id = create_in_progress_task(dir.path(), "takeover-already-owner", "agent-1");

        let response =
            takeover_response(dir.path(), task_id, "agent-1".into(), Some(600), false).unwrap();

        assert_eq!(response.decision, DecisionPath::AlreadyOwner);
        assert_eq!(response.resulting_assignee.as_deref(), Some("agent-1"));
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
    fn render_json_includes_previous_owner_and_decision() {
        let response = TakeoverResponse {
            event: "takeover",
            task_id: 42,
            previous_owner: "owner-1".into(),
            new_owner: "agent-2".into(),
            decision: DecisionPath::OwnerInactive,
            forced: false,
            threshold_secs: 600,
            owner_inactive_secs: Some(3600),
            resulting_status: Status::InProgress,
            resulting_assignee: Some("agent-2".into()),
        };

        let rendered = render_response(&response, Format::Json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            value.get("event").and_then(|v| v.as_str()),
            Some("takeover")
        );
        assert_eq!(
            value.get("previous_owner").and_then(|v| v.as_str()),
            Some("owner-1")
        );
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("owner_inactive")
        );
        assert_eq!(
            value.get("resulting_assignee").and_then(|v| v.as_str()),
            Some("agent-2")
        );
    }

    #[test]
    fn render_pretty_and_minimal_include_decision_and_owners() {
        let response = TakeoverResponse {
            event: "takeover",
            task_id: 42,
            previous_owner: "owner-1".into(),
            new_owner: "agent-2".into(),
            decision: DecisionPath::Forced,
            forced: true,
            threshold_secs: 600,
            owner_inactive_secs: Some(1),
            resulting_status: Status::InProgress,
            resulting_assignee: Some("agent-2".into()),
        };

        let pretty = render_response(&response, Format::Pretty).unwrap();
        let plain = strip_ansi(&pretty);
        assert!(plain.contains("owners: owner-1 -> agent-2"));
        assert!(plain.contains("decision: forced"));

        let minimal = render_response(&response, Format::Minimal).unwrap();
        let parts = minimal.split('\t').collect::<Vec<_>>();
        assert_eq!(parts[0], "takeover");
        assert_eq!(parts[2], "owner-1");
        assert_eq!(parts[3], "agent-2");
        assert_eq!(parts[4], "forced");
        assert_eq!(parts[5], "in_progress");
    }
}
