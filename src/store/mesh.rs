use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::lock;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A registered agent in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registration {
    pub name: String,
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: String,
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
}

/// A file/path reservation held by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reservation {
    pub agent: String,
    pub paths: Vec<String>,
    pub reason: Option<String>,
    pub since: DateTime<Utc>,
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

    /// Register an agent in the mesh. Creates registry entry + inbox dir.
    pub fn join(&self, name: &str, session_id: Option<&str>) -> crate::error::Result<Registration> {
        Self::validate_name(name)?;
        self.ensure_dirs()?;

        let lock = lock::acquire_lock(&self.registry_lock_path())?;

        // Check for name conflict
        let path = self.registration_path(name);
        if path.exists() {
            lock::release_lock(lock)?;
            return Err(crate::error::TakError::MeshNameConflict(name.into()));
        }

        let now = Utc::now();
        let sid = session_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let reg = Registration {
            name: name.into(),
            pid: std::process::id(),
            session_id: sid,
            cwd,
            started_at: now,
            updated_at: now,
            status: "active".into(),
            metadata: serde_json::Map::new(),
        };

        let json = serde_json::to_string_pretty(&reg)?;
        fs::write(&path, json)?;

        // Create inbox directory for this agent
        fs::create_dir_all(self.agent_inbox_dir(name))?;

        lock::release_lock(lock)?;

        // Best-effort feed event
        let _ = self.append_feed(&FeedEvent {
            ts: now,
            agent: name.into(),
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

    /// List all registered agents.
    ///
    /// Note: PID-based stale cleanup is intentionally NOT done here because
    /// `tak` is a CLI tool where each invocation is a separate process.
    /// The PID stored at `join` time is always dead by the next command.
    /// Use a future `mesh cleanup --stale` for explicit stale detection.
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

    /// Clean up a stale agent entry (best-effort).
    /// Reserved for future `mesh cleanup --stale` command.
    #[allow(dead_code)]
    fn cleanup_stale_agent(&self, name: &str) -> crate::error::Result<()> {
        let path = self.registration_path(name);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let inbox = self.agent_inbox_dir(name);
        if inbox.exists() {
            let _ = fs::remove_dir_all(&inbox);
        }
        self.remove_agent_reservations_locked(name)?;
        let _ = self.append_feed(&FeedEvent {
            ts: Utc::now(),
            agent: name.into(),
            event_type: "mesh.leave.stale".into(),
            target: None,
            preview: Some("stale agent cleaned up".into()),
        });
        Ok(())
    }

    /// Remove all reservations belonging to an agent.
    /// Acquires the reservations lock internally.
    fn remove_agent_reservations_locked(&self, name: &str) -> crate::error::Result<()> {
        let lock = lock::acquire_lock(&self.reservations_lock_path())?;
        let path = self.reservations_path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            let reservations: Vec<Reservation> = serde_json::from_str(&content)?;
            let filtered: Vec<&Reservation> =
                reservations.iter().filter(|r| r.agent != name).collect();
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
        Self::validate_name(from)?;
        Self::validate_name(to)?;
        // Verify recipient exists
        let to_path = self.registration_path(to);
        if !to_path.exists() {
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
            reply_to: reply_to.map(|s| s.to_string()),
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
        Self::validate_name(from)?;
        let agents = self.list_agents()?;
        let mut messages = Vec::new();
        for agent in &agents {
            if agent.name != from {
                let msg = self.send(from, &agent.name, text, None)?;
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
        paths: Vec<String>,
        reason: Option<&str>,
    ) -> crate::error::Result<Reservation> {
        Self::validate_name(agent)?;
        // Require registry membership to prevent invisible locks from unregistered names
        if !self.registration_path(agent).exists() {
            return Err(crate::error::TakError::MeshAgentNotFound(agent.into()));
        }
        let lock = lock::acquire_lock(&self.reservations_lock_path())?;

        let content = fs::read_to_string(self.reservations_path())?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content)?;

        // Check for conflicts with other agents
        for existing in &reservations {
            if existing.agent == agent {
                continue;
            }
            for new_path in &paths {
                for held_path in &existing.paths {
                    if paths_conflict(new_path, held_path) {
                        lock::release_lock(lock)?;
                        return Err(crate::error::TakError::MeshReservationConflict(
                            new_path.clone(),
                            existing.agent.clone(),
                        ));
                    }
                }
            }
        }

        // Replace mode: remove any existing reservation by this agent
        reservations.retain(|r| r.agent != agent);

        let now = Utc::now();
        let reservation = Reservation {
            agent: agent.into(),
            paths,
            reason: reason.map(|s| s.to_string()),
            since: now,
        };
        reservations.push(reservation.clone());

        let json = serde_json::to_string_pretty(&reservations)?;
        fs::write(self.reservations_path(), json)?;

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
    pub fn release(&self, agent: &str, paths: Vec<String>) -> crate::error::Result<()> {
        Self::validate_name(agent)?;
        // Require registry membership
        if !self.registration_path(agent).exists() {
            return Err(crate::error::TakError::MeshAgentNotFound(agent.into()));
        }
        let lock = lock::acquire_lock(&self.reservations_lock_path())?;

        let content = fs::read_to_string(self.reservations_path())?;
        let mut reservations: Vec<Reservation> = serde_json::from_str(&content)?;

        if paths.is_empty() {
            // Release all
            reservations.retain(|r| r.agent != agent);
        } else {
            // Remove specific paths from the agent's reservation
            for res in &mut reservations {
                if res.agent == agent {
                    res.paths.retain(|p| !paths.contains(p));
                }
            }
            // Remove empty reservations
            reservations.retain(|r| !r.paths.is_empty());
        }

        let json = serde_json::to_string_pretty(&reservations)?;
        fs::write(self.reservations_path(), json)?;

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
        let reservations: Vec<Reservation> = serde_json::from_str(&content)?;
        Ok(reservations)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a PID is alive using `kill -0` (signal 0 checks existence without
/// actually sending a signal). Reserved for future `mesh cleanup --stale`.
#[cfg(unix)]
#[allow(dead_code)]
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(true) // Conservative: assume alive on error
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn is_pid_alive(_pid: u32) -> bool {
    true // Conservative: assume alive on non-Unix
}

/// Two paths conflict if one is a prefix of the other (directory containment)
/// or they are exactly equal.
fn paths_conflict(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a_norm = a.trim_end_matches('/');
    let b_norm = b.trim_end_matches('/');
    if a_norm == b_norm {
        return true;
    }
    let a_dir = format!("{a_norm}/");
    let b_dir = format!("{b_norm}/");
    b_norm.starts_with(&a_dir) || a_norm.starts_with(&b_dir)
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
    use tempfile::tempdir;

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
            pid: 12345,
            session_id: "test-session".into(),
            cwd: "/repo".into(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
            status: "active".into(),
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
        assert!(!json.contains("reply_to"));
    }

    #[test]
    fn reservation_round_trips() {
        let res = Reservation {
            agent: "AgentA".into(),
            paths: vec!["src/store/".into(), "src/model.rs".into()],
            reason: Some("task-17".into()),
            since: Utc::now(),
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
        let reg = store.join("agent-1", Some("sess-1")).unwrap();
        assert_eq!(reg.name, "agent-1");
        assert_eq!(reg.session_id, "sess-1");
        assert_eq!(reg.status, "active");
        assert!(store.agent_inbox_dir("agent-1").exists());

        let agents = store.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "agent-1");
    }

    #[test]
    fn join_name_conflict() {
        let (_dir, store) = setup_mesh();
        store.join("agent-1", None).unwrap();
        let err = store.join("agent-1", None).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshNameConflict(_)));
    }

    #[test]
    fn join_invalid_name() {
        let (_dir, store) = setup_mesh();
        assert!(store.join("", None).is_err());
        assert!(store.join("has space", None).is_err());
        assert!(store.join("has/slash", None).is_err());
    }

    #[test]
    fn path_traversal_rejected_on_all_entry_points() {
        let (_dir, store) = setup_mesh();
        let evil = "../../../etc";
        assert!(store.join(evil, None).is_err());
        assert!(store.leave(evil).is_err());
        assert!(store.send(evil, "ok", "hi", None).is_err());
        assert!(store.send("ok", evil, "hi", None).is_err());
        assert!(store.inbox(evil, false).is_err());
        assert!(store.broadcast(evil, "hi").is_err());
        assert!(store.reserve(evil, vec!["f".into()], None).is_err());
        assert!(store.release(evil, vec![]).is_err());
    }

    #[test]
    fn leave_removes_registration() {
        let (_dir, store) = setup_mesh();
        store.join("agent-1", None).unwrap();
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
        store.join("sender", None).unwrap();
        store.join("receiver", None).unwrap();

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
    fn send_to_unknown_agent() {
        let (_dir, store) = setup_mesh();
        store.join("sender", None).unwrap();
        let err = store.send("sender", "ghost", "hello", None).unwrap_err();
        assert!(matches!(err, crate::error::TakError::MeshAgentNotFound(_)));
    }

    #[test]
    fn broadcast_sends_to_all_except_sender() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        store.join("B", None).unwrap();
        store.join("C", None).unwrap();

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
        store.join("lonely", None).unwrap();
        let msgs = store.inbox("lonely", false).unwrap();
        assert!(msgs.is_empty());
    }

    // -- reservation tests --------------------------------------------------

    #[test]
    fn reserve_and_list() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        let res = store
            .reserve("A", vec!["src/store/".into()], Some("task-1"))
            .unwrap();
        assert_eq!(res.agent, "A");
        assert_eq!(res.paths, vec!["src/store/"]);

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn reserve_conflict() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        store.join("B", None).unwrap();
        store.reserve("A", vec!["src/store/".into()], None).unwrap();

        // Sub-path conflict
        let err = store
            .reserve("B", vec!["src/store/mesh.rs".into()], None)
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::TakError::MeshReservationConflict(_, _)
        ));
    }

    #[test]
    fn reserve_same_agent_replaces() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.reserve("A", vec!["src/b.rs".into()], None).unwrap();

        let all = store.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].paths, vec!["src/b.rs"]);
    }

    #[test]
    fn release_specific_paths() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
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
        store.join("A", None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.release("A", vec![]).unwrap();

        let all = store.list_reservations().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn corrupt_reservations_errors_instead_of_silent_drop() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
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
    }

    #[test]
    fn leave_cleans_up_reservations() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        store.reserve("A", vec!["src/a.rs".into()], None).unwrap();
        store.leave("A").unwrap();

        let all = store.list_reservations().unwrap();
        assert!(all.is_empty());
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
        store.join("A", None).unwrap();
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
        store.join("good", None).unwrap();
        // Write corrupt registry entry
        fs::write(store.registration_path("bad"), "NOT VALID JSON").unwrap();
        let err = store.list_agents().unwrap_err();
        assert!(matches!(
            err,
            crate::error::TakError::MeshCorruptFile(_, _)
        ));
    }

    #[test]
    fn inbox_errors_on_corrupt_message() {
        let (_dir, store) = setup_mesh();
        store.join("A", None).unwrap();
        // Write corrupt message to inbox
        let inbox = store.agent_inbox_dir("A");
        fs::write(inbox.join("corrupt.json"), "NOT VALID JSON").unwrap();
        let err = store.inbox("A", false).unwrap_err();
        assert!(matches!(
            err,
            crate::error::TakError::MeshCorruptFile(_, _)
        ));
    }
}
