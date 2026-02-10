use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Result, TakError};
use crate::store::paths::normalized_paths_conflict;

// ---------------------------------------------------------------------------
// Shared enums (moved from store::blackboard on CoordinationDb migration)
// ---------------------------------------------------------------------------

/// Lifecycle state of a blackboard note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum BlackboardStatus {
    Open,
    Closed,
}

impl std::fmt::Display for BlackboardStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DbRegistration {
    pub name: String,
    pub generation: i64,
    pub session_id: String,
    pub cwd: String,
    pub pid: Option<u32>,
    pub host: Option<String>,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbMessage {
    pub id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub text: String,
    pub reply_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
    pub acked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbReservation {
    pub id: i64,
    pub agent: String,
    pub generation: i64,
    pub path: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbNote {
    pub id: i64,
    pub from_agent: String,
    pub message: String,
    pub status: String,
    pub tags: Vec<String>,
    pub task_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_by: Option<String>,
    pub closed_reason: Option<String>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbEvent {
    pub id: i64,
    pub agent: Option<String>,
    pub event_type: String,
    pub target: Option<String>,
    pub preview: Option<String>,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Helper: parse RFC 3339 timestamps from SQLite TEXT columns
// ---------------------------------------------------------------------------

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_dt_opt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.map(|v| parse_dt(&v))
}

// ---------------------------------------------------------------------------
// CoordinationDb
// ---------------------------------------------------------------------------

pub struct CoordinationDb {
    conn: Connection,
}

impl CoordinationDb {
    /// Open (or create) the coordination database at the given file path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\
             PRAGMA foreign_keys=ON;\
             PRAGMA busy_timeout=5000;",
        )?;
        let db = Self { conn };
        db.create_tables()?;
        Ok(db)
    }

    /// Open an in-memory database (for tests).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;\
             PRAGMA busy_timeout=5000;",
        )?;
        let db = Self { conn };
        db.create_tables()?;
        Ok(db)
    }

    /// Convenience: open `<repo_root>/.tak/runtime/coordination.db`,
    /// creating the runtime directory if needed.
    pub fn from_repo(repo_root: &Path) -> Result<Self> {
        let runtime_dir = repo_root.join(".tak").join("runtime");
        fs::create_dir_all(&runtime_dir)?;
        Self::open(&runtime_dir.join("coordination.db"))
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agents (
                name TEXT PRIMARY KEY,
                generation INTEGER NOT NULL DEFAULT 1,
                session_id TEXT NOT NULL,
                cwd TEXT NOT NULL,
                pid INTEGER,
                host TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                from_agent TEXT NOT NULL,
                to_agent TEXT NOT NULL,
                text TEXT NOT NULL,
                reply_to TEXT,
                created_at TEXT NOT NULL,
                read_at TEXT,
                acked_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_messages_to_agent_acked
                ON messages(to_agent, acked_at);

            CREATE TABLE IF NOT EXISTS reservations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent TEXT NOT NULL REFERENCES agents(name) ON DELETE CASCADE,
                generation INTEGER NOT NULL,
                path TEXT NOT NULL,
                reason TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_reservations_path
                ON reservations(path);
            CREATE INDEX IF NOT EXISTS idx_reservations_agent
                ON reservations(agent);
            CREATE INDEX IF NOT EXISTS idx_reservations_expires
                ON reservations(expires_at);

            CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_agent TEXT NOT NULL,
                message TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                closed_by TEXT,
                closed_reason TEXT,
                closed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS note_tags (
                note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY (note_id, tag)
            );

            CREATE TABLE IF NOT EXISTS note_tasks (
                note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                task_id TEXT NOT NULL,
                PRIMARY KEY (note_id, task_id)
            );

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent TEXT,
                event_type TEXT NOT NULL,
                target TEXT,
                preview TEXT,
                detail TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_created
                ON events(created_at);",
        )?;
        Ok(())
    }

    /// Expose the raw connection (for tests or advanced usage).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // -----------------------------------------------------------------------
    // Agent registry
    // -----------------------------------------------------------------------

    /// Register or re-register an agent. Generation is bumped to MAX(current)+1.
    pub fn join_agent(
        &self,
        name: &str,
        session_id: &str,
        cwd: &str,
        pid: Option<u32>,
        host: Option<&str>,
    ) -> Result<DbRegistration> {
        let now = Utc::now().to_rfc3339();

        // Compute next generation: MAX(existing) + 1, or 1 if new.
        let current_gen: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) FROM agents WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let next_gen = current_gen + 1;

        self.conn.execute(
            "INSERT INTO agents (name, generation, session_id, cwd, pid, host, status, started_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7, '{}')
             ON CONFLICT(name) DO UPDATE SET
                generation = ?2,
                session_id = ?3,
                cwd = ?4,
                pid = ?5,
                host = ?6,
                status = 'active',
                started_at = ?7,
                updated_at = ?7,
                metadata = '{}'",
            params![name, next_gen, session_id, cwd, pid.map(|p| p as i64), host, &now],
        )?;

