use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::store::coordination::CoordinationLinks;
use crate::store::lock;
use crate::store::paths::{normalize_reservation_path, normalized_paths_conflict};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

const DEFAULT_REGISTRATION_TTL_SECS: u64 = 15 * 60;
const DEFAULT_RESERVATION_TTL_SECS: u64 = 30 * 60;
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeshLeaseConfig {
    pub registration_ttl_secs: u64,
    pub reservation_ttl_secs: u64,
    pub heartbeat_interval_secs: u64,
}

impl Default for MeshLeaseConfig {
    fn default() -> Self {
        Self {
            registration_ttl_secs: DEFAULT_REGISTRATION_TTL_SECS,
            reservation_ttl_secs: DEFAULT_RESERVATION_TTL_SECS,
            heartbeat_interval_secs: DEFAULT_HEARTBEAT_INTERVAL_SECS,
        }
    }
}

/// A registered agent in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registration {
    pub name: String,
    pub session_id: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// A message between agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub text: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "CoordinationLinks::is_empty")]
    pub links: CoordinationLinks,
}

/// A file/path reservation held by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reservation {
    pub agent: String,
    pub paths: Vec<String>,
    pub reason: Option<String>,
    pub since: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// A single event in the activity feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedEvent {
    pub ts: DateTime<Utc>,
    pub agent: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatResult {
    pub agent: String,
    pub refreshed_at: DateTime<Utc>,
    pub refreshed_reservations: usize,
    pub registration: Registration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupStaleAgent {
    pub agent: String,
    pub reason: String,
    pub reservation_paths: Vec<String>,
    pub inbox_messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupReport {
    pub dry_run: bool,
    pub ttl_secs: u64,
    pub candidates: Vec<CleanupStaleAgent>,
    pub removed_agents: Vec<String>,
    pub removed_reservations: usize,
    pub removed_inbox_messages: usize,
}

// ---------------------------------------------------------------------------
// MeshStore
// ---------------------------------------------------------------------------

/// Manages the mesh coordination runtime under `.tak/runtime/mesh/`.
pub struct MeshStore {
    root: PathBuf,
}

impl MeshStore {
    /// Open (but do not yet create) the mesh runtime directory.
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.join("runtime").join("mesh"),
        }
    }

    /// Create all required subdirectories and seed files.
    pub fn ensure_dirs(&self) -> crate::error::Result<()> {
        fs::create_dir_all(self.registry_dir())?;
        fs::create_dir_all(self.inbox_dir())?;
        fs::create_dir_all(self.locks_dir())?;
        let res_path = self.reservations_path();
        if !res_path.exists() {
            fs::write(&res_path, "[]")?;
        }
        let feed_path = self.feed_path();
        if !feed_path.exists() {
            fs::write(&feed_path, "")?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exists(&self) -> bool {
        self.root.exists()
    }

    fn tak_root_path(&self) -> PathBuf {
        self.root
            .parent()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone())
    }

    fn repo_root_path(&self) -> PathBuf {
        self.tak_root_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn lease_config(&self) -> MeshLeaseConfig {
        let mut cfg = MeshLeaseConfig::default();
        let config_path = self.tak_root_path().join("config.json");

        let Ok(content) = fs::read_to_string(config_path) else {
            return cfg;
        };

        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            return cfg;
        };

        let Some(mesh) = value.get("mesh").and_then(serde_json::Value::as_object) else {
            return cfg;
        };

        cfg.registration_ttl_secs = first_positive_u64(
            &[
                mesh.get("registration_ttl_secs"),
                mesh.get("registration_ttl_seconds"),
            ],
            cfg.registration_ttl_secs,
        );

        cfg.reservation_ttl_secs = first_positive_u64(
            &[
                mesh.get("reservation_ttl_secs"),
                mesh.get("reservation_ttl_seconds"),
            ],
            cfg.reservation_ttl_secs,
        );

        cfg.heartbeat_interval_secs = first_positive_u64(
            &[
                mesh.get("heartbeat_interval_secs"),
                mesh.get("heartbeat_interval_seconds"),
            ],
            cfg.heartbeat_interval_secs,
        );

        cfg
    }

    // -- path helpers -------------------------------------------------------

    fn registry_dir(&self) -> PathBuf {
        self.root.join("registry")
    }

    fn inbox_dir(&self) -> PathBuf {
        self.root.join("inbox")
    }

    fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn reservations_path(&self) -> PathBuf {
        self.root.join("reservations.json")
    }

    fn feed_path(&self) -> PathBuf {
        self.root.join("feed.jsonl")
    }

    fn registry_lock_path(&self) -> PathBuf {
        self.locks_dir().join("registry.lock")
    }

    fn inbox_lock_path(&self) -> PathBuf {
        self.locks_dir().join("inbox.lock")
    }

    fn reservations_lock_path(&self) -> PathBuf {
        self.locks_dir().join("reservations.lock")
    }

    fn feed_lock_path(&self) -> PathBuf {
        self.locks_dir().join("feed.lock")
    }

    fn registration_path(&self, name: &str) -> PathBuf {
        self.registry_dir().join(format!("{name}.json"))
    }

    fn agent_inbox_dir(&self, name: &str) -> PathBuf {
        self.inbox_dir().join(name)
    }

    // -- feed ---------------------------------------------------------------

    /// Append a feed event (lock + append + unlock).
    pub fn append_feed(&self, event: &FeedEvent) -> crate::error::Result<()> {
        let lock = lock::acquire_lock(&self.feed_lock_path())?;
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.feed_path())?;
        file.write_all(line.as_bytes())?;
        lock::release_lock(lock)?;
        Ok(())
    }

    /// Read feed events, optionally limited to the last N.
    pub fn read_feed(&self, limit: Option<usize>) -> crate::error::Result<Vec<FeedEvent>> {
        let path = self.feed_path();
        if !path.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&path)?;
        let mut events: Vec<FeedEvent> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if let Some(n) = limit {
            let len = events.len();
            if len > n {
                events = events.split_off(len - n);
            }
        }
        Ok(events)
    }

    // -- registration -------------------------------------------------------

    /// Validate an agent name: non-empty, ASCII alphanumeric + hyphen + underscore.
    fn validate_name(name: &str) -> crate::error::Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(crate::error::TakError::MeshInvalidName);
        }
        Ok(())
    }

    fn generate_name() -> String {
        crate::agent::generated_fallback()
    }

    fn resolve_session_id(session_id: Option<&str>) -> String {
        session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                std::env::var("TAK_SESSION_ID")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                std::env::var("CLAUDE_SESSION_ID")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }

    /// Register an agent in the mesh. Creates registry entry + inbox dir.
    ///
    /// If `name` is omitted, a unique adjective-animal codename is auto-generated.
    pub fn join(
        &self,
        name: Option<&str>,
        session_id: Option<&str>,
    ) -> crate::error::Result<Registration> {
        if let Some(name) = name {
            Self::validate_name(name)?;
        }
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.registry_lock_path())?;

        let resolved_name = if let Some(name) = name {
            let path = self.registration_path(name);
            if path.exists() {
                lock::release_lock(lock)?;
                return Err(crate::error::TakError::MeshNameConflict(name.into()));
            }
            name.to_string()
        } else {
            let mut generated = Self::generate_name();
            while self.registration_path(&generated).exists() {
                generated = Self::generate_name();
            }
            generated
        };

        let path = self.registration_path(&resolved_name);

        let now = Utc::now();
        let sid = Self::resolve_session_id(session_id);
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let reg = Registration {
            name: resolved_name.clone(),
            session_id: sid,
            cwd,
            started_at: now,
            updated_at: now,
            status: "active".into(),
            pid: Some(std::process::id()),
            host: local_host_name(),
            last_seen_at: Some(now),
            metadata: serde_json::Map::new(),
        };

        let json = serde_json::to_string_pretty(&reg)?;
        fs::write(&path, json)?;

        // Create inbox directory for this agent
        fs::create_dir_all(self.agent_inbox_dir(&resolved_name))?;

        lock::release_lock(lock)?;

        // Best-effort feed event
        let _ = self.append_feed(&FeedEvent {
            ts: now,
            agent: resolved_name,
            event_type: "mesh.join".into(),
            target: None,
            preview: Some("joined the mesh".into()),
        });

        Ok(reg)
    }

    /// Unregister an agent. Removes registry entry, inbox, and reservations.
    pub fn leave(&self, name: &str) -> crate::error::Result<()> {
        Self::validate_name(name)?;
        let reg_lock = lock::acquire_lock(&self.registry_lock_path())?;

        let path = self.registration_path(name);
        if !path.exists() {
            lock::release_lock(reg_lock)?;
            return Err(crate::error::TakError::MeshAgentNotFound(name.into()));
        }

        // Clean reservations first — this is the fallible step that can encounter
        // corrupt state. If it fails, no destructive changes have been made yet,
        // so the caller can retry after fixing the issue.
        self.remove_agent_reservations_locked(name)?;

        // Remove inbox under inbox lock to avoid races with concurrent send/inbox
        {
            let _inbox_lock = lock::acquire_lock(&self.inbox_lock_path())?;
            let inbox = self.agent_inbox_dir(name);
            if inbox.exists() {
                fs::remove_dir_all(&inbox)?;
            }
        }

        // Remove registration last (point of no return)
        fs::remove_file(&path)?;

        lock::release_lock(reg_lock)?;

        let _ = self.append_feed(&FeedEvent {
            ts: Utc::now(),
            agent: name.into(),
            event_type: "mesh.leave".into(),
            target: None,
            preview: Some("left the mesh".into()),
        });

        Ok(())
    }

    /// Unregister the current agent using implicit context.
    ///
    /// Resolution order:
    /// 1) `$TAK_AGENT` name (if not found, continue fallback resolution)
    /// 2) `$TAK_SESSION_ID`/`$CLAUDE_SESSION_ID` match (session-first; use cwd only to break ties)
    /// 3) single agent in current cwd
    /// 4) single agent in registry
    pub fn leave_current(&self) -> crate::error::Result<String> {
        if let Some(name) = std::env::var("TAK_AGENT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            match self.leave(&name) {
                Ok(()) => return Ok(name),
                Err(crate::error::TakError::MeshAgentNotFound(_)) => {
                    // TAK_AGENT can point at assignment metadata rather than an
                    // actual mesh registration. Fall through to session/cwd
                    // based resolution so stop hooks can still clean up.
                }
                Err(err) => return Err(err),
            }
        }

        let agents = self.list_agents()?;
        if agents.is_empty() {
            return Err(crate::error::TakError::MeshAgentNotFound(
                "current-session".into(),
            ));
        }

        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let session_id = std::env::var("TAK_SESSION_ID")
            .ok()
            .or_else(|| std::env::var("CLAUDE_SESSION_ID").ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(sid) = session_id {
            let by_session: Vec<&Registration> =
                agents.iter().filter(|a| a.session_id == sid).collect();
            if by_session.len() == 1 {
                let name = by_session[0].name.clone();
                self.leave(&name)?;
                return Ok(name);
            }
            if by_session.len() > 1 {
                let by_session_cwd: Vec<&Registration> = by_session
                    .iter()
                    .copied()
                    .filter(|a| !cwd.is_empty() && a.cwd == cwd)
                    .collect();
                if by_session_cwd.len() == 1 {
                    let name = by_session_cwd[0].name.clone();
                    self.leave(&name)?;
                    return Ok(name);
                }

                let ambiguous = if by_session_cwd.len() > 1 {
                    by_session_cwd
                } else {
                    by_session
                };
                let names = ambiguous
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(crate::error::TakError::MeshAmbiguousAgent(names));
            }
        }

        let by_cwd: Vec<&Registration> = agents
            .iter()
            .filter(|a| !cwd.is_empty() && a.cwd == cwd)
            .collect();
        if by_cwd.len() == 1 {
            let name = by_cwd[0].name.clone();
            self.leave(&name)?;
            return Ok(name);
        }
        if by_cwd.len() > 1 {
            let names = by_cwd
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(crate::error::TakError::MeshAmbiguousAgent(names));
        }

        if agents.len() == 1 {
            let name = agents[0].name.clone();
            self.leave(&name)?;
            return Ok(name);
        }

        let names = agents
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(crate::error::TakError::MeshAmbiguousAgent(names))
    }

    pub fn heartbeat(
        &self,
        name: Option<&str>,
        session_id: Option<&str>,
    ) -> crate::error::Result<HeartbeatResult> {
        if let Some(name) = name {
            Self::validate_name(name)?;
        }

        let agents = self.list_agents()?;
        let resolved_name = resolve_agent_name_for_runtime(&agents, name, session_id)?;

        let reg_lock = lock::acquire_lock(&self.registry_lock_path())?;
        let path = self.registration_path(&resolved_name);
        if !path.exists() {
            lock::release_lock(reg_lock)?;
            return Err(crate::error::TakError::MeshAgentNotFound(resolved_name));
        }

        let content = fs::read_to_string(&path)?;
        let mut reg: Registration = serde_json::from_str(&content).map_err(|e| {
            crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
        })?;

        let now = Utc::now();
        reg.updated_at = now;
        reg.last_seen_at = Some(now);
        reg.pid = Some(std::process::id());
        if let Some(host) = local_host_name() {
            reg.host = Some(host);
        }

        let json = serde_json::to_string_pretty(&reg)?;
        fs::write(&path, json)?;

        let cfg = self.lease_config();
        let refreshed =
            self.refresh_agent_reservations_locked(&reg.name, now, cfg.reservation_ttl_secs)?;

        lock::release_lock(reg_lock)?;

        let _ = self.append_feed(&FeedEvent {
            ts: now,
            agent: reg.name.clone(),
            event_type: "mesh.heartbeat".into(),
            target: None,
            preview: Some(format!("heartbeat (reservations refreshed: {refreshed})")),
        });

        Ok(HeartbeatResult {
            agent: reg.name.clone(),
            refreshed_at: now,
            refreshed_reservations: refreshed,
            registration: reg,
        })
    }

    pub fn cleanup_stale(
        &self,
        ttl_override_secs: Option<u64>,
        dry_run: bool,
    ) -> crate::error::Result<CleanupReport> {
        let cfg = self.lease_config();
        let ttl_secs = ttl_override_secs
            .filter(|v| *v > 0)
            .unwrap_or(cfg.registration_ttl_secs);

        if !self.exists() {
            return Ok(CleanupReport {
                dry_run,
                ttl_secs,
                candidates: vec![],
                removed_agents: vec![],
                removed_reservations: 0,
                removed_inbox_messages: 0,
            });
        }

        let now = Utc::now();
        let reg_lock = lock::acquire_lock(&self.registry_lock_path())?;

        let agents = self.list_agents()?;
        let live_agent_names: HashSet<String> = agents.iter().map(|a| a.name.clone()).collect();

        let reservations_lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let reservations_path = self.reservations_path();
        let mut reservations: Vec<Reservation> = if reservations_path.exists() {
            let content = fs::read_to_string(&reservations_path)?;
            serde_json::from_str(&content).map_err(|e| {
                crate::error::TakError::MeshCorruptFile(
                    reservations_path.display().to_string(),
                    e.to_string(),
                )
            })?
        } else {
            vec![]
        };

        let mut candidates = Vec::new();
        let mut stale_names = HashSet::new();

        for reg in &agents {
            if let Some(reason) = self.stale_reason_for_registration(reg, now, ttl_secs) {
                stale_names.insert(reg.name.clone());
                let reservation_paths = reservations
                    .iter()
                    .filter(|r| r.agent == reg.name)
                    .flat_map(|r| r.paths.clone())
                    .collect::<Vec<_>>();
                let inbox_messages = count_inbox_messages(&self.agent_inbox_dir(&reg.name))?;
                candidates.push(CleanupStaleAgent {
                    agent: reg.name.clone(),
                    reason,
                    reservation_paths,
                    inbox_messages,
                });
            }
        }

        for orphan in reservations
            .iter()
            .map(|r| r.agent.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|agent| !live_agent_names.contains(agent))
        {
            if stale_names.insert(orphan.clone()) {
                let reservation_paths = reservations
                    .iter()
                    .filter(|r| r.agent == orphan)
                    .flat_map(|r| r.paths.clone())
                    .collect::<Vec<_>>();
                let inbox_messages = count_inbox_messages(&self.agent_inbox_dir(&orphan))?;
                candidates.push(CleanupStaleAgent {
                    agent: orphan,
                    reason: "missing-registration".into(),
                    reservation_paths,
                    inbox_messages,
                });
            }
        }

        candidates.sort_by(|a, b| a.agent.cmp(&b.agent));

        if dry_run {
            lock::release_lock(reservations_lock)?;
            lock::release_lock(reg_lock)?;
            return Ok(CleanupReport {
                dry_run: true,
                ttl_secs,
                candidates,
                removed_agents: vec![],
                removed_reservations: 0,
                removed_inbox_messages: 0,
            });
        }

        let mut removed_agents = stale_names.iter().cloned().collect::<Vec<_>>();
        removed_agents.sort();

        for name in &removed_agents {
            let reg_path = self.registration_path(name);
            if reg_path.exists() {
                fs::remove_file(reg_path)?;
            }
        }

        let before_res = reservations.len();
        reservations.retain(|r| !stale_names.contains(&r.agent));
        let removed_reservations = before_res.saturating_sub(reservations.len());
        let json = serde_json::to_string_pretty(&reservations)?;
        fs::write(&reservations_path, json)?;
        lock::release_lock(reservations_lock)?;

        let inbox_lock = lock::acquire_lock(&self.inbox_lock_path())?;
        let mut removed_inbox_messages = 0usize;
        for name in &removed_agents {
            let inbox = self.agent_inbox_dir(name);
            removed_inbox_messages += count_inbox_messages(&inbox)?;
            if inbox.exists() {
                fs::remove_dir_all(inbox)?;
            }
        }
        lock::release_lock(inbox_lock)?;

        lock::release_lock(reg_lock)?;

        for name in &removed_agents {
            let _ = self.append_feed(&FeedEvent {
                ts: now,
                agent: name.clone(),
                event_type: "mesh.leave.stale".into(),
                target: None,
                preview: Some("stale agent cleaned up".into()),
            });
        }

        Ok(CleanupReport {
            dry_run: false,
            ttl_secs,
            candidates,
            removed_agents,
            removed_reservations,
            removed_inbox_messages,
        })
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> crate::error::Result<Vec<Registration>> {
        if !self.exists() {
            return Ok(vec![]);
        }
        let dir = self.registry_dir();
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut agents = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            let reg: Registration = serde_json::from_str(&content).map_err(|e| {
                crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
            })?;
            agents.push(reg);
        }
        agents.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(agents)
    }

    fn stale_reason_for_registration(
        &self,
        reg: &Registration,
        now: DateTime<Utc>,
        ttl_secs: u64,
    ) -> Option<String> {
        let last_seen = reg.last_seen_at.unwrap_or(reg.updated_at);
        if now.signed_duration_since(last_seen) <= Duration::seconds(ttl_secs as i64) {
            return None;
        }

        if let Some(pid) = reg.pid
            && registration_host_matches_local(reg)
            && !is_pid_alive(pid)
        {
            return Some(format!("ttl-expired:{ttl_secs}s,pid-not-alive:{pid}"));
        }

        Some(format!("ttl-expired:{ttl_secs}s"))
    }

    /// Refresh reservation lease metadata for one agent.
    /// Acquires the reservations lock internally.
    fn refresh_agent_reservations_locked(
        &self,
        agent: &str,
        now: DateTime<Utc>,
        ttl_secs: u64,
    ) -> crate::error::Result<usize> {
        let lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let path = self.reservations_path();
        if !path.exists() {
            lock::release_lock(lock)?;
            return Ok(0);
        }

        let content = fs::read_to_string(&path)?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content).map_err(|e| {
            crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
        })?;

        let mut refreshed = 0usize;
        for reservation in &mut reservations {
            if reservation.agent != agent {
                continue;
            }
            reservation.ttl_secs = Some(ttl_secs);
            reservation.last_heartbeat_at = Some(now);
            reservation.expires_at = Some(now + Duration::seconds(ttl_secs as i64));
            refreshed += 1;
        }

        if refreshed > 0 {
            let json = serde_json::to_string_pretty(&reservations)?;
            fs::write(&path, json)?;
        }

        lock::release_lock(lock)?;
        Ok(refreshed)
    }

    /// Remove all reservations belonging to an agent.
    /// Acquires the reservations lock internally.
    fn remove_agent_reservations_locked(&self, name: &str) -> crate::error::Result<()> {
        let lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let path = self.reservations_path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            let reservations: Vec<Reservation> = serde_json::from_str(&content).map_err(|e| {
                crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
            })?;
            let filtered: Vec<Reservation> = reservations
                .into_iter()
                .filter(|r| r.agent != name)
                .collect();
            let json = serde_json::to_string_pretty(&filtered)?;
            fs::write(&path, json)?;
        }
        lock::release_lock(lock)?;
        Ok(())
    }

    // -- messaging ----------------------------------------------------------

    /// Send a message to a specific agent. Enqueues in their inbox directory.
    pub fn send(
        &self,
        from: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> crate::error::Result<Message> {
        self.send_with_links(from, to, text, reply_to, CoordinationLinks::default())
    }

    /// Send a message with optional cross-channel linkage metadata.
    pub fn send_with_links(
        &self,
        from: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
        mut links: CoordinationLinks,
    ) -> crate::error::Result<Message> {
        Self::validate_name(from)?;
        Self::validate_name(to)?;

        let reply_to = reply_to
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        if let Some(parent_id) = reply_to.as_ref() {
            links.mesh_message_ids.push(parent_id.clone());
        }
        links.normalize();

        // Hold registry lock to serialize against concurrent leave, preventing
        // send from writing to an inbox that leave is about to delete (which
        // would create an orphaned inbox directory for a non-existent agent)
        let _reg_lock = lock::acquire_lock(&self.registry_lock_path())?;
        if !self.registration_path(to).exists() {
            return Err(crate::error::TakError::MeshAgentNotFound(to.into()));
        }

        let lock = lock::acquire_lock(&self.inbox_lock_path())?;

        let now = Utc::now();
        let msg = Message {
            id: uuid::Uuid::new_v4().to_string(),
            from: from.into(),
            to: to.into(),
            text: text.into(),
            timestamp: now,
            reply_to,
            links,
        };

        let inbox = self.agent_inbox_dir(to);
        fs::create_dir_all(&inbox)?;
        let ts = now.format("%Y%m%d%H%M%S%3f");
        let short_id = msg.id.get(..8).unwrap_or(&msg.id);
        let filename = format!("{ts}-{short_id}.json");
        let json = serde_json::to_string_pretty(&msg)?;
        fs::write(inbox.join(&filename), json)?;

        lock::release_lock(lock)?;

        let _ = self.append_feed(&FeedEvent {
            ts: now,
            agent: from.into(),
            event_type: "mesh.send".into(),
            target: Some(to.into()),
            preview: Some(truncate(text, 80)),
        });

        Ok(msg)
    }

    /// Broadcast a message to all registered agents (except sender).
    pub fn broadcast(&self, from: &str, text: &str) -> crate::error::Result<Vec<Message>> {
        self.broadcast_with_links(from, text, CoordinationLinks::default())
    }

    /// Broadcast with optional cross-channel linkage metadata.
    pub fn broadcast_with_links(
        &self,
        from: &str,
        text: &str,
        links: CoordinationLinks,
    ) -> crate::error::Result<Vec<Message>> {
        Self::validate_name(from)?;
        let agents = self.list_agents()?;
        let mut messages = Vec::new();
        for agent in &agents {
            if agent.name != from {
                let msg = self.send_with_links(from, &agent.name, text, None, links.clone())?;
                messages.push(msg);
            }
        }
        Ok(messages)
    }

    /// Read inbox messages for an agent. If `ack` is true, delete after reading.
    pub fn inbox(&self, name: &str, ack: bool) -> crate::error::Result<Vec<Message>> {
        Self::validate_name(name)?;
        let dir = self.agent_inbox_dir(name);
        if !dir.exists() {
            return Ok(vec![]);
        }

        let lock = lock::acquire_lock(&self.inbox_lock_path())?;

        let mut messages = Vec::new();
        let mut files = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            let msg: Message = serde_json::from_str(&content).map_err(|e| {
                crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
            })?;
            files.push(path.clone());
            messages.push(msg);
        }

        // Sort by timestamp
        messages.sort_by_key(|m| m.timestamp);

        if ack {
            for path in &files {
                let _ = fs::remove_file(path);
            }
        }

        lock::release_lock(lock)?;
        Ok(messages)
    }

    // -- reservations -------------------------------------------------------

    /// Reserve one or more paths for an agent. Fails if any path conflicts
    /// with another agent's reservation. Same-agent reservations are replaced.
    pub fn reserve(
        &self,
        agent: &str,
        raw_paths: Vec<String>,
        reason: Option<&str>,
    ) -> crate::error::Result<Reservation> {
        Self::validate_name(agent)?;

        let repo_root = self.repo_root_path();

        // Canonicalize requested paths per RFC 0001 normalization contract.
        let mut paths = Vec::new();
        for path in raw_paths {
            let normalized = normalize_reservation_path(&path, &repo_root)
                .map_err(|_| crate::error::TakError::MeshInvalidPath(path.clone()))?;
            paths.push(normalized);
        }
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Err(crate::error::TakError::MeshInvalidPath("<empty>".into()));
        }

        // Hold registry lock to serialize against concurrent leave, preventing TOCTOU
        // where leave could remove registration between our check and reservation write
        let _reg_lock = lock::acquire_lock(&self.registry_lock_path())?;
        if !self.registration_path(agent).exists() {
            return Err(crate::error::TakError::MeshAgentNotFound(agent.into()));
        }

        let cfg = self.lease_config();
        let now = Utc::now();
        let live_agents: HashSet<String> = self
            .list_agents()?
            .into_iter()
            .map(|registration| registration.name)
            .collect();

        let lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let reservations_path = self.reservations_path();
        let content = fs::read_to_string(&reservations_path)?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content).map_err(|e| {
            crate::error::TakError::MeshCorruptFile(
                reservations_path.display().to_string(),
                e.to_string(),
            )
        })?;

        // Opportunistically drop stale reservation rows before conflict checks.
        reservations.retain(|existing| {
            live_agents.contains(&existing.agent)
                && !reservation_is_expired(existing, now, cfg.reservation_ttl_secs)
        });

        // Normalize existing rows opportunistically so conflict checks are
        // deterministic even when legacy spellings are present.
        for existing in &mut reservations {
            existing.paths = existing
                .paths
                .iter()
                .map(|path| normalize_existing_path(path, &repo_root))
                .collect();
            existing.paths.sort();
            existing.paths.dedup();
        }
        reservations.retain(|reservation| !reservation.paths.is_empty());

        // Check for conflicts with other agents
        for existing in &reservations {
            if existing.agent == agent {
                continue;
            }
            for new_path in &paths {
                for held_path in &existing.paths {
                    if paths_conflict(new_path, held_path) {
                        lock::release_lock(lock)?;
                        return Err(crate::error::TakError::MeshReservationConflict {
                            requested_path: new_path.clone(),
                            held_path: held_path.clone(),
                            owner: existing.agent.clone(),
                            reason: existing.reason.clone().unwrap_or_else(|| "none".into()),
                            age_secs: (now - existing.since).num_seconds().max(0),
                        });
                    }
                }
            }
        }

        // Replace mode: remove any existing reservation by this agent
        reservations.retain(|r| r.agent != agent);

        let reservation = Reservation {
            agent: agent.into(),
            paths,
            reason: reason.map(|s| s.to_string()),
            since: now,
            ttl_secs: Some(cfg.reservation_ttl_secs),
            last_heartbeat_at: Some(now),
            expires_at: Some(now + Duration::seconds(cfg.reservation_ttl_secs as i64)),
        };
        reservations.push(reservation.clone());

        let json = serde_json::to_string_pretty(&reservations)?;
        fs::write(&reservations_path, json)?;

        lock::release_lock(lock)?;

        let _ = self.append_feed(&FeedEvent {
            ts: now,
            agent: agent.into(),
            event_type: "mesh.reserve".into(),
            target: None,
            preview: Some(format!("reserved {}", reservation.paths.join(", "))),
        });

        Ok(reservation)
    }

    /// Release reservations. If `paths` is empty, release all for the agent.
    pub fn release(&self, agent: &str, raw_paths: Vec<String>) -> crate::error::Result<()> {
        Self::validate_name(agent)?;

        let repo_root = self.repo_root_path();

        // Canonicalize requested release paths using the same rules as reserve.
        let mut paths = Vec::new();
        for path in raw_paths {
            let normalized = normalize_reservation_path(&path, &repo_root)
                .map_err(|_| crate::error::TakError::MeshInvalidPath(path.clone()))?;
            paths.push(normalized);
        }
        paths.sort();
        paths.dedup();

        // Hold registry lock to serialize against concurrent leave
        let _reg_lock = lock::acquire_lock(&self.registry_lock_path())?;
        if !self.registration_path(agent).exists() {
            return Err(crate::error::TakError::MeshAgentNotFound(agent.into()));
        }

        let cfg = self.lease_config();
        let now = Utc::now();

        let lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let reservations_path = self.reservations_path();
        let content = fs::read_to_string(&reservations_path)?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content).map_err(|e| {
            crate::error::TakError::MeshCorruptFile(
                reservations_path.display().to_string(),
                e.to_string(),
            )
        })?;

        // Drop expired rows before applying explicit release edits.
        reservations
            .retain(|existing| !reservation_is_expired(existing, now, cfg.reservation_ttl_secs));

        for reservation in &mut reservations {
            reservation.paths = reservation
                .paths
                .iter()
                .map(|path| normalize_existing_path(path, &repo_root))
                .collect();
            reservation.paths.sort();
            reservation.paths.dedup();
        }
        reservations.retain(|reservation| !reservation.paths.is_empty());

        if paths.is_empty() {
            // Release all
            reservations.retain(|r| r.agent != agent);
        } else {
            // Remove specific canonical paths from the agent's reservation.
            for res in &mut reservations {
                if res.agent == agent {
                    res.paths
                        .retain(|p| !paths.iter().any(|release_path| p == release_path));
                }
            }
            // Remove empty reservations
            reservations.retain(|r| !r.paths.is_empty());
        }

        let json = serde_json::to_string_pretty(&reservations)?;
        fs::write(&reservations_path, json)?;

        lock::release_lock(lock)?;

        let _ = self.append_feed(&FeedEvent {
            ts: Utc::now(),
            agent: agent.into(),
            event_type: "mesh.release".into(),
            target: None,
            preview: if paths.is_empty() {
                Some("released all".into())
            } else {
                Some(format!("released {}", paths.join(", ")))
            },
        });

        Ok(())
    }

    /// List all current reservations.
    pub fn list_reservations(&self) -> crate::error::Result<Vec<Reservation>> {
        let path = self.reservations_path();
        if !path.exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(&path)?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content).map_err(|e| {
            crate::error::TakError::MeshCorruptFile(path.display().to_string(), e.to_string())
        })?;

        let cfg = self.lease_config();
        let now = Utc::now();
        let repo_root = self.repo_root_path();
        let live_agents: HashSet<String> = self
            .list_agents()?
            .into_iter()
            .map(|registration| registration.name)
            .collect();

        for reservation in &mut reservations {
            reservation.paths = reservation
                .paths
                .iter()
                .map(|path| normalize_existing_path(path, &repo_root))
                .collect();
            reservation.paths.sort();
            reservation.paths.dedup();
        }

        let active = reservations
            .into_iter()
            .filter(|reservation| {
                !reservation.paths.is_empty()
                    && live_agents.contains(&reservation.agent)
                    && !reservation_is_expired(reservation, now, cfg.reservation_ttl_secs)
            })
            .collect();

        Ok(active)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn first_positive_u64(values: &[Option<&serde_json::Value>], default_value: u64) -> u64 {
    values
        .iter()
        .flatten()
        .find_map(|v| v.as_u64())
        .filter(|v| *v > 0)
        .unwrap_or(default_value)
}

