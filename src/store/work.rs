use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::{Result, TakError};
use crate::store::lock;

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum WorkVerifyMode {
    #[default]
    Isolated,
    Local,
}

impl std::fmt::Display for WorkVerifyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Isolated => write!(f, "isolated"),
            Self::Local => write!(f, "local"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum WorkClaimStrategy {
    #[default]
    PriorityThenAge,
    EpicCloseout,
}

impl std::fmt::Display for WorkClaimStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PriorityThenAge => write!(f, "priority_then_age"),
            Self::EpicCloseout => write!(f, "epic_closeout"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkState {
    pub agent: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub processed: u32,
    #[serde(default)]
    pub verify_mode: WorkVerifyMode,
    #[serde(default)]
    pub claim_strategy: WorkClaimStrategy,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

impl WorkState {
    pub fn inactive(agent: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            agent: agent.into(),
            active: false,
            current_task_id: None,
            tag: None,
            remaining: None,
            processed: 0,
            verify_mode: WorkVerifyMode::default(),
            claim_strategy: WorkClaimStrategy::default(),
            started_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        }
    }

    fn normalize(&mut self) {
        self.agent = self.agent.trim().to_string();
        self.tag = self
            .tag
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if !self.active {
            self.current_task_id = None;
        }

        if self.updated_at < self.started_at {
            self.updated_at = self.started_at;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivationOutcome {
    pub resumed: bool,
    pub state: WorkState,
}

/// Manages `tak work` loop state under `.tak/runtime/work/`.
pub struct WorkStore {
    root: PathBuf,
}

impl WorkStore {
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.join("runtime").join("work"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.states_dir())?;
        fs::create_dir_all(self.locks_dir())?;
        Ok(())
    }

    pub fn validate_agent_name(agent: &str) -> Result<()> {
        let valid = !agent.is_empty()
            && agent
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(TakError::WorkInvalidAgentName(agent.to_string()));
        }
        Ok(())
    }

    pub fn load(&self, agent: &str) -> Result<Option<WorkState>> {
        Self::validate_agent_name(agent)?;
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path(agent))?;
        let state = self.read_state_locked(agent)?;
        lock::release_lock(lock)?;

        Ok(state)
    }

    pub fn status(&self, agent: &str) -> Result<WorkState> {
        let now = Utc::now();
        Ok(self
            .load(agent)?
            .unwrap_or_else(|| WorkState::inactive(agent, now)))
    }

    /// Persist a fully materialized state for an agent.
    pub fn save(&self, state: &WorkState) -> Result<WorkState> {
        Self::validate_agent_name(&state.agent)?;
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path(&state.agent))?;
        let mut next = state.clone();
        next.updated_at = Utc::now();
        next.normalize();
        self.write_state_locked(&next)?;
        lock::release_lock(lock)?;

        Ok(next)
    }

    pub fn activate(
        &self,
        agent: &str,
        tag: Option<String>,
        limit: Option<u32>,
        verify_mode: Option<WorkVerifyMode>,
        claim_strategy: Option<WorkClaimStrategy>,
    ) -> Result<ActivationOutcome> {
        Self::validate_agent_name(agent)?;
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path(agent))?;
        let existing = self.read_state_locked(agent)?;
        let now = Utc::now();

        let (resumed, mut state) = match existing {
            Some(mut current) => {
                let resumed = current.active;
                if !resumed {
                    current.started_at = now;
                    current.processed = 0;
                    current.current_task_id = None;
                    current.tag = None;
                    current.remaining = None;
                    current.verify_mode = WorkVerifyMode::default();
                    current.claim_strategy = WorkClaimStrategy::default();
                }
                (resumed, current)
            }
            None => {
                let mut fresh = WorkState::inactive(agent, now);
                fresh.active = true;
                (false, fresh)
            }
        };

        state.agent = agent.to_string();
        state.active = true;

        if let Some(tag) = tag {
            state.tag = Some(tag);
        }
        if let Some(limit) = limit {
            state.remaining = Some(limit);
        }
        if let Some(verify_mode) = verify_mode {
            state.verify_mode = verify_mode;
        }
        if let Some(claim_strategy) = claim_strategy {
            state.claim_strategy = claim_strategy;
        }

        state.updated_at = now;
        state.normalize();

        self.write_state_locked(&state)?;
        lock::release_lock(lock)?;

        Ok(ActivationOutcome { resumed, state })
    }

    pub fn deactivate(&self, agent: &str) -> Result<WorkState> {
        Self::validate_agent_name(agent)?;
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.lock_path(agent))?;
        let now = Utc::now();

        let mut state = self
            .read_state_locked(agent)?
            .unwrap_or_else(|| WorkState::inactive(agent, now));

        state.agent = agent.to_string();
        state.active = false;
        state.current_task_id = None;
        state.updated_at = now;
        state.normalize();

        self.write_state_locked(&state)?;
        lock::release_lock(lock)?;

        Ok(state)
    }

    fn states_dir(&self) -> PathBuf {
        self.root.join("states")
    }

    fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn state_path(&self, agent: &str) -> PathBuf {
        self.states_dir().join(format!("{agent}.json"))
    }

    fn lock_path(&self, agent: &str) -> PathBuf {
        self.locks_dir().join(format!("{agent}.lock"))
    }

    fn read_state_locked(&self, agent: &str) -> Result<Option<WorkState>> {
        let path = self.state_path(agent);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)?;
        let mut state: WorkState = serde_json::from_str(&content)
            .map_err(|e| TakError::WorkCorruptFile(path.display().to_string(), e.to_string()))?;

        if state.agent.trim().is_empty() || state.agent != agent {
            state.agent = agent.to_string();
        }

        state.normalize();
        Ok(Some(state))
    }

    fn write_state_locked(&self, state: &WorkState) -> Result<()> {
        let mut normalized = state.clone();
        normalized.normalize();

        fs::write(
            self.state_path(&normalized.agent),
            serde_json::to_string_pretty(&normalized)?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tempfile::tempdir;

    fn setup_store() -> (tempfile::TempDir, WorkStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        let store = WorkStore::open(&tak_root);
        store.ensure_dirs().unwrap();
        (dir, store)
    }

    #[test]
    fn activate_creates_and_persists_state() {
        let (_dir, store) = setup_store();

        let outcome = store
            .activate(
                "agent_1",
                Some("  cli  ".into()),
                Some(3),
                Some(WorkVerifyMode::Local),
                Some(WorkClaimStrategy::EpicCloseout),
            )
            .unwrap();

        assert!(!outcome.resumed);
        assert!(outcome.state.active);
        assert_eq!(outcome.state.agent, "agent_1");
        assert_eq!(outcome.state.tag.as_deref(), Some("cli"));
        assert_eq!(outcome.state.remaining, Some(3));
        assert_eq!(outcome.state.verify_mode, WorkVerifyMode::Local);
        assert_eq!(
            outcome.state.claim_strategy,
            WorkClaimStrategy::EpicCloseout
        );

        let loaded = store.load("agent_1").unwrap().unwrap();
        assert_eq!(loaded, outcome.state);
    }

    #[test]
    fn activate_resumes_existing_active_state() {
        let (_dir, store) = setup_store();

        let first = store
            .activate("agent-1", Some("core".into()), Some(2), None, None)
            .unwrap()
            .state;
        let second = store.activate("agent-1", None, None, None, None).unwrap();

        assert!(second.resumed);
        assert_eq!(second.state.started_at, first.started_at);
        assert_eq!(second.state.tag.as_deref(), Some("core"));
        assert_eq!(second.state.remaining, Some(2));
        assert_eq!(
            second.state.claim_strategy,
            WorkClaimStrategy::PriorityThenAge
        );
        assert!(second.state.updated_at >= first.updated_at);
    }

    #[test]
    fn deactivate_marks_state_inactive_and_clears_current_task() {
        let (_dir, store) = setup_store();

        let mut active = store
            .activate("agent-1", None, Some(1), None, None)
            .unwrap()
            .state;
        active.current_task_id = Some(42);
        store.write_state_locked(&active).unwrap();

        let stopped = store.deactivate("agent-1").unwrap();
        assert!(!stopped.active);
        assert!(stopped.current_task_id.is_none());

        let loaded = store.load("agent-1").unwrap().unwrap();
        assert!(!loaded.active);
        assert!(loaded.current_task_id.is_none());
    }

    #[test]
    fn status_for_missing_agent_returns_inactive_without_persisting_file() {
        let (_dir, store) = setup_store();

        let status = store.status("agent-1").unwrap();
        assert_eq!(status.agent, "agent-1");
        assert!(!status.active);

        assert!(!store.state_path("agent-1").exists());
    }

    #[test]
    fn save_persists_updated_state_fields() {
        let (_dir, store) = setup_store();

        let mut state = store
            .activate("agent-1", Some("cli".into()), Some(2), None, None)
            .unwrap()
            .state;
        state.current_task_id = Some(42);
        state.processed = 3;
        state.remaining = Some(1);

        let saved = store.save(&state).unwrap();
        assert_eq!(saved.current_task_id, Some(42));
        assert_eq!(saved.processed, 3);
        assert_eq!(saved.remaining, Some(1));

        let loaded = store.load("agent-1").unwrap().unwrap();
        assert_eq!(loaded.current_task_id, Some(42));
        assert_eq!(loaded.processed, 3);
        assert_eq!(loaded.remaining, Some(1));
    }

    #[test]
    fn unknown_fields_survive_repeated_invocations() {
        let (_dir, store) = setup_store();
        let now = Utc::now();

        let mut state = WorkState::inactive("agent-1", now);
        state.active = true;
        state
            .extensions
            .insert("future_field".into(), serde_json::json!({"x": 1}));
        fs::write(
            store.state_path("agent-1"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();

        let _ = store
            .activate("agent-1", None, Some(4), None, None)
            .unwrap();

        let raw = fs::read_to_string(store.state_path("agent-1")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(value.get("future_field").is_some());
    }

    #[test]
    fn concurrent_activation_updates_keep_state_file_valid() {
        let (dir, store) = setup_store();
        let tak_root = dir.path().join(".tak");

        let handles = (1..=8)
            .map(|limit| {
                let tak_root = tak_root.clone();
                thread::spawn(move || {
                    let store = WorkStore::open(&tak_root);
                    store
                        .activate(
                            "agent-1",
                            None,
                            Some(limit),
                            Some(WorkVerifyMode::Isolated),
                            Some(WorkClaimStrategy::EpicCloseout),
                        )
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap();
        }

        let raw = fs::read_to_string(store.state_path("agent-1")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value.get("agent").and_then(|v| v.as_str()), Some("agent-1"));
        assert_eq!(
            value.get("verify_mode").and_then(|v| v.as_str()),
            Some("isolated")
        );
        assert_eq!(
            value.get("claim_strategy").and_then(|v| v.as_str()),
            Some("epic_closeout")
        );
    }

    #[test]
    fn invalid_agent_name_is_rejected() {
        let (_dir, store) = setup_store();
        let err = store.status("bad name").unwrap_err();
        assert!(matches!(err, TakError::WorkInvalidAgentName(_)));
    }

    #[test]
    fn corrupt_state_file_returns_structured_error() {
        let (_dir, store) = setup_store();

        fs::write(store.state_path("agent-1"), "not json").unwrap();
        let err = store.load("agent-1").unwrap_err();
        assert!(matches!(err, TakError::WorkCorruptFile(_, _)));
    }
}