        self.get_agent(name)
    }

    /// Unregister an agent. FK CASCADE removes their reservations.
    pub fn leave_agent(&self, name: &str) -> Result<()> {
        let changes = self
            .conn
            .execute("DELETE FROM agents WHERE name = ?1", params![name])?;
        if changes == 0 {
            return Err(TakError::MeshAgentNotFound(name.to_string()));
        }
        Ok(())
    }

    /// List all registered agents, ordered by name.
    pub fn list_agents(&self) -> Result<Vec<DbRegistration>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, generation, session_id, cwd, pid, host, status, started_at, updated_at, metadata
             FROM agents ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DbRegistration {
                name: row.get(0)?,
                generation: row.get(1)?,
                session_id: row.get(2)?,
                cwd: row.get(3)?,
                pid: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                host: row.get(5)?,
                status: row.get(6)?,
                started_at: parse_dt(&row.get::<_, String>(7)?),
                updated_at: parse_dt(&row.get::<_, String>(8)?),
                metadata: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Get a single agent by name.
    pub fn get_agent(&self, name: &str) -> Result<DbRegistration> {
        let mut stmt = self.conn.prepare(
            "SELECT name, generation, session_id, cwd, pid, host, status, started_at, updated_at, metadata
             FROM agents WHERE name = ?1",
        )?;
        stmt.query_row(params![name], |row| {
            Ok(DbRegistration {
                name: row.get(0)?,
                generation: row.get(1)?,
                session_id: row.get(2)?,
                cwd: row.get(3)?,
                pid: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                host: row.get(5)?,
                status: row.get(6)?,
                started_at: parse_dt(&row.get::<_, String>(7)?),
                updated_at: parse_dt(&row.get::<_, String>(8)?),
                metadata: row.get(9)?,
            })
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => TakError::MeshAgentNotFound(name.to_string()),
            other => TakError::Db(other),
        })
    }

    /// Update the agent's heartbeat timestamp (and optionally pid/host).
    pub fn heartbeat_agent(&self, name: &str, pid: Option<u32>, host: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let changes = self.conn.execute(
            "UPDATE agents SET updated_at = ?2, pid = COALESCE(?3, pid), host = COALESCE(?4, host)
             WHERE name = ?1",
            params![name, &now, pid.map(|p| p as i64), host],
        )?;
        if changes == 0 {
            return Err(TakError::MeshAgentNotFound(name.to_string()));
        }
        Ok(())
    }

    /// Delete agents whose `updated_at` is older than `max_age_secs` seconds ago.
    /// Returns the names of removed agents.
    pub fn cleanup_stale_agents(&self, max_age_secs: i64) -> Result<Vec<String>> {
        let cutoff = (Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();

        let mut stmt = self
            .conn
            .prepare("SELECT name FROM agents WHERE updated_at < ?1")?;
        let names: Vec<String> = stmt
            .query_map(params![&cutoff], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if !names.is_empty() {
            self.conn
                .execute("DELETE FROM agents WHERE updated_at < ?1", params![&cutoff])?;
        }

        Ok(names)
    }

    // -----------------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------------

    /// Send a direct message from one agent to another.
    pub fn send_message(
        &self,
        from: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<DbMessage> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO messages (id, from_agent, to_agent, text, reply_to, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&id, from, to, text, reply_to, &now],
        )?;

        Ok(DbMessage {
            id,
            from_agent: from.to_string(),
            to_agent: to.to_string(),
            text: text.to_string(),
            reply_to: reply_to.map(|s| s.to_string()),
            created_at: parse_dt(&now),
            read_at: None,
            acked_at: None,
        })
    }

    /// Read unacked messages for an agent, oldest first.
    pub fn read_inbox(&self, name: &str) -> Result<Vec<DbMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_agent, to_agent, text, reply_to, created_at, read_at, acked_at
             FROM messages
             WHERE to_agent = ?1 AND acked_at IS NULL
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![name], |row| {
            Ok(DbMessage {
                id: row.get(0)?,
                from_agent: row.get(1)?,
                to_agent: row.get(2)?,
                text: row.get(3)?,
                reply_to: row.get(4)?,
                created_at: parse_dt(&row.get::<_, String>(5)?),
                read_at: parse_dt_opt(row.get(6)?),
                acked_at: parse_dt_opt(row.get(7)?),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Acknowledge specific messages by ID.
    pub fn ack_messages(&self, name: &str, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = Utc::now().to_rfc3339();
        let mut count = 0usize;
        for id in ids {
            count += self.conn.execute(
                "UPDATE messages SET acked_at = ?1 WHERE id = ?2 AND to_agent = ?3 AND acked_at IS NULL",
                params![&now, id, name],
            )?;
        }
        Ok(count)
    }

    /// Acknowledge all unacked messages for an agent.
    pub fn ack_all_messages(&self, name: &str) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let count = self.conn.execute(
            "UPDATE messages SET acked_at = ?1 WHERE to_agent = ?2 AND acked_at IS NULL",
            params![&now, name],
        )?;
        Ok(count)
    }

    /// Broadcast a message to all registered agents except the sender.
    /// Returns the list of sent messages.
    pub fn broadcast_message(&self, from: &str, text: &str) -> Result<Vec<DbMessage>> {
        let tx = self.conn.unchecked_transaction()?;

        let recipients: Vec<String> = {
            let mut stmt = tx.prepare("SELECT name FROM agents WHERE name != ?1 ORDER BY name")?;
            stmt.query_map(params![from], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        let now = Utc::now().to_rfc3339();
        let mut messages = Vec::with_capacity(recipients.len());

        for to in &recipients {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO messages (id, from_agent, to_agent, text, reply_to, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![&id, from, to, text, &now],
            )?;
            messages.push(DbMessage {
                id,
                from_agent: from.to_string(),
                to_agent: to.clone(),
                text: text.to_string(),
                reply_to: None,
                created_at: parse_dt(&now),
                read_at: None,
                acked_at: None,
            });
        }

        tx.commit()?;
        Ok(messages)
    }

    // -----------------------------------------------------------------------
    // Reservations
    // -----------------------------------------------------------------------

    /// Reserve a file path for an agent.
    ///
    /// Performs in order:
    /// 1. Generation fence check (agent must exist, generation must match)
    /// 2. Prune expired reservations
    /// 3. Conflict check (prefix-based via `normalized_paths_conflict`)
    /// 4. Insert reservation
    pub fn reserve(
        &self,
        agent: &str,
        generation: i64,
        path: &str,
        reason: Option<&str>,
        ttl_secs: i64,
    ) -> Result<DbReservation> {
        let tx = self.conn.unchecked_transaction()?;

        // 1. Generation fence
        let current_gen: i64 = tx
            .query_row(
                "SELECT generation FROM agents WHERE name = ?1",
                params![agent],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    TakError::MeshAgentNotFound(agent.to_string())
                }
                other => TakError::Db(other),
            })?;

        if current_gen != generation {
            return Err(TakError::MeshStaleGeneration {
                agent: agent.to_string(),
                expected: current_gen,
                got: generation,
            });
        }

        // 2. Prune expired reservations
        let now_str = Utc::now().to_rfc3339();
        tx.execute(
            "DELETE FROM reservations WHERE expires_at <= ?1",
            params![&now_str],
        )?;

        // 3. Conflict check: load all live reservations, then Rust-side check
        {
            let mut stmt = tx.prepare(
                "SELECT agent, path, reason, created_at FROM reservations WHERE agent != ?1",
            )?;
            let rows: Vec<(String, String, Option<String>, String)> = stmt
                .query_map(params![agent], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            for (owner, held_path, held_reason, created_at_str) in &rows {
                if normalized_paths_conflict(path, held_path) {
                    let created_at = parse_dt(created_at_str);
                    let age = Utc::now().signed_duration_since(created_at).num_seconds();
                    return Err(TakError::MeshReservationConflict {
                        requested_path: path.to_string(),
                        held_path: held_path.clone(),
                        owner: owner.clone(),
                        reason: held_reason.clone().unwrap_or_else(|| "(none)".to_string()),
                        age_secs: age,
                    });
                }
            }
        }

        // Also remove this agent's existing reservation for the same path
        // (allow re-reservation / update).
        tx.execute(
            "DELETE FROM reservations WHERE agent = ?1 AND path = ?2",
            params![agent, path],
        )?;

        // 4. Insert
        let now = Utc::now();
        let created_at = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339();

        tx.execute(
            "INSERT INTO reservations (agent, generation, path, reason, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![agent, generation, path, reason, &created_at, &expires_at],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;

        Ok(DbReservation {
            id,
            agent: agent.to_string(),
            generation,
            path: path.to_string(),
            reason: reason.map(|s| s.to_string()),
            created_at: parse_dt(&created_at),
            expires_at: parse_dt(&expires_at),
        })
    }

    /// Release a specific path reservation for an agent.
    pub fn release_path(&self, agent: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM reservations WHERE agent = ?1 AND path = ?2",
            params![agent, path],
        )?;
        Ok(())
    }

    /// Release all reservations for an agent.
    pub fn release_all(&self, agent: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM reservations WHERE agent = ?1", params![agent])?;
        Ok(())
    }

    /// List all non-expired reservations.
    pub fn list_reservations(&self) -> Result<Vec<DbReservation>> {
        let now = Utc::now().to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT id, agent, generation, path, reason, created_at, expires_at
             FROM reservations
             WHERE expires_at > ?1
             ORDER BY id",
        )?;
        let rows = stmt.query_map(params![&now], |row| {
            Ok(DbReservation {
                id: row.get(0)?,
                agent: row.get(1)?,
                generation: row.get(2)?,
                path: row.get(3)?,
                reason: row.get(4)?,
                created_at: parse_dt(&row.get::<_, String>(5)?),
                expires_at: parse_dt(&row.get::<_, String>(6)?),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Notes (blackboard)
    // -----------------------------------------------------------------------

    /// Post a new note.
    pub fn post_note(
        &self,
        from: &str,
        message: &str,
        tags: &[String],
        task_ids: &[String],
    ) -> Result<DbNote> {
        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();

        tx.execute(
            "INSERT INTO notes (from_agent, message, status, created_at, updated_at)
             VALUES (?1, ?2, 'open', ?3, ?3)",
            params![from, message, &now],
        )?;
        let id = tx.last_insert_rowid();

        for tag in tags {
            tx.execute(
                "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?1, ?2)",
                params![id, tag],
            )?;
        }
        for task_id in task_ids {
            tx.execute(
                "INSERT OR IGNORE INTO note_tasks (note_id, task_id) VALUES (?1, ?2)",
                params![id, task_id],
            )?;
        }

        tx.commit()?;

        // Return with sorted tags/task_ids to match DB retrieval order
        let mut sorted_tags = tags.to_vec();
        sorted_tags.sort();
        let mut sorted_task_ids = task_ids.to_vec();
        sorted_task_ids.sort();

        Ok(DbNote {
            id,
            from_agent: from.to_string(),
            message: message.to_string(),
            status: "open".to_string(),
            tags: sorted_tags,
            task_ids: sorted_task_ids,
            created_at: parse_dt(&now),
            updated_at: parse_dt(&now),
            closed_by: None,
            closed_reason: None,
            closed_at: None,
        })
    }

    /// List notes with optional filters.
    pub fn list_notes(
        &self,
        status: Option<&str>,
        tag: Option<&str>,
        task_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<DbNote>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;
        let mut joins = String::new();

        if let Some(s) = status {
            conditions.push(format!("n.status = ?{param_idx}"));
            param_values.push(Box::new(s.to_string()));
            param_idx += 1;
        }
        if let Some(t) = tag {
            joins.push_str(&format!(
                " JOIN note_tags nt ON nt.note_id = n.id AND nt.tag = ?{param_idx}"
            ));
            param_values.push(Box::new(t.to_string()));
            param_idx += 1;
        }
        if let Some(tid) = task_id {
            joins.push_str(&format!(
                " JOIN note_tasks nta ON nta.note_id = n.id AND nta.task_id = ?{param_idx}"
            ));
            param_values.push(Box::new(tid.to_string()));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let limit_clause = if let Some(lim) = limit {
            param_values.push(Box::new(lim as i64));
            format!(" LIMIT ?{param_idx}")
        } else {
            String::new()
        };

        let sql = format!(
            "SELECT DISTINCT n.id, n.from_agent, n.message, n.status, n.created_at, n.updated_at, \
             n.closed_by, n.closed_reason, n.closed_at \
             FROM notes n{joins}{where_clause} ORDER BY n.id DESC{limit_clause}"
        );

        let params_slice: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let partial: Vec<DbNote> = stmt
            .query_map(params_slice.as_slice(), |row| {
                Ok(DbNote {
                    id: row.get(0)?,
                    from_agent: row.get(1)?,
                    message: row.get(2)?,
                    status: row.get(3)?,
                    tags: vec![],
                    task_ids: vec![],
                    created_at: parse_dt(&row.get::<_, String>(4)?),
                    updated_at: parse_dt(&row.get::<_, String>(5)?),
                    closed_by: row.get(6)?,
                    closed_reason: row.get(7)?,
                    closed_at: parse_dt_opt(row.get(8)?),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut notes = Vec::with_capacity(partial.len());
        for mut note in partial {
            note.tags = self.note_tags(note.id)?;
            note.task_ids = self.note_task_ids(note.id)?;
            notes.push(note);
        }
        Ok(notes)
    }

    /// Get a single note by ID.
    pub fn get_note(&self, id: i64) -> Result<DbNote> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_agent, message, status, created_at, updated_at, \
             closed_by, closed_reason, closed_at FROM notes WHERE id = ?1",
        )?;
        let note = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => TakError::BlackboardNoteNotFound(id as u64),
                other => TakError::Db(other),
            })?;

        let (
            nid,
            from_agent,
            message,
            status,
            created_at,
            updated_at,
            closed_by,
            closed_reason,
            closed_at,
        ) = note;
        let tags = self.note_tags(nid)?;
        let task_ids = self.note_task_ids(nid)?;

        Ok(DbNote {
            id: nid,
            from_agent,
            message,
            status,
            tags,
            task_ids,
            created_at: parse_dt(&created_at),
            updated_at: parse_dt(&updated_at),
            closed_by,
            closed_reason,
            closed_at: parse_dt_opt(closed_at),
        })
    }

    /// Close a note.
    pub fn close_note(&self, id: i64, by: &str, reason: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let changes = self.conn.execute(
            "UPDATE notes SET status = 'closed', closed_by = ?2, closed_reason = ?3, \
             closed_at = ?4, updated_at = ?4 WHERE id = ?1",
            params![id, by, reason, &now],
        )?;
        if changes == 0 {
            return Err(TakError::BlackboardNoteNotFound(id as u64));
        }
        Ok(())
    }

    /// Reopen a note (clear closed fields).
    pub fn reopen_note(&self, id: i64, by: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        // `by` is recorded in the updated_at as side-effect context;
        // the main semantic is clearing the closed fields.
        let _ = by;
        let changes = self.conn.execute(
            "UPDATE notes SET status = 'open', closed_by = NULL, closed_reason = NULL, \
             closed_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, &now],
        )?;
        if changes == 0 {
            return Err(TakError::BlackboardNoteNotFound(id as u64));
        }
        Ok(())
    }

    // Helper: load tags for a note
    fn note_tags(&self, note_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM note_tags WHERE note_id = ?1 ORDER BY tag")?;
        let tags = stmt
            .query_map(params![note_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    // Helper: load task_ids for a note
    fn note_task_ids(&self, note_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task_id FROM note_tasks WHERE note_id = ?1 ORDER BY task_id")?;
        let ids = stmt
            .query_map(params![note_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    // -----------------------------------------------------------------------
    // Events (activity feed)
    // -----------------------------------------------------------------------

    /// Append an event to the feed.
    pub fn append_event(
        &self,
        agent: Option<&str>,
        event_type: &str,
        target: Option<&str>,
        preview: Option<&str>,
    ) -> Result<DbEvent> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO events (agent, event_type, target, preview, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agent, event_type, target, preview, &now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(DbEvent {
            id,
            agent: agent.map(|s| s.to_string()),
            event_type: event_type.to_string(),
            target: target.map(|s| s.to_string()),
            preview: preview.map(|s| s.to_string()),
            detail: None,
            created_at: parse_dt(&now),
        })
    }

    /// Read events, most recent first. Optional limit.
    pub fn read_events(&self, limit: Option<u32>) -> Result<Vec<DbEvent>> {
        let sql = if let Some(lim) = limit {
            format!(
                "SELECT id, agent, event_type, target, preview, detail, created_at \
                 FROM events ORDER BY id DESC LIMIT {}",
                lim
            )
        } else {
            "SELECT id, agent, event_type, target, preview, detail, created_at \
             FROM events ORDER BY id DESC"
                .to_string()
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(DbEvent {
                id: row.get(0)?,
                agent: row.get(1)?,
                event_type: row.get(2)?,
                target: row.get(3)?,
                preview: row.get(4)?,
                detail: row.get(5)?,
                created_at: parse_dt(&row.get::<_, String>(6)?),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Prune events older than `max_age_secs`. Returns the number deleted.
    pub fn prune_events(&self, max_age_secs: i64) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();
        let count = self
            .conn
            .execute("DELETE FROM events WHERE created_at < ?1", params![&cutoff])?;
        Ok(count)
    }

    /// Prune events keeping only the most recent `max_events`. Returns the number deleted.
    pub fn prune_events_by_count(&self, max_events: u32) -> Result<usize> {
        let count = self.conn.execute(
            "DELETE FROM events WHERE id NOT IN (
                SELECT id FROM events ORDER BY id DESC LIMIT ?1
            )",
            params![max_events],
        )?;
        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Stale cleanup (combined transaction)
    // -----------------------------------------------------------------------

    /// Run combined cleanup in a single transaction:
    /// 1. Delete stale agents (FK CASCADE removes reservations)
    /// 2. Delete old acked messages
    /// 3. Delete old events
    /// 4. Delete expired reservations
    ///
    /// Returns (stale_agents, acked_msgs_deleted, events_deleted, expired_reservations_deleted).
    pub fn cleanup_all(
        &self,
        agent_ttl_secs: i64,
        msg_ttl_secs: i64,
        event_ttl_secs: i64,
    ) -> Result<(Vec<String>, usize, usize, usize)> {
        let tx = self.conn.unchecked_transaction()?;

        let now = Utc::now();

        // 1. Stale agents
        let agent_cutoff = (now - chrono::Duration::seconds(agent_ttl_secs)).to_rfc3339();
        let mut stmt = tx.prepare("SELECT name FROM agents WHERE updated_at < ?1")?;
        let stale_agents: Vec<String> = stmt
            .query_map(params![&agent_cutoff], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        if !stale_agents.is_empty() {
            tx.execute(
                "DELETE FROM agents WHERE updated_at < ?1",
                params![&agent_cutoff],
            )?;
        }

        // 2. Old acked messages
        let msg_cutoff = (now - chrono::Duration::seconds(msg_ttl_secs)).to_rfc3339();
        let msgs_deleted = tx.execute(
            "DELETE FROM messages WHERE acked_at IS NOT NULL AND acked_at < ?1",
            params![&msg_cutoff],
        )?;

        // 3. Old events
        let event_cutoff = (now - chrono::Duration::seconds(event_ttl_secs)).to_rfc3339();
        let events_deleted = tx.execute(
            "DELETE FROM events WHERE created_at < ?1",
            params![&event_cutoff],
        )?;

        // 4. Expired reservations
        let now_str = now.to_rfc3339();
        let reservations_deleted = tx.execute(
            "DELETE FROM reservations WHERE expires_at <= ?1",
            params![&now_str],
        )?;

        tx.commit()?;

        Ok((
            stale_agents,
            msgs_deleted,
            events_deleted,
            reservations_deleted,
        ))
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Schema / smoke tests
    // -----------------------------------------------------------------------

    #[test]
    fn schema_tables_exist() {
        let db = CoordinationDb::open_memory().unwrap();
        let tables: Vec<String> = db
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"agents".to_string()));
        assert!(tables.contains(&"messages".to_string()));
        assert!(tables.contains(&"reservations".to_string()));
        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"note_tags".to_string()));
        assert!(tables.contains(&"note_tasks".to_string()));
        assert!(tables.contains(&"events".to_string()));
    }

    #[test]
    fn foreign_keys_enabled() {
        let db = CoordinationDb::open_memory().unwrap();
        let fk: i64 = db
            .conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    // -----------------------------------------------------------------------
    // Agent registry
    // -----------------------------------------------------------------------

    #[test]
    fn agent_join_and_list() {
        let db = CoordinationDb::open_memory().unwrap();

        let reg = db
            .join_agent("agent-a", "sess-1", "/tmp", Some(1234), Some("host-1"))
            .unwrap();
        assert_eq!(reg.name, "agent-a");
        assert_eq!(reg.generation, 1);
        assert_eq!(reg.session_id, "sess-1");
        assert_eq!(reg.pid, Some(1234));
        assert_eq!(reg.host.as_deref(), Some("host-1"));
        assert_eq!(reg.status, "active");

        let agents = db.list_agents().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "agent-a");
    }

    #[test]
    fn agent_rejoin_bumps_generation() {
        let db = CoordinationDb::open_memory().unwrap();

        let r1 = db
            .join_agent("agent-a", "sess-1", "/tmp", None, None)
            .unwrap();
        assert_eq!(r1.generation, 1);

        let r2 = db
            .join_agent("agent-a", "sess-2", "/home", None, None)
            .unwrap();
        assert_eq!(r2.generation, 2);
        assert_eq!(r2.session_id, "sess-2");
        assert_eq!(r2.cwd, "/home");
    }

    #[test]
    fn agent_leave_removes() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("agent-a", "s", "/", None, None).unwrap();
        db.leave_agent("agent-a").unwrap();
        assert!(db.list_agents().unwrap().is_empty());
    }

    #[test]
    fn agent_leave_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.leave_agent("ghost").unwrap_err();
        assert_eq!(err.code(), "mesh_agent_not_found");
    }

    #[test]
    fn agent_get() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("agent-a", "s1", "/a", Some(42), None)
            .unwrap();
        let a = db.get_agent("agent-a").unwrap();
        assert_eq!(a.name, "agent-a");
        assert_eq!(a.pid, Some(42));
    }

    #[test]
    fn agent_get_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.get_agent("ghost").unwrap_err();
        assert_eq!(err.code(), "mesh_agent_not_found");
    }

    #[test]
    fn agent_heartbeat() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("agent-a", "s", "/", None, None).unwrap();

        let before = db.get_agent("agent-a").unwrap().updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        db.heartbeat_agent("agent-a", Some(999), None).unwrap();
        let after = db.get_agent("agent-a").unwrap();
        assert!(after.updated_at >= before);
        assert_eq!(after.pid, Some(999));
    }

    #[test]
    fn agent_heartbeat_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.heartbeat_agent("ghost", None, None).unwrap_err();
        assert_eq!(err.code(), "mesh_agent_not_found");
    }

    #[test]
    fn agent_cleanup_stale() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("old", "s", "/", None, None).unwrap();

        // Force the updated_at to be old
        db.conn
            .execute(
                "UPDATE agents SET updated_at = datetime('now', '-120 seconds') WHERE name = 'old'",
                [],
            )
            .unwrap();

        db.join_agent("new", "s", "/", None, None).unwrap();

        let removed = db.cleanup_stale_agents(60).unwrap();
        assert_eq!(removed, vec!["old"]);
        assert_eq!(db.list_agents().unwrap().len(), 1);
        assert_eq!(db.list_agents().unwrap()[0].name, "new");
    }

    #[test]
    fn agent_leave_cascades_reservations() {
        let db = CoordinationDb::open_memory().unwrap();
        let reg = db.join_agent("a", "s", "/", None, None).unwrap();
        db.reserve("a", reg.generation, "src/foo.rs", None, 3600)
            .unwrap();
        assert_eq!(db.list_reservations().unwrap().len(), 1);

        db.leave_agent("a").unwrap();
        assert!(db.list_reservations().unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------------

    #[test]
    fn message_send_and_read_inbox() {
        let db = CoordinationDb::open_memory().unwrap();

        let msg = db.send_message("alice", "bob", "hello bob", None).unwrap();
        assert_eq!(msg.from_agent, "alice");
        assert_eq!(msg.to_agent, "bob");
        assert_eq!(msg.text, "hello bob");
        assert!(msg.acked_at.is_none());

        let inbox = db.read_inbox("bob").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "hello bob");

        // alice's inbox is empty
        assert!(db.read_inbox("alice").unwrap().is_empty());
    }

    #[test]
    fn message_ack_specific() {
        let db = CoordinationDb::open_memory().unwrap();
        let m1 = db.send_message("a", "b", "msg1", None).unwrap();
        let _m2 = db.send_message("a", "b", "msg2", None).unwrap();

        let acked = db.ack_messages("b", &[m1.id.clone()]).unwrap();
        assert_eq!(acked, 1);

        let inbox = db.read_inbox("b").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "msg2");
    }

    #[test]
    fn message_ack_all() {
        let db = CoordinationDb::open_memory().unwrap();
        db.send_message("a", "b", "m1", None).unwrap();
        db.send_message("a", "b", "m2", None).unwrap();

        let acked = db.ack_all_messages("b").unwrap();
        assert_eq!(acked, 2);
        assert!(db.read_inbox("b").unwrap().is_empty());
    }

    #[test]
    fn message_ack_empty_ids() {
        let db = CoordinationDb::open_memory().unwrap();
        let acked = db.ack_messages("nobody", &[]).unwrap();
        assert_eq!(acked, 0);
    }

    #[test]
    fn message_reply_to() {
        let db = CoordinationDb::open_memory().unwrap();
        let m1 = db.send_message("a", "b", "original", None).unwrap();
        let m2 = db.send_message("b", "a", "reply", Some(&m1.id)).unwrap();
        assert_eq!(m2.reply_to.as_deref(), Some(m1.id.as_str()));
    }

    #[test]
    fn message_broadcast() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("a", "s", "/", None, None).unwrap();
        db.join_agent("b", "s", "/", None, None).unwrap();
        db.join_agent("c", "s", "/", None, None).unwrap();

        let msgs = db.broadcast_message("a", "hello all").unwrap();
        assert_eq!(msgs.len(), 2);

        let b_inbox = db.read_inbox("b").unwrap();
        assert_eq!(b_inbox.len(), 1);
        assert_eq!(b_inbox[0].text, "hello all");

        let c_inbox = db.read_inbox("c").unwrap();
        assert_eq!(c_inbox.len(), 1);

        // sender does not receive own broadcast
        assert!(db.read_inbox("a").unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // Reservations
    // -----------------------------------------------------------------------

    #[test]
    fn reservation_basic_lifecycle() {
        let db = CoordinationDb::open_memory().unwrap();
        let reg = db.join_agent("a", "s", "/", None, None).unwrap();

        let res = db
            .reserve("a", reg.generation, "src/main.rs", Some("editing"), 3600)
            .unwrap();
        assert_eq!(res.agent, "a");
        assert_eq!(res.path, "src/main.rs");
        assert_eq!(res.reason.as_deref(), Some("editing"));

        let all = db.list_reservations().unwrap();
        assert_eq!(all.len(), 1);

        db.release_path("a", "src/main.rs").unwrap();
        assert!(db.list_reservations().unwrap().is_empty());
    }

    #[test]
    fn reservation_conflict_detected() {
        let db = CoordinationDb::open_memory().unwrap();
        let ra = db.join_agent("a", "s", "/", None, None).unwrap();
        let rb = db.join_agent("b", "s", "/", None, None).unwrap();

        db.reserve("a", ra.generation, "src/store", Some("refactor"), 3600)
            .unwrap();

        // b tries to reserve a path under src/store — conflict
        let err = db
            .reserve("b", rb.generation, "src/store/mesh.rs", None, 3600)
            .unwrap_err();
        assert_eq!(err.code(), "mesh_reservation_conflict");
    }

    #[test]
    fn reservation_same_agent_no_conflict() {
        let db = CoordinationDb::open_memory().unwrap();
        let reg = db.join_agent("a", "s", "/", None, None).unwrap();

        db.reserve("a", reg.generation, "src/main.rs", None, 3600)
            .unwrap();
        // Same agent, same path — should succeed (re-reservation)
        db.reserve(
            "a",
            reg.generation,
            "src/main.rs",
            Some("updated reason"),
            3600,
        )
        .unwrap();

        let all = db.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].reason.as_deref(), Some("updated reason"));
    }

    #[test]
    fn reservation_stale_generation_rejected() {
        let db = CoordinationDb::open_memory().unwrap();
        let r1 = db.join_agent("a", "s1", "/", None, None).unwrap();
        assert_eq!(r1.generation, 1);

        // Re-join bumps generation
        let _r2 = db.join_agent("a", "s2", "/", None, None).unwrap();

        // Try to reserve with old generation
        let err = db.reserve("a", 1, "src/foo.rs", None, 3600).unwrap_err();
        assert_eq!(err.code(), "mesh_stale_generation");
    }

    #[test]
    fn reservation_agent_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db
            .reserve("ghost", 1, "src/foo.rs", None, 3600)
            .unwrap_err();
        assert_eq!(err.code(), "mesh_agent_not_found");
    }

    #[test]
    fn reservation_expired_excluded_from_list() {
        let db = CoordinationDb::open_memory().unwrap();
        let reg = db.join_agent("a", "s", "/", None, None).unwrap();

        // Insert a reservation that's already expired
        let past = (Utc::now() - chrono::Duration::seconds(100)).to_rfc3339();
        db.conn
            .execute(
                "INSERT INTO reservations (agent, generation, path, reason, created_at, expires_at)
                 VALUES ('a', ?1, 'old/path', NULL, ?2, ?2)",
                params![reg.generation, &past],
            )
            .unwrap();

        assert!(db.list_reservations().unwrap().is_empty());
    }

    #[test]
    fn reservation_expired_cleaned_during_reserve() {
        let db = CoordinationDb::open_memory().unwrap();
        let ra = db.join_agent("a", "s", "/", None, None).unwrap();
        let rb = db.join_agent("b", "s", "/", None, None).unwrap();

        // Agent a holds "src/store" but it's expired
        let past = (Utc::now() - chrono::Duration::seconds(100)).to_rfc3339();
        db.conn
            .execute(
                "INSERT INTO reservations (agent, generation, path, reason, created_at, expires_at)
                 VALUES ('a', ?1, 'src/store', NULL, ?2, ?2)",
                params![ra.generation, &past],
            )
            .unwrap();

        // Agent b should be able to reserve src/store/mesh.rs — expired reservation cleaned up
        db.reserve("b", rb.generation, "src/store/mesh.rs", None, 3600)
            .unwrap();

        let all = db.list_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, "src/store/mesh.rs");
    }

    #[test]
    fn reservation_release_all() {
        let db = CoordinationDb::open_memory().unwrap();
        let reg = db.join_agent("a", "s", "/", None, None).unwrap();

        db.reserve("a", reg.generation, "src/a.rs", None, 3600)
            .unwrap();
        db.reserve("a", reg.generation, "src/b.rs", None, 3600)
            .unwrap();
        assert_eq!(db.list_reservations().unwrap().len(), 2);

        db.release_all("a").unwrap();
        assert!(db.list_reservations().unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // Notes (blackboard)
    // -----------------------------------------------------------------------

    #[test]
    fn note_post_and_get() {
        let db = CoordinationDb::open_memory().unwrap();

        let note = db
            .post_note(
                "alice",
                "need review on auth module",
                &["review".to_string(), "auth".to_string()],
                &["abc123".to_string()],
            )
            .unwrap();
        assert_eq!(note.from_agent, "alice");
        assert_eq!(note.status, "open");
        assert_eq!(note.tags, vec!["auth", "review"]);
        assert_eq!(note.task_ids, vec!["abc123"]);

        let fetched = db.get_note(note.id).unwrap();
        assert_eq!(fetched.message, "need review on auth module");
        assert_eq!(fetched.tags.len(), 2);
    }

    #[test]
    fn note_get_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.get_note(999).unwrap_err();
        assert_eq!(err.code(), "blackboard_note_not_found");
    }

    #[test]
    fn note_close_and_reopen() {
        let db = CoordinationDb::open_memory().unwrap();
        let note = db.post_note("alice", "important note", &[], &[]).unwrap();

        db.close_note(note.id, "bob", Some("resolved")).unwrap();
        let closed = db.get_note(note.id).unwrap();
        assert_eq!(closed.status, "closed");
        assert_eq!(closed.closed_by.as_deref(), Some("bob"));
        assert_eq!(closed.closed_reason.as_deref(), Some("resolved"));
        assert!(closed.closed_at.is_some());

        db.reopen_note(note.id, "alice").unwrap();
        let reopened = db.get_note(note.id).unwrap();
        assert_eq!(reopened.status, "open");
        assert!(reopened.closed_by.is_none());
        assert!(reopened.closed_reason.is_none());
        assert!(reopened.closed_at.is_none());
    }

    #[test]
    fn note_close_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.close_note(999, "bob", None).unwrap_err();
        assert_eq!(err.code(), "blackboard_note_not_found");
    }

    #[test]
    fn note_reopen_not_found() {
        let db = CoordinationDb::open_memory().unwrap();
        let err = db.reopen_note(999, "bob").unwrap_err();
        assert_eq!(err.code(), "blackboard_note_not_found");
    }

    #[test]
    fn note_list_no_filters() {
        let db = CoordinationDb::open_memory().unwrap();
        db.post_note("a", "first", &[], &[]).unwrap();
        db.post_note("b", "second", &[], &[]).unwrap();

        let notes = db.list_notes(None, None, None, None).unwrap();
        assert_eq!(notes.len(), 2);
        // Most recent first
        assert_eq!(notes[0].message, "second");
        assert_eq!(notes[1].message, "first");
    }

    #[test]
    fn note_list_filter_by_status() {
        let db = CoordinationDb::open_memory().unwrap();
        let n1 = db.post_note("a", "open one", &[], &[]).unwrap();
        db.post_note("b", "open two", &[], &[]).unwrap();
        db.close_note(n1.id, "a", None).unwrap();

        let open = db.list_notes(Some("open"), None, None, None).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].message, "open two");

        let closed = db.list_notes(Some("closed"), None, None, None).unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].message, "open one");
    }

    #[test]
    fn note_list_filter_by_tag() {
        let db = CoordinationDb::open_memory().unwrap();
        db.post_note("a", "tagged", &["review".to_string()], &[])
            .unwrap();
        db.post_note("b", "untagged", &[], &[]).unwrap();

        let filtered = db.list_notes(None, Some("review"), None, None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "tagged");
    }

    #[test]
    fn note_list_filter_by_task_id() {
        let db = CoordinationDb::open_memory().unwrap();
        db.post_note("a", "linked", &[], &["task-42".to_string()])
            .unwrap();
        db.post_note("b", "unlinked", &[], &[]).unwrap();

        let filtered = db.list_notes(None, None, Some("task-42"), None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "linked");
    }

    #[test]
    fn note_list_with_limit() {
        let db = CoordinationDb::open_memory().unwrap();
        for i in 0..5 {
            db.post_note("a", &format!("note {i}"), &[], &[]).unwrap();
        }

        let limited = db.list_notes(None, None, None, Some(2)).unwrap();
        assert_eq!(limited.len(), 2);
        // Most recent first
        assert_eq!(limited[0].message, "note 4");
        assert_eq!(limited[1].message, "note 3");
    }

    #[test]
    fn note_list_combined_filters() {
        let db = CoordinationDb::open_memory().unwrap();
        db.post_note(
            "a",
            "match",
            &["review".to_string()],
            &["task-1".to_string()],
        )
        .unwrap();
        db.post_note("b", "tag only", &["review".to_string()], &[])
            .unwrap();
        db.post_note("c", "task only", &[], &["task-1".to_string()])
            .unwrap();

        let filtered = db
            .list_notes(Some("open"), Some("review"), Some("task-1"), None)
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "match");
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    #[test]
    fn event_append_and_read() {
        let db = CoordinationDb::open_memory().unwrap();

        db.append_event(Some("alice"), "join", None, None).unwrap();
        db.append_event(Some("bob"), "reserve", Some("src/foo.rs"), Some("editing"))
            .unwrap();

        let events = db.read_events(None).unwrap();
        assert_eq!(events.len(), 2);
        // Most recent first
        assert_eq!(events[0].event_type, "reserve");
        assert_eq!(events[1].event_type, "join");
    }

    #[test]
    fn event_read_with_limit() {
        let db = CoordinationDb::open_memory().unwrap();
        for i in 0..5 {
            db.append_event(Some("a"), &format!("ev-{i}"), None, None)
                .unwrap();
        }

        let events = db.read_events(Some(2)).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "ev-4");
        assert_eq!(events[1].event_type, "ev-3");
    }

    #[test]
    fn event_prune_by_age() {
        let db = CoordinationDb::open_memory().unwrap();
        db.append_event(Some("a"), "old", None, None).unwrap();

        // Backdate the event
        db.conn
            .execute(
                "UPDATE events SET created_at = datetime('now', '-120 seconds') WHERE event_type = 'old'",
                [],
            )
            .unwrap();

        db.append_event(Some("a"), "new", None, None).unwrap();

        let pruned = db.prune_events(60).unwrap();
        assert_eq!(pruned, 1);

        let events = db.read_events(None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "new");
    }

    #[test]
    fn event_prune_by_count() {
        let db = CoordinationDb::open_memory().unwrap();
        for i in 0..5 {
            db.append_event(Some("a"), &format!("ev-{i}"), None, None)
                .unwrap();
        }

        let pruned = db.prune_events_by_count(2).unwrap();
        assert_eq!(pruned, 3);

        let events = db.read_events(None).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "ev-4");
        assert_eq!(events[1].event_type, "ev-3");
    }

    // -----------------------------------------------------------------------
    // Stale cleanup (combined)
    // -----------------------------------------------------------------------

    #[test]
    fn cleanup_all_combined() {
        let db = CoordinationDb::open_memory().unwrap();

        // 1. Set up a stale agent with a reservation
        let reg = db.join_agent("stale", "s", "/", None, None).unwrap();
        db.reserve("stale", reg.generation, "src/x.rs", None, 3600)
            .unwrap();
        db.conn
            .execute(
                "UPDATE agents SET updated_at = datetime('now', '-200 seconds') WHERE name = 'stale'",
                [],
            )
            .unwrap();

        // A fresh agent
        db.join_agent("fresh", "s", "/", None, None).unwrap();

        // 2. An old acked message
        db.send_message("a", "b", "old msg", None).unwrap();
        db.ack_all_messages("b").unwrap();
        db.conn
            .execute(
                "UPDATE messages SET acked_at = datetime('now', '-200 seconds') WHERE text = 'old msg'",
                [],
            )
            .unwrap();

        // A recent unacked message
        db.send_message("a", "b", "new msg", None).unwrap();

        // 3. An old event
        db.append_event(Some("a"), "old-ev", None, None).unwrap();
        db.conn
            .execute(
                "UPDATE events SET created_at = datetime('now', '-200 seconds') WHERE event_type = 'old-ev'",
                [],
            )
            .unwrap();

        // A recent event
        db.append_event(Some("a"), "new-ev", None, None).unwrap();

        // Run cleanup with 100s TTLs
        let (stale_agents, msgs_del, events_del, _res_del) = db.cleanup_all(100, 100, 100).unwrap();

        assert_eq!(stale_agents, vec!["stale"]);
        assert_eq!(msgs_del, 1);
        assert_eq!(events_del, 1);

        // Verify state
        assert_eq!(db.list_agents().unwrap().len(), 1);
        assert_eq!(db.list_agents().unwrap()[0].name, "fresh");

        let inbox = db.read_inbox("b").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "new msg");

        let events = db.read_events(None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "new-ev");

        // Reservation was cascade-deleted with agent
        assert!(db.list_reservations().unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // from_repo (filesystem test)
    // -----------------------------------------------------------------------

    #[test]
    fn from_repo_creates_runtime_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join(".tak")).unwrap();

        let db = CoordinationDb::from_repo(repo_root).unwrap();
        db.join_agent("test", "s", "/", None, None).unwrap();

        assert!(repo_root.join(".tak/runtime/coordination.db").exists());
    }
}