fn local_host_name() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn registration_host_matches_local(registration: &Registration) -> bool {
    let Some(registration_host) = registration.host.as_deref().map(str::trim) else {
        return true;
    };
    if registration_host.is_empty() {
        return true;
    }
    let Some(local_host) = local_host_name() else {
        return true;
    };
    registration_host.eq_ignore_ascii_case(local_host.trim())
}

fn resolve_agent_name_for_runtime(
    agents: &[Registration],
    explicit_name: Option<&str>,
    explicit_session_id: Option<&str>,
) -> crate::error::Result<String> {
    if agents.is_empty() {
        return Err(crate::error::TakError::MeshAgentNotFound(
            "current-session".into(),
        ));
    }

    if let Some(name) = explicit_name {
        if agents.iter().any(|agent| agent.name == name) {
            return Ok(name.to_string());
        }
        return Err(crate::error::TakError::MeshAgentNotFound(name.to_string()));
    }

    if let Some(name) = std::env::var("TAK_AGENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && agents.iter().any(|agent| agent.name == *s))
    {
        return Ok(name);
    }

    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let session_id = explicit_session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("TAK_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("CLAUDE_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    if let Some(sid) = session_id {
        let by_session: Vec<&Registration> =
            agents.iter().filter(|a| a.session_id == sid).collect();
        if by_session.len() == 1 {
            return Ok(by_session[0].name.clone());
        }
        if by_session.len() > 1 {
            let by_session_cwd: Vec<&Registration> = by_session
                .iter()
                .copied()
                .filter(|a| !cwd.is_empty() && a.cwd == cwd)
                .collect();
            if by_session_cwd.len() == 1 {
                return Ok(by_session_cwd[0].name.clone());
            }

            let ambiguous = if by_session_cwd.len() > 1 {
                by_session_cwd
            } else {
                by_session
            };
            let names = ambiguous
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(crate::error::TakError::MeshAmbiguousAgent(names));
        }
    }

    let by_cwd: Vec<&Registration> = agents
        .iter()
        .filter(|a| !cwd.is_empty() && a.cwd == cwd)
        .collect();
    if by_cwd.len() == 1 {
        return Ok(by_cwd[0].name.clone());
    }
    if by_cwd.len() > 1 {
        let names = by_cwd
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(crate::error::TakError::MeshAmbiguousAgent(names));
    }

    if agents.len() == 1 {
        return Ok(agents[0].name.clone());
    }

    let names = agents
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(crate::error::TakError::MeshAmbiguousAgent(names))
}

fn count_inbox_messages(path: &Path) -> crate::error::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_path = entry.path();
        if file_path.extension().and_then(|e| e.to_str()) == Some("json") {
            count += 1;
        }
    }

    Ok(count)
}

fn reservation_ttl_secs(reservation: &Reservation, default_ttl_secs: u64) -> u64 {
    reservation
        .ttl_secs
        .filter(|v| *v > 0)
        .unwrap_or(default_ttl_secs)
}

fn reservation_last_touch(reservation: &Reservation) -> DateTime<Utc> {
    reservation.last_heartbeat_at.unwrap_or(reservation.since)
}

fn reservation_is_expired(
    reservation: &Reservation,
    now: DateTime<Utc>,
    default_ttl_secs: u64,
) -> bool {
    if let Some(expires_at) = reservation.expires_at {
        return expires_at <= now;
    }

    let ttl_secs = reservation_ttl_secs(reservation, default_ttl_secs);
    now.signed_duration_since(reservation_last_touch(reservation))
        > Duration::seconds(ttl_secs as i64)
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

#[cfg(not(unix))]
fn is_pid_alive(_pid: u32) -> bool {
    true
}

/// Legacy lexical normalizer used as a fallback when older reservation rows
/// cannot be canonicalized under current strict rules.
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

fn normalize_existing_path(path: &str, repo_root: &Path) -> String {
    normalize_reservation_path(path, repo_root).unwrap_or_else(|_| normalize_path(path))
}

/// Two paths conflict if one is a segment-bounded prefix of the other or they
/// are exactly equal.
fn paths_conflict(a: &str, b: &str) -> bool {
    let a = normalize_path(a);
    let b = normalize_path(b);
    normalized_paths_conflict(&a, &b)
}

/// Truncate a string to max_len chars, adding "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // -- setup helper -------------------------------------------------------

    fn setup_mesh() -> (tempfile::TempDir, MeshStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        let store = MeshStore::open(&tak_root);
        store.ensure_dirs().unwrap();
        (dir, store)
    }

    // -- data model round-trip tests ----------------------------------------

    #[test]
    fn registration_round_trips() {
        let reg = Registration {
            name: "agent-1".into(),
            session_id: "test-session".into(),
            cwd: "/repo".into(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
            status: "active".into(),
            pid: Some(1234),
            host: Some("localhost".into()),
            last_seen_at: Some(Utc::now()),
            metadata: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&reg).unwrap();
        let parsed: Registration = serde_json::from_str(&json).unwrap();
        assert_eq!(reg, parsed);
        // Empty metadata omitted
        assert!(!json.contains("metadata"));
    }

    #[test]
    fn message_round_trips() {
        let msg = Message {
            id: "abc-123".into(),
            from: "AgentA".into(),
            to: "AgentB".into(),
            text: "please take task 17".into(),
            timestamp: Utc::now(),
            reply_to: None,
            links: CoordinationLinks::default(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
        assert!(!json.contains("reply_to"));
        assert!(!json.contains("links"));
    }

    #[test]
    fn reservation_round_trips() {
        let res = Reservation {
            agent: "AgentA".into(),
            paths: vec!["src/store/".into(), "src/model.rs".into()],
            reason: Some("task-17".into()),
            since: Utc::now(),
            ttl_secs: Some(120),
            last_heartbeat_at: Some(Utc::now()),
            expires_at: Some(Utc::now()),
        };
        let json = serde_json::to_string(&res).unwrap();
        let parsed: Reservation = serde_json::from_str(&json).unwrap();
        assert_eq!(res, parsed);
    }

    #[test]
    fn feed_event_round_trips() {
        let evt = FeedEvent {
            ts: Utc::now(),
            agent: "AgentA".into(),
            event_type: "mesh.join".into(),
            target: None,
            preview: Some("joined the mesh".into()),
        };
        let json = serde_json::to_string(&evt).unwrap();
        let parsed: FeedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(evt, parsed);
        assert!(!json.contains("target"));
    }

    // -- directory / feed tests ---------------------------------------------

    #[test]
    fn ensure_dirs_creates_structure() {
        let (_dir, store) = setup_mesh();
        assert!(store.registry_dir().exists());
        assert!(store.inbox_dir().exists());
        assert!(store.locks_dir().exists());
        assert!(store.reservations_path().exists());
        assert!(store.feed_path().exists());
        // Idempotent
        store.ensure_dirs().unwrap();
    }

    #[test]
    fn lease_config_uses_defaults_without_config_file() {
        let (_dir, store) = setup_mesh();
        let cfg = store.lease_config();

        assert_eq!(cfg.registration_ttl_secs, DEFAULT_REGISTRATION_TTL_SECS);
        assert_eq!(cfg.reservation_ttl_secs, DEFAULT_RESERVATION_TTL_SECS);
        assert_eq!(cfg.heartbeat_interval_secs, DEFAULT_HEARTBEAT_INTERVAL_SECS);
    }

    #[test]
    fn lease_config_reads_mesh_defaults_from_config_file() {
        let dir = tempdir().unwrap();
        let _store = crate::store::files::FileStore::init(dir.path()).unwrap();
        let mesh = MeshStore::open(&dir.path().join(".tak"));

        let cfg = mesh.lease_config();
        assert_eq!(cfg.registration_ttl_secs, 900);
        assert_eq!(cfg.reservation_ttl_secs, 1800);
        assert_eq!(cfg.heartbeat_interval_secs, 30);
    }

    #[test]
    fn lease_config_reads_legacy_seconds_aliases() {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        fs::write(
            tak_root.join("config.json"),
            r#"{
  "version": 2,
  "mesh": {
    "registration_ttl_seconds": 111,
    "reservation_ttl_seconds": 222,
    "heartbeat_interval_seconds": 7
  }
}"#,
        )
        .unwrap();

        let mesh = MeshStore::open(&tak_root);
        let cfg = mesh.lease_config();

        assert_eq!(cfg.registration_ttl_secs, 111);
        assert_eq!(cfg.reservation_ttl_secs, 222);
        assert_eq!(cfg.heartbeat_interval_secs, 7);
    }

    #[test]
    fn feed_append_and_read() {
        let (_dir, store) = setup_mesh();
        let evt1 = FeedEvent {
            ts: Utc::now(),
            agent: "A".into(),
            event_type: "mesh.join".into(),
            target: None,
            preview: Some("joined".into()),
        };
        let evt2 = FeedEvent {
            ts: Utc::now(),
            agent: "B".into(),
            event_type: "mesh.join".into(),
            target: None,
            preview: Some("joined".into()),
        };
        store.append_feed(&evt1).unwrap();
        store.append_feed(&evt2).unwrap();

        let all = store.read_feed(None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].agent, "A");
        assert_eq!(all[1].agent, "B");

        // Limit
        let last = store.read_feed(Some(1)).unwrap();
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].agent, "B");
    }

    #[test]
    fn feed_read_empty() {
        let (_dir, store) = setup_mesh();
        let events = store.read_feed(None).unwrap();
        assert!(events.is_empty());
    }

    // -- registration tests -------------------------------------------------

    #[test]
    fn join_and_list() {
        let (_dir, store) = setup_mesh();
        let reg = store.join(Some("agent-1"), Some("sess-1")).unwrap();
        assert_eq!(reg.name, "agent-1");
        assert_eq!(reg.session_id, "sess-1");
        assert_eq!(reg.status, "active");
        assert!(store.agent_inbox_dir("agent-1").exists());

        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "agent-1");
    }

    #[test]
    fn join_sets_presence_lease_metadata() {
        let (_dir, store) = setup_mesh();
        let before = Utc::now();

        let reg = store.join(Some("agent-meta"), Some("sess-meta")).unwrap();

        assert_eq!(reg.pid, Some(std::process::id()));
        assert!(reg.last_seen_at.is_some());
        assert!(reg.last_seen_at.unwrap() >= before);

        let listed = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "agent-meta")
            .expect("agent-meta should be listed");
        assert_eq!(listed.pid, Some(std::process::id()));
        assert!(listed.last_seen_at.is_some());
    }

    #[test]
    fn join_auto_generates_adjective_animal_name() {
        let (_dir, store) = setup_mesh();
        let reg = store.join(None, Some("sess-1")).unwrap();

        let parts: Vec<&str> = reg.name.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));

        assert_eq!(reg.session_id, "sess-1");
        assert!(store.agent_inbox_dir(&reg.name).exists());
    }

    #[test]
    fn join_auto_name_is_unique() {
        let (_dir, store) = setup_mesh();
        let a = store.join(None, None).unwrap();
        let b = store.join(None, None).unwrap();
        assert_ne!(a.name, b.name);
    }

    #[test]
    fn join_uses_env_session_id_when_not_provided() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("CLAUDE_SESSION_ID", "sess-from-env") };

        let (_dir, store) = setup_mesh();
        let reg = store.join(Some("agent-env"), None).unwrap();
        assert_eq!(reg.session_id, "sess-from-env");

        unsafe { std::env::remove_var("CLAUDE_SESSION_ID") };
    }

    #[test]
    fn leave_current_prefers_session_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TAK_AGENT");
            std::env::set_var("CLAUDE_SESSION_ID", "sess-a");
        }

        let (_dir, store) = setup_mesh();
        store.join(Some("agent-a"), Some("sess-a")).unwrap();
        store.join(Some("agent-b"), Some("sess-b")).unwrap();

        let left = store.leave_current().unwrap();
        assert_eq!(left, "agent-a");

        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "agent-b");

        unsafe { std::env::remove_var("CLAUDE_SESSION_ID") };
    }

    #[test]
    fn leave_current_falls_back_when_tak_agent_is_not_registered() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TAK_AGENT", "assigned-agent");
            std::env::set_var("CLAUDE_SESSION_ID", "sess-auto");
        }

        let (_dir, store) = setup_mesh();
        let reg = store.join(None, Some("sess-auto")).unwrap();

        let left = store.leave_current().unwrap();
        assert_eq!(left, reg.name);

        let agents = store.list_agents().unwrap();
        assert!(agents.is_empty());

        unsafe {
            std::env::remove_var("TAK_AGENT");
            std::env::remove_var("CLAUDE_SESSION_ID");
        }
    }

    #[test]
    fn leave_current_matches_session_even_if_cwd_changed() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TAK_AGENT");
            std::env::set_var("CLAUDE_SESSION_ID", "sess-target");
        }

        let (_dir, store) = setup_mesh();
        store.join(Some("agent-a"), Some("sess-target")).unwrap();
        store.join(Some("agent-b"), Some("sess-other")).unwrap();

        // Simulate running `mesh leave` from a different cwd than `mesh join`.
        let reg_path = store.registration_path("agent-a");
        let mut reg: Registration =
            serde_json::from_str(&fs::read_to_string(&reg_path).unwrap()).unwrap();
        reg.cwd = "/different/cwd".into();
        fs::write(&reg_path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();

        let left = store.leave_current().unwrap();
        assert_eq!(left, "agent-a");

        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "agent-b");

        unsafe { std::env::remove_var("CLAUDE_SESSION_ID") };
    }

    #[test]
    fn leave_current_ambiguous_without_context() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("TAK_AGENT");
            std::env::remove_var("TAK_SESSION_ID");
            std::env::remove_var("CLAUDE_SESSION_ID");
        }

        let (_dir, store) = setup_mesh();
        store.join(Some("agent-a"), Some("sess-a")).unwrap();
        store.join(Some("agent-b"), Some("sess-b")).unwrap();

        let err = store.leave_current().unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAmbiguousAgent(_)));
    }

    #[test]
    fn join_name_conflict() {
        let (_dir, store) = setup_mesh();
        store.join(Some("agent-1"), None).unwrap();
        let err = store.join(Some("agent-1"), None).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshNameConflict(_)));
    }

    #[test]
    fn join_invalid_name() {
        let (_dir, store) = setup_mesh();
        assert!(store.join(Some(""), None).is_err());
        assert!(store.join(Some("has space"), None).is_err());
        assert!(store.join(Some("has/slash"), None).is_err());
    }

    #[test]
    fn path_traversal_rejected_on_all_entry_points() {
        let (_dir, store) = setup_mesh();
        let evil = "../../../etc";
        assert!(store.join(Some(evil), None).is_err());
        assert!(store.leave(evil).is_err());
        assert!(store.send(evil, "ok", "hi", None).is_err());
        assert!(store.send("ok", evil, "hi", None).is_err());
        assert!(store.inbox(evil, false).is_err());
        assert!(store.broadcast(evil, "hi").is_err());
        assert!(store.reserve(evil, vec!["f".into()], None).is_err());
        assert!(store.release(evil, vec![]).is_err());
        assert!(store.heartbeat(Some(evil), None).is_err());
    }

    #[test]
    fn leave_removes_registration() {
        let (_dir, store) = setup_mesh();
        store.join(Some("agent-1"), None).unwrap();
        store.leave("agent-1").unwrap();

        let agents = store.list_agents().unwrap();
        assert!(agents.is_empty());
        assert!(!store.registration_path("agent-1").exists());
        assert!(!store.agent_inbox_dir("agent-1").exists());
    }

    #[test]
    fn leave_unknown_agent() {
        let (_dir, store) = setup_mesh();
        let err = store.leave("ghost").unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAgentNotFound(_)));
    }

    #[test]
    fn list_empty_when_no_mesh() {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        let store = MeshStore::open(&tak_root);
        // Don't call ensure_dirs -- mesh not initialized
        let agents = store.list_agents().unwrap();
        assert!(agents.is_empty());
    }

    // -- messaging tests ----------------------------------------------------

    #[test]
    fn send_and_inbox() {
        let (_dir, store) = setup_mesh();
        store.join(Some("sender"), None).unwrap();
        store.join(Some("receiver"), None).unwrap();

        let msg = store.send("sender", "receiver", "hello", None).unwrap();
        assert_eq!(msg.from, "sender");
        assert_eq!(msg.to, "receiver");
        assert_eq!(msg.text, "hello");

        let inbox = store.inbox("receiver", false).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "hello");

        // Not acked -- still there
        let inbox2 = store.inbox("receiver", false).unwrap();
        assert_eq!(inbox2.len(), 1);

        // Ack -- gone
        let inbox3 = store.inbox("receiver", true).unwrap();
        assert_eq!(inbox3.len(), 1);
        let inbox4 = store.inbox("receiver", false).unwrap();
        assert!(inbox4.is_empty());
    }

    #[test]
    fn send_with_links_normalizes_and_persists_cross_channel_ids() {
        let (_dir, store) = setup_mesh();
        store.join(Some("sender"), None).unwrap();
        store.join(Some("receiver"), None).unwrap();

        let msg = store
            .send_with_links(
                "sender",
                "receiver",
                "hello",
                Some(" parent-msg "),
                CoordinationLinks {
                    mesh_message_ids: vec!["mesh-b".into(), "mesh-b".into()],
                    blackboard_note_ids: vec![9, 3, 9],
                    history_event_ids: vec![" h2 ".into(), "h1".into()],
                },
            )
            .unwrap();

        assert_eq!(msg.reply_to.as_deref(), Some("parent-msg"));
        assert_eq!(msg.links.mesh_message_ids, vec!["mesh-b", "parent-msg"]);
        assert_eq!(msg.links.blackboard_note_ids, vec![3, 9]);
        assert_eq!(msg.links.history_event_ids, vec!["h1", "h2"]);

        let inbox = store.inbox("receiver", false).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].links, msg.links);
    }

    #[test]
    fn send_to_unknown_agent() {
        let (_dir, store) = setup_mesh();
        store.join(Some("sender"), None).unwrap();
        let err = store.send("sender", "ghost", "hello", None).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAgentNotFound(_)));
    }

    #[test]
    fn broadcast_sends_to_all_except_sender() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.join(Some("B"), None).unwrap();
        store.join(Some("C"), None).unwrap();

        let msgs = store.broadcast("A", "announcement").unwrap();
        assert_eq!(msgs.len(), 2);

        let b_inbox = store.inbox("B", false).unwrap();
        assert_eq!(b_inbox.len(), 1);
        assert_eq!(b_inbox[0].text, "announcement");

        let c_inbox = store.inbox("C", false).unwrap();
        assert_eq!(c_inbox.len(), 1);

        // A should have no messages
        let a_inbox = store.inbox("A", false).unwrap();
        assert!(a_inbox.is_empty());
    }

    #[test]
    fn inbox_empty_returns_empty_vec() {
        let (_dir, store) = setup_mesh();
        store.join(Some("lonely"), None).unwrap();
        let msgs = store.inbox("lonely", false).unwrap();
        assert!(msgs.is_empty());
    }

    // -- reservation tests --------------------------------------------------

    #[test]
    fn reserve_and_list() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        let res = store
            .reserve("A", vec!["src/store/".into()], Some("task-1"))
            .unwrap();
        assert_eq!(res.agent, "A");
        assert_eq!(res.paths, vec!["src/store"]);

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn reserve_conflict() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.join(Some("B"), None).unwrap();
        store
            .reserve("A", vec!["src/store/".into()], Some("task-1"))
            .unwrap();

        // Sub-path conflict
        let err = store
            .reserve("B", vec!["src/store/mesh.rs".into()], None)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("requested 'src/store/mesh.rs' overlaps held 'src/store'"));
        assert!(msg.contains("agent 'A'"));
        assert!(msg.contains("reason: task-1"));
        assert!(msg.contains("age:"));

        match err {
            crate::error::TakError::MeshReservationConflict {
                requested_path,
                held_path,
                owner,
                reason,
                age_secs,
            } => {
                assert_eq!(requested_path, "src/store/mesh.rs");
                assert_eq!(held_path, "src/store");
                assert_eq!(owner, "A");
                assert_eq!(reason, "task-1");
                assert!(age_secs >= 0);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn reserve_same_agent_replaces() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.reserve("A", vec!["src/b.rs".into()], None).unwrap();

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].paths, vec!["src/b.rs"]);
    }

    #[test]
    fn release_specific_paths() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store
            .reserve("A", vec!["src/a.rs".into(), "src/b.rs".into()], None)
            .unwrap();
        store.release("A", vec!["src/a.rs".into()]).unwrap();

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].paths, vec!["src/b.rs"]);
    }

    #[test]
    fn release_all() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.release("A", vec![]).unwrap();

        let all = store.list_reservations().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn corrupt_reservations_errors_instead_of_silent_drop() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        // Write corrupt data
        fs::write(store.reservations_path(), "NOT VALID JSON").unwrap();
        // All reservation operations should error, not silently default
        assert!(store.list_reservations().is_err());
        assert!(store.reserve("A", vec!["src/a.rs".into()], None).is_err());
        assert!(store.release("A", vec![]).is_err());
    }

    #[test]
    fn paths_conflict_logic() {
        assert!(paths_conflict("src/store/", "src/store/mesh.rs"));
        assert!(paths_conflict("src/store/mesh.rs", "src/store/"));
        assert!(paths_conflict("src/store/", "src/store/"));
        assert!(paths_conflict("src/store", "src/store/"));
        assert!(!paths_conflict("src/store/", "src/model.rs"));
        assert!(!paths_conflict("src/a.rs", "src/b.rs"));
        // Equivalent spellings must conflict after normalization
        assert!(paths_conflict("src/./lib.rs", "src/lib.rs"));
        assert!(paths_conflict("src/../src/lib.rs", "src/lib.rs"));
        assert!(paths_conflict("src//lib.rs", "src/lib.rs"));
        assert!(paths_conflict("./src/store/", "src/store/mesh.rs"));
    }

    #[test]
    fn normalize_path_cases() {
        assert_eq!(normalize_path("src/./lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src/../src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src//lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("./src/store/"), "src/store/");
        assert_eq!(normalize_path("src/store"), "src/store");
        assert_eq!(normalize_path(""), "");
        assert_eq!(normalize_path("."), "");
        assert_eq!(normalize_path("a/b/c"), "a/b/c");
    }

    #[test]
    fn leave_cleans_up_reservations() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.leave("A").unwrap();

        let all = store.list_reservations().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn reserve_sets_lease_metadata() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();

        let reservation = store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        assert_eq!(reservation.agent, "A");
        assert!(reservation.ttl_secs.is_some());
        assert!(reservation.last_heartbeat_at.is_some());
        assert!(reservation.expires_at.is_some());

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].ttl_secs.is_some());
        assert!(all[0].expires_at.is_some());
    }

    #[test]
    fn heartbeat_refreshes_registration_and_reservation_lease() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), Some("sess-a")).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();

        let reg_path = store.registration_path("A");
        let mut reg: Registration =
            serde_json::from_str(&fs::read_to_string(&reg_path).unwrap()).unwrap();
        reg.last_seen_at = Some(Utc::now() - Duration::seconds(3600));
        reg.updated_at = Utc::now() - Duration::seconds(3600);
        fs::write(&reg_path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();

        let before = Utc::now();
        let heartbeat = store.heartbeat(Some("A"), None).unwrap();
        assert_eq!(heartbeat.agent, "A");
        assert_eq!(heartbeat.refreshed_reservations, 1);

        let refreshed: Registration =
            serde_json::from_str(&fs::read_to_string(&reg_path).unwrap()).unwrap();
        assert!(refreshed.last_seen_at.is_some());
        assert!(refreshed.last_seen_at.unwrap() >= before);

        let reservations = store.list_reservations().unwrap();
        assert_eq!(reservations.len(), 1);
        let expires = reservations[0].expires_at.expect("lease expiry set");
        assert!(expires > before);
    }

    #[test]
    fn cleanup_stale_dry_run_then_apply() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.join(Some("B"), None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.reserve("B", vec!["src/b.rs".into()], None).unwrap();

        let reg_path = store.registration_path("A");
        let mut reg: Registration =
            serde_json::from_str(&fs::read_to_string(&reg_path).unwrap()).unwrap();
        reg.last_seen_at = Some(Utc::now() - Duration::seconds(7200));
        reg.updated_at = Utc::now() - Duration::seconds(7200);
        fs::write(&reg_path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();

        let preview = store.cleanup_stale(Some(60), true).unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.removed_agents.len(), 0);
        assert!(
            preview
                .candidates
                .iter()
                .any(|candidate| candidate.agent == "A")
        );

        let applied = store.cleanup_stale(Some(60), false).unwrap();
        assert!(!applied.dry_run);
        assert!(applied.removed_agents.iter().any(|name| name == "A"));

        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "B");

        let reservations = store.list_reservations().unwrap();
        assert_eq!(reservations.len(), 1);
        assert_eq!(reservations[0].agent, "B");
    }

    #[test]
    fn reserve_reclaims_expired_conflict_rows() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.join(Some("B"), None).unwrap();
        store.reserve("A", vec!["src/store/".into()], None).unwrap();

        let mut reservations: Vec<Reservation> =
            serde_json::from_str(&fs::read_to_string(store.reservations_path()).unwrap()).unwrap();
        reservations[0].expires_at = Some(Utc::now() - Duration::seconds(5));
        fs::write(
            store.reservations_path(),
            serde_json::to_string_pretty(&reservations).unwrap(),
        )
        .unwrap();

        // The stale/expired A reservation should be reclaimed before conflict checks.
        let b_res = store
            .reserve("B", vec!["src/store/mesh.rs".into()], None)
            .unwrap();
        assert_eq!(b_res.agent, "B");

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].agent, "B");
    }

    // -- Path normalization -------------------------------------------------

    #[test]
    fn reserve_normalizes_stored_paths() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store
            .reserve("A", vec!["src/./lib.rs".into()], None)
            .unwrap();
        let all = store.list_reservations().unwrap();
        assert_eq!(all[0].paths, vec!["src/lib.rs"]);
    }

    #[test]
    fn reserve_detects_conflict_through_equivalent_spelling() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.join(Some("B"), None).unwrap();
        store.reserve("A", vec!["src/store/".into()], None).unwrap();
        let err = store
            .reserve("B", vec!["./src/store/mesh.rs".into()], None)
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::TakError::MeshReservationConflict { .. }
        ));
    }

    #[test]
    fn release_with_equivalent_spelling() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store
            .reserve("A", vec!["src/a.rs".into(), "src/b.rs".into()], None)
            .unwrap();
        // Release using un-normalized spelling
        store.release("A", vec!["./src/a.rs".into()]).unwrap();
        let all = store.list_reservations().unwrap();
        assert_eq!(all[0].paths, vec!["src/b.rs"]);
    }

    #[test]
    fn release_trailing_slash_equivalence() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        store.reserve("A", vec!["src/store/".into()], None).unwrap();
        // Release without trailing slash — should still match
        store.release("A", vec!["src/store".into()]).unwrap();
        let all = store.list_reservations().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn reserve_rejects_paths_escaping_repo_root() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();

        let err = store
            .reserve("A", vec!["../src/mesh.rs".into()], None)
            .unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshInvalidPath(_)));
    }

    #[test]
    fn reserve_accepts_absolute_paths_under_repo_and_converts_to_relative() {
        let (dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();

        let absolute = dir.path().join("src").join("store").join("mesh.rs");
        let reservation = store
            .reserve("A", vec![absolute.to_string_lossy().into_owned()], None)
            .unwrap();

        assert_eq!(reservation.paths, vec!["src/store/mesh.rs"]);
    }

    #[test]
    fn reserve_rejects_absolute_paths_outside_repo() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();

        let outside = tempdir().unwrap();
        let absolute = outside.path().join("elsewhere.txt");
        let err = store
            .reserve("A", vec![absolute.to_string_lossy().into_owned()], None)
            .unwrap_err();

        assert!(matches!(err, crate::error::TakError::MeshInvalidPath(_)));
    }

    // -- Fix 1: reserve/release require registered agent --------------------

    #[test]
    fn reserve_rejects_unregistered_agent() {
        let (_dir, store) = setup_mesh();
        let err = store
            .reserve("ghost", vec!["src/a.rs".into()], None)
            .unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAgentNotFound(_)));
    }

    #[test]
    fn release_rejects_unregistered_agent() {
        let (_dir, store) = setup_mesh();
        let err = store.release("ghost", vec![]).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAgentNotFound(_)));
    }

    // -- Fix 2: leave with corrupt reservations preserves registration ------

    #[test]
    fn leave_with_corrupt_reservations_preserves_registration() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        fs::write(store.reservations_path(), "NOT VALID JSON").unwrap();
        // leave should fail due to corrupt reservations
        assert!(store.leave("A").is_err());
        // Registration must still exist — no partial deletion
        assert!(store.registration_path("A").exists());
        assert!(store.agent_inbox_dir("A").exists());
        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
    }

    // -- Fix 4: corrupt JSON surfaced as errors -----------------------------

    #[test]
    fn list_agents_errors_on_corrupt_registry() {
        let (_dir, store) = setup_mesh();
        store.join(Some("good"), None).unwrap();
        // Write corrupt registry entry
        fs::write(store.registration_path("bad"), "NOT VALID JSON").unwrap();
        let err = store.list_agents().unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshCorruptFile(_, _)));
    }

    #[test]
    fn inbox_errors_on_corrupt_message() {
        let (_dir, store) = setup_mesh();
        store.join(Some("A"), None).unwrap();
        // Write corrupt message to inbox
        let inbox = store.agent_inbox_dir("A");
        fs::write(inbox.join("corrupt.json"), "NOT VALID JSON").unwrap();
        let err = store.inbox("A", false).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshCorruptFile(_, _)));
    }
}
