use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::model::{Learning, Task};
use crate::task_id::TaskId;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let idx = Self { conn };
        idx.create_tables()?;
        Ok(idx)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let idx = Self { conn };
        idx.create_tables()?;
        Ok(idx)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                kind TEXT NOT NULL DEFAULT 'task',
                parent_id TEXT REFERENCES tasks(id),
                assignee TEXT,
                priority INTEGER,
                estimate TEXT,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dependencies (
                task_id TEXT NOT NULL REFERENCES tasks(id),
                depends_on_id TEXT NOT NULL REFERENCES tasks(id),
                dep_type TEXT,
                reason TEXT,
                PRIMARY KEY (task_id, depends_on_id)
            );
            CREATE TABLE IF NOT EXISTS tags (
                task_id TEXT NOT NULL REFERENCES tasks(id),
                tag TEXT NOT NULL,
                PRIMARY KEY (task_id, tag)
            );
            CREATE TABLE IF NOT EXISTS skills (
                task_id TEXT NOT NULL REFERENCES tasks(id),
                skill TEXT NOT NULL,
                PRIMARY KEY (task_id, skill)
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_kind ON tasks(kind);
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
            CREATE TABLE IF NOT EXISTS learnings (
                id INTEGER PRIMARY KEY,
                numeric_id INTEGER NOT NULL UNIQUE,
                title TEXT NOT NULL,
                description TEXT,
                category TEXT NOT NULL DEFAULT 'insight',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS learning_tags (
                learning_id INTEGER NOT NULL REFERENCES learnings(id),
                tag TEXT NOT NULL,
                PRIMARY KEY (learning_id, tag)
            );
            CREATE TABLE IF NOT EXISTS learning_tasks (
                learning_id INTEGER NOT NULL REFERENCES learnings(id),
                task_id TEXT NOT NULL,
                PRIMARY KEY (learning_id, task_id)
            );
            CREATE INDEX IF NOT EXISTS idx_learning_tasks_task ON learning_tasks(task_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS learnings_fts USING fts5(
                title,
                description,
                content=learnings,
                content_rowid=numeric_id
            );
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    pub fn rebuild(&self, tasks: &[Task]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM skills; DELETE FROM tags; DELETE FROM dependencies; DELETE FROM tasks;",
        )?;

        // Pass 1: insert all task rows with parent_id deferred (avoids FK failures)
        for task in tasks {
            let task_id = TaskId::from(task.id);
            tx.execute(
                "INSERT INTO tasks (id, title, description, status, kind, parent_id, assignee, priority, estimate, attempt_count, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    &task_id, task.title, task.description,
                    task.status.to_string(), task.kind.to_string(),
                    task.assignee,
                    task.planning.priority.map(|p| p.rank() as i64),
                    task.planning.estimate.map(|e| e.to_string()),
                    task.execution.attempt_count,
                    task.created_at.to_rfc3339(), task.updated_at.to_rfc3339(),
                ],
            )?;
        }

        // Pass 2: set parent_id, insert dependencies and tags
        for task in tasks {
            let task_id = TaskId::from(task.id);
            if let Some(parent) = task.parent {
                let parent_id = TaskId::from(parent);
                tx.execute(
                    "UPDATE tasks SET parent_id = ?1 WHERE id = ?2",
                    params![&parent_id, &task_id],
                )?;
            }
            for dep in &task.depends_on {
                let dep_id = TaskId::from(dep.id);
                tx.execute(
                    "INSERT OR IGNORE INTO dependencies (task_id, depends_on_id, dep_type, reason) VALUES (?1, ?2, ?3, ?4)",
                    params![&task_id, &dep_id, dep.dep_type.as_ref().map(|t| t.to_string()), dep.reason],
                )?;
            }
            for tag in &task.tags {
                tx.execute(
                    "INSERT OR IGNORE INTO tags (task_id, tag) VALUES (?1, ?2)",
                    params![&task_id, tag],
                )?;
            }
            for skill in &task.planning.required_skills {
                tx.execute(
                    "INSERT OR IGNORE INTO skills (task_id, skill) VALUES (?1, ?2)",
                    params![&task_id, skill],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, task: &Task) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        let task_id = TaskId::from(task.id);
        let parent_id = task.parent.map(TaskId::from);

        tx.execute(
            "INSERT OR REPLACE INTO tasks (id, title, description, status, kind, parent_id, assignee, priority, estimate, attempt_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &task_id, task.title, task.description,
                task.status.to_string(), task.kind.to_string(),
                parent_id, task.assignee,
                task.planning.priority.map(|p| p.rank() as i64),
                task.planning.estimate.map(|e| e.to_string()),
                task.execution.attempt_count,
                task.created_at.to_rfc3339(), task.updated_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "DELETE FROM dependencies WHERE task_id = ?1",
            params![&task_id],
        )?;
        for dep in &task.depends_on {
            let dep_id = TaskId::from(dep.id);
            tx.execute(
                "INSERT OR IGNORE INTO dependencies (task_id, depends_on_id, dep_type, reason) VALUES (?1, ?2, ?3, ?4)",
                params![&task_id, &dep_id, dep.dep_type.as_ref().map(|t| t.to_string()), dep.reason],
            )?;
        }
        tx.execute("DELETE FROM tags WHERE task_id = ?1", params![&task_id])?;
        for tag in &task.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (task_id, tag) VALUES (?1, ?2)",
                params![&task_id, tag],
            )?;
        }
        tx.execute("DELETE FROM skills WHERE task_id = ?1", params![&task_id])?;
        for skill in &task.planning.required_skills {
            tx.execute(
                "INSERT OR IGNORE INTO skills (task_id, skill) VALUES (?1, ?2)",
                params![&task_id, skill],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: impl Into<TaskId>) -> Result<()> {
        let id = id.into();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM skills WHERE task_id = ?1", params![&id])?;
        tx.execute("DELETE FROM tags WHERE task_id = ?1", params![&id])?;
        tx.execute("DELETE FROM dependencies WHERE task_id = ?1", params![&id])?;
        tx.execute(
            "DELETE FROM dependencies WHERE depends_on_id = ?1",
            params![&id],
        )?;
        tx.execute("DELETE FROM tasks WHERE id = ?1", params![&id])?;
        tx.commit()?;
        Ok(())
    }

    /// Return IDs of available (claimable) tasks.
    ///
    /// `kind=idea` items are intentionally excluded so default `next`/`claim`
    /// flows don't execute raw ideas by accident.
    ///
    /// If `assignee` is Some, also include tasks already assigned to that person.
    /// If `assignee` is None, only unassigned tasks.
    pub fn available(&self, assignee: Option<&str>) -> Result<Vec<TaskId>> {
        let (sql, has_param) = match assignee {
            Some(_) => (
                "SELECT t.id FROM tasks t
                 WHERE t.status = 'pending'
                 AND t.kind != 'idea'
                 AND (t.assignee IS NULL OR t.assignee = ?1)
                 AND NOT EXISTS (
                     SELECT 1 FROM dependencies d
                     JOIN tasks dep ON d.depends_on_id = dep.id
                     WHERE d.task_id = t.id
                     AND dep.status NOT IN ('done', 'cancelled')
                 )
                 ORDER BY COALESCE(t.priority, 4), t.created_at, t.id",
                true,
            ),
            None => (
                "SELECT t.id FROM tasks t
                 WHERE t.status = 'pending'
                 AND t.kind != 'idea'
                 AND t.assignee IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM dependencies d
                     JOIN tasks dep ON d.depends_on_id = dep.id
                     WHERE d.task_id = t.id
                     AND dep.status NOT IN ('done', 'cancelled')
                 )
                 ORDER BY COALESCE(t.priority, 4), t.created_at, t.id",
                false,
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let ids = if has_param {
            stmt.query_map(params![assignee.unwrap()], |row| row.get(0))?
                .collect::<std::result::Result<Vec<TaskId>, _>>()?
        } else {
            stmt.query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<TaskId>, _>>()?
        };
        Ok(ids)
    }

    pub fn blocked(&self) -> Result<Vec<TaskId>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT t.id FROM tasks t
             JOIN dependencies d ON d.task_id = t.id
             JOIN tasks dep ON d.depends_on_id = dep.id
             WHERE t.status = 'pending'
             AND dep.status NOT IN ('done', 'cancelled')
             ORDER BY t.id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    /// Check whether a specific task is blocked by unfinished dependencies.
    pub fn is_blocked(&self, id: impl Into<TaskId>) -> Result<bool> {
        let id = id.into();
        let mut stmt = self.conn.prepare(
            "SELECT EXISTS(
                SELECT 1 FROM dependencies d
                JOIN tasks dep ON d.depends_on_id = dep.id
                WHERE d.task_id = ?1
                AND dep.status NOT IN ('done', 'cancelled')
            )",
        )?;
        let blocked: bool = stmt.query_row(params![&id], |row| row.get(0))?;
        Ok(blocked)
    }

    /// Return IDs of tasks that depend on the given task.
    pub fn dependents_of(&self, id: impl Into<TaskId>) -> Result<Vec<TaskId>> {
        let id = id.into();
        let mut stmt = self.conn.prepare(
            "SELECT task_id FROM dependencies WHERE depends_on_id = ?1 ORDER BY task_id",
        )?;
        let ids = stmt
            .query_map(params![&id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    /// Return IDs of tasks matching a given status and assignee.
    pub fn tasks_by_status_assignee(&self, status: &str, assignee: &str) -> Result<Vec<TaskId>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks
             WHERE status = ?1 AND assignee = ?2
             ORDER BY created_at, id",
        )?;
        let ids = stmt
            .query_map(params![status, assignee], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    pub fn children_of(&self, parent_id: impl Into<TaskId>) -> Result<Vec<TaskId>> {
        let parent_id = parent_id.into();
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE parent_id = ?1 ORDER BY id")?;
        let ids = stmt
            .query_map(params![&parent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    pub fn roots(&self) -> Result<Vec<TaskId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE parent_id IS NULL ORDER BY id")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    /// Return IDs filtered by exact task kind string as stored in SQLite.
    ///
    /// This is intentionally string-based so index-level filtering remains
    /// forward-compatible with newly introduced kinds.
    pub fn ids_by_kind(&self, kind: &str) -> Result<Vec<TaskId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE kind = ?1 ORDER BY id")?;
        let ids = stmt
            .query_map(params![kind], |row| row.get(0))?
            .collect::<std::result::Result<Vec<TaskId>, _>>()?;
        Ok(ids)
    }

    pub fn would_cycle(
        &self,
        task_id: impl Into<TaskId>,
        depends_on_id: impl Into<TaskId>,
    ) -> Result<bool> {
        let task_id = task_id.into();
        let depends_on_id = depends_on_id.into();
        if task_id == depends_on_id {
            return Ok(true);
        }
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE reachable(id) AS (
                SELECT ?1
                UNION
                SELECT d.depends_on_id FROM dependencies d
                JOIN reachable r ON d.task_id = r.id
            )
            SELECT EXISTS(SELECT 1 FROM reachable WHERE id = ?2)",
        )?;
        let exists: bool = stmt.query_row(params![&depends_on_id, &task_id], |row| row.get(0))?;
        Ok(exists)
    }

    /// Check if making `child_id` a child of `parent_id` would create a parent-child cycle.
    pub fn would_parent_cycle(
        &self,
        child_id: impl Into<TaskId>,
        parent_id: impl Into<TaskId>,
    ) -> Result<bool> {
        let child_id = child_id.into();
        let parent_id = parent_id.into();
        if child_id == parent_id {
            return Ok(true);
        }
        // Check if child_id is an ancestor of parent_id via parent edges
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE ancestors(id) AS (
                SELECT parent_id FROM tasks WHERE id = ?1
                UNION
                SELECT t.parent_id FROM tasks t
                JOIN ancestors a ON t.id = a.id
                WHERE t.parent_id IS NOT NULL
            )
            SELECT EXISTS(SELECT 1 FROM ancestors WHERE id = ?2)",
        )?;
        let exists: bool = stmt.query_row(params![&parent_id, &child_id], |row| row.get(0))?;
        Ok(exists)
    }

    pub fn get_fingerprint(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM metadata WHERE key = 'fingerprint'")?;
        let result = stmt.query_row([], |row| row.get::<_, String>(0));
        match result {
            Ok(fp) => Ok(Some(fp)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_fingerprint(&self, fingerprint: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('fingerprint', ?1)",
            params![fingerprint],
        )?;
        Ok(())
    }

    /// Returns true when task-related ID columns use the expected TEXT schema.
    pub fn uses_text_task_id_schema(&self) -> Result<bool> {
        Ok(self.column_type_is_text("tasks", "id")?
            && self.column_type_is_text("tasks", "parent_id")?
            && self.column_type_is_text("dependencies", "task_id")?
            && self.column_type_is_text("dependencies", "depends_on_id")?
            && self.column_type_is_text("tags", "task_id")?
            && self.column_type_is_text("skills", "task_id")?
            && self.column_type_is_text("learning_tasks", "task_id")?)
    }

    fn column_type_is_text(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;

        for row in rows {
            let (name, ty) = row?;
            if name == column {
                return Ok(ty.eq_ignore_ascii_case("TEXT"));
            }
        }

        Ok(false)
    }

    // === Learning index methods ===

    pub fn upsert_learning(&self, learning: &Learning) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Read old data for FTS cleanup, if this learning already exists
        let old: Option<(String, Option<String>)> = {
            let mut stmt = tx.prepare("SELECT title, description FROM learnings WHERE id = ?1")?;
            match stmt.query_row(params![learning.id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            }) {
                Ok(row) => Some(row),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            }
        };

        // Delete old FTS entry with actual content values
        if let Some((old_title, old_desc)) = old {
            tx.execute(
                "INSERT INTO learnings_fts(learnings_fts, rowid, title, description) VALUES('delete', ?1, ?2, ?3)",
                params![learning.id, old_title, old_desc],
            )?;
        }

        // Delete from junction tables and main table
        tx.execute(
            "DELETE FROM learning_tags WHERE learning_id = ?1",
            params![learning.id],
        )?;
        tx.execute(
            "DELETE FROM learning_tasks WHERE learning_id = ?1",
            params![learning.id],
        )?;
        tx.execute("DELETE FROM learnings WHERE id = ?1", params![learning.id])?;

        // Insert new data
        tx.execute(
            "INSERT INTO learnings (id, numeric_id, title, description, category, created_at, updated_at)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                learning.id, learning.title, learning.description,
                learning.category.to_string(),
                learning.created_at.to_rfc3339(), learning.updated_at.to_rfc3339(),
            ],
        )?;

        // Insert into FTS
        tx.execute(
            "INSERT INTO learnings_fts(rowid, title, description) VALUES(?1, ?2, ?3)",
            params![learning.id, learning.title, learning.description],
        )?;

        for tag in &learning.tags {
            tx.execute(
                "INSERT OR IGNORE INTO learning_tags (learning_id, tag) VALUES (?1, ?2)",
                params![learning.id, tag],
            )?;
        }
        for &task_id in &learning.task_ids {
            let task_id = TaskId::from(task_id);
            tx.execute(
                "INSERT OR IGNORE INTO learning_tasks (learning_id, task_id) VALUES (?1, ?2)",
                params![learning.id, &task_id],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn delete_learning(&self, id: u64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Read old data for FTS cleanup
        let old: Option<(String, Option<String>)> = {
            let mut stmt = tx.prepare("SELECT title, description FROM learnings WHERE id = ?1")?;
            match stmt.query_row(params![id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            }) {
                Ok(row) => Some(row),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            }
        };

        if let Some((old_title, old_desc)) = old {
            tx.execute(
                "INSERT INTO learnings_fts(learnings_fts, rowid, title, description) VALUES('delete', ?1, ?2, ?3)",
                params![id, old_title, old_desc],
            )?;
        }

        tx.execute(
            "DELETE FROM learning_tags WHERE learning_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM learning_tasks WHERE learning_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM learnings WHERE id = ?1", params![id])?;

        tx.commit()?;
        Ok(())
    }

    pub fn rebuild_learnings(&self, learnings: &[Learning]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM learning_tags; DELETE FROM learning_tasks; DELETE FROM learnings; DELETE FROM learnings_fts;",
        )?;

        for learning in learnings {
            tx.execute(
                "INSERT INTO learnings (id, numeric_id, title, description, category, created_at, updated_at)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    learning.id, learning.title, learning.description,
                    learning.category.to_string(),
                    learning.created_at.to_rfc3339(), learning.updated_at.to_rfc3339(),
                ],
            )?;
            tx.execute(
                "INSERT INTO learnings_fts(rowid, title, description) VALUES(?1, ?2, ?3)",
                params![learning.id, learning.title, learning.description],
            )?;
            for tag in &learning.tags {
                tx.execute(
                    "INSERT OR IGNORE INTO learning_tags (learning_id, tag) VALUES (?1, ?2)",
                    params![learning.id, tag],
                )?;
            }
            for &task_id in &learning.task_ids {
                let task_id = TaskId::from(task_id);
                tx.execute(
                    "INSERT OR IGNORE INTO learning_tasks (learning_id, task_id) VALUES (?1, ?2)",
                    params![learning.id, &task_id],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Query learnings, optionally filtered by category and/or tag.
    pub fn query_learnings(
        &self,
        category: Option<&str>,
        tag: Option<&str>,
        task_id: Option<u64>,
    ) -> Result<Vec<u64>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        let mut from = "learnings l".to_string();

        if let Some(cat) = category {
            conditions.push(format!("l.category = ?{param_idx}"));
            param_values.push(Box::new(cat.to_string()));
            param_idx += 1;
        }
        if let Some(t) = tag {
            from.push_str(&format!(
                " JOIN learning_tags lt ON lt.learning_id = l.id AND lt.tag = ?{param_idx}"
            ));
            param_values.push(Box::new(t.to_string()));
            param_idx += 1;
        }
        if let Some(tid) = task_id {
            from.push_str(&format!(
                " JOIN learning_tasks lta ON lta.learning_id = l.id AND lta.task_id = ?{param_idx}"
            ));
            param_values.push(Box::new(TaskId::from(tid)));
            let _ = param_idx; // suppress unused warning
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("SELECT l.id FROM {from}{where_clause} ORDER BY l.id");
        let params: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let ids = stmt
            .query_map(params.as_slice(), |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    /// Full-text search for learnings relevant to a given query string.
    /// Returns learning IDs ordered by FTS5 rank (most relevant first).
    pub fn suggest_learnings(&self, query: &str) -> Result<Vec<u64>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }

        // Sanitize: extract alphanumeric tokens, join with spaces for FTS5 implicit AND
        let tokens: Vec<&str> = query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Ok(vec![]);
        }

        // Use OR between tokens for broader matching
        let fts_query = tokens.join(" OR ");

        let mut stmt = self.conn.prepare(
            "SELECT l.id FROM learnings l
             JOIN learnings_fts f ON f.rowid = l.numeric_id
             WHERE learnings_fts MATCH ?1
             ORDER BY rank",
        )?;
        let ids = stmt
            .query_map(params![fts_query], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    /// Return learning IDs linked to a given task ID.
    pub fn learnings_for_task(&self, task_id: impl Into<TaskId>) -> Result<Vec<u64>> {
        let task_id = task_id.into();
        let mut stmt = self.conn.prepare(
            "SELECT learning_id FROM learning_tasks WHERE task_id = ?1 ORDER BY learning_id",
        )?;
        let ids = stmt
            .query_map(params![&task_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn get_learning_fingerprint(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM metadata WHERE key = 'learning_fingerprint'")?;
        let result = stmt.query_row([], |row| row.get::<_, String>(0));
        match result {
            Ok(fp) => Ok(Some(fp)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_learning_fingerprint(&self, fingerprint: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('learning_fingerprint', ?1)",
            params![fingerprint],
        )?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Contract, Dependency, Execution, GitInfo, Kind, Planning, Status, Task};
    use chrono::Utc;

    fn make_task(id: u64, status: Status, depends_on: Vec<u64>, parent: Option<u64>) -> Task {
        let now = Utc::now();
        Task {
            id,
            title: format!("Task {}", id),
            description: None,
            status,
            kind: Kind::Task,
            parent,
            depends_on: depends_on.into_iter().map(Dependency::simple).collect(),
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        }
    }

    fn tids(ids: &[u64]) -> Vec<TaskId> {
        ids.iter().copied().map(TaskId::from).collect()
    }

    fn column_type(idx: &Index, table: &str, column: &str) -> String {
        let mut stmt = idx
            .conn()
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .unwrap();

        for row in rows {
            let (name, ty) = row.unwrap();
            if name == column {
                return ty;
            }
        }

        panic!("column {column} missing from {table}");
    }

    #[test]
    fn schema_uses_text_for_task_related_ids() {
        let idx = Index::open_memory().unwrap();

        assert_eq!(column_type(&idx, "tasks", "id"), "TEXT");
        assert_eq!(column_type(&idx, "tasks", "parent_id"), "TEXT");
        assert_eq!(column_type(&idx, "dependencies", "task_id"), "TEXT");
        assert_eq!(column_type(&idx, "dependencies", "depends_on_id"), "TEXT");
        assert_eq!(column_type(&idx, "tags", "task_id"), "TEXT");
        assert_eq!(column_type(&idx, "skills", "task_id"), "TEXT");
        assert_eq!(column_type(&idx, "learning_tasks", "task_id"), "TEXT");
    }

    #[test]
    fn rebuild_and_query_available() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![1], None),
            make_task(3, Status::Pending, vec![], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.available(None).unwrap(), tids(&[1, 3]));
        assert_eq!(idx.blocked().unwrap(), tids(&[2]));
    }

    #[test]
    fn finishing_dep_unblocks_task() {
        let idx = Index::open_memory().unwrap();
        let mut tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![1], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.available(None).unwrap(), tids(&[1]));
        tasks[0].status = Status::Done;
        idx.upsert(&tasks[0]).unwrap();
        assert_eq!(idx.available(None).unwrap(), tids(&[2]));
    }

    #[test]
    fn available_excludes_idea_tasks_by_default() {
        let idx = Index::open_memory().unwrap();

        let mut idea = make_task(1, Status::Pending, vec![], None);
        idea.kind = Kind::Idea;
        let task = make_task(2, Status::Pending, vec![], None);

        idx.rebuild(&[idea, task]).unwrap();
        assert_eq!(idx.available(None).unwrap(), tids(&[2]));
    }

    #[test]
    fn available_with_assignee_still_excludes_idea_tasks() {
        let idx = Index::open_memory().unwrap();

        let mut idea = make_task(1, Status::Pending, vec![], None);
        idea.kind = Kind::Idea;
        idea.assignee = Some("agent-idea".into());

        let mut task = make_task(2, Status::Pending, vec![], None);
        task.assignee = Some("agent-idea".into());

        idx.rebuild(&[idea, task]).unwrap();
        assert_eq!(idx.available(Some("agent-idea")).unwrap(), tids(&[2]));
    }

    #[test]
    fn cycle_detection() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![1], None),
            make_task(3, Status::Pending, vec![2], None),
        ];
        idx.rebuild(&tasks).unwrap();
        // 3 -> 2 -> 1. If 1 depends on 3, that's a cycle.
        assert!(idx.would_cycle(1, 3).unwrap());
        // 3 depending on 1 is not a cycle (1 doesn't transitively depend on 3)
        assert!(!idx.would_cycle(3, 1).unwrap());
    }

    #[test]
    fn cycle_detection_with_text_task_ids() {
        let idx = Index::open_memory().unwrap();
        let id_a = 0xaaaa_aaaa_aaaa_aaa1_u64;
        let id_b = 0xbbbb_bbbb_bbbb_bbb2_u64;
        let id_c = 0xcccc_cccc_cccc_ccc3_u64;
        let tasks = vec![
            make_task(id_a, Status::Pending, vec![], None),
            make_task(id_b, Status::Pending, vec![id_a], None),
            make_task(id_c, Status::Pending, vec![id_b], None),
        ];
        idx.rebuild(&tasks).unwrap();

        let a: TaskId = "aaaaaaaaaaaaaaa1".parse().unwrap();
        let c: TaskId = "ccccccccccccccc3".parse().unwrap();

        // c -> b -> a. If a depends on c, that's a cycle.
        assert!(idx.would_cycle(a.clone(), c.clone()).unwrap());
        // c depending on a is not a cycle (a doesn't transitively depend on c)
        assert!(!idx.would_cycle(c, a).unwrap());
    }

    #[test]
    fn children_and_roots() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![], Some(1)),
            make_task(3, Status::Pending, vec![], Some(1)),
            make_task(4, Status::Pending, vec![], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.roots().unwrap(), tids(&[1, 4]));
        assert_eq!(idx.children_of(1).unwrap(), tids(&[2, 3]));
        assert_eq!(idx.children_of(4).unwrap(), Vec::<TaskId>::new());
    }

    #[test]
    fn ids_by_kind_support_meta_and_idea_without_regressing_availability_queries() {
        let idx = Index::open_memory().unwrap();

        let mut t1 = make_task(1, Status::Pending, vec![], None);
        let mut t2 = make_task(2, Status::Pending, vec![], None);
        let mut t3 = make_task(3, Status::Pending, vec![], None);
        t2.kind = Kind::Meta;
        t3.kind = Kind::Idea;

        idx.rebuild(&[t1.clone(), t2, t3]).unwrap();
        assert_eq!(idx.ids_by_kind("meta").unwrap(), tids(&[2]));
        assert_eq!(idx.ids_by_kind("idea").unwrap(), tids(&[3]));
        assert_eq!(idx.ids_by_kind("task").unwrap(), tids(&[1]));
        assert!(idx.ids_by_kind("Meta").unwrap().is_empty());

        // Verify upsert also persists kind changes.
        t1.kind = Kind::Idea;
        idx.upsert(&t1).unwrap();
        assert_eq!(idx.ids_by_kind("idea").unwrap(), tids(&[1, 3]));

        // Availability semantics should still include non-idea kinds and exclude ideas.
        assert_eq!(idx.available(None).unwrap(), tids(&[2]));
    }

    #[test]
    fn rebuild_with_forward_pointing_deps() {
        // Task 1 depends on task 3, which appears later in ID order.
        // With foreign keys ON, a single-pass rebuild would fail because
        // task 3's row doesn't exist yet when inserting the dependency.
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![3], None),
            make_task(2, Status::Pending, vec![], None),
            make_task(3, Status::Pending, vec![], None),
        ];
        idx.rebuild(&tasks).unwrap();
        // Task 1 is blocked by task 3; tasks 2 and 3 are available
        assert_eq!(idx.available(None).unwrap(), tids(&[2, 3]));
        assert_eq!(idx.blocked().unwrap(), tids(&[1]));
    }

    #[test]
    fn rebuild_with_forward_pointing_parent() {
        // Task 1 is a child of task 3 — parent ID points forward in ID order.
        // Without deferred parent_id in pass 1, FK constraint would fail.
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], Some(3)),
            make_task(2, Status::Pending, vec![], None),
            make_task(3, Status::Pending, vec![], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.children_of(3).unwrap(), tids(&[1]));
        assert_eq!(idx.roots().unwrap(), tids(&[2, 3]));
    }

    #[test]
    fn stale_index_detected_after_file_change() {
        use crate::store::files::FileStore;
        use crate::store::repo::Repo;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task_a = store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();
        let task_b = store
            .create(
                "B".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        // First open builds index
        let repo = Repo::open(dir.path()).unwrap();
        let avail = repo.index.available(None).unwrap();
        let expected = vec![TaskId::from(task_a.id), TaskId::from(task_b.id)];
        assert_eq!(avail, expected);
        drop(repo);

        // Simulate external change: delete task B's file
        let task_b_path = dir
            .path()
            .join(".tak/tasks")
            .join(format!("{}.json", TaskId::from(task_b.id)));
        std::fs::remove_file(task_b_path).unwrap();

        // Re-open should detect staleness and rebuild
        let repo = Repo::open(dir.path()).unwrap();
        let avail = repo.index.available(None).unwrap();
        assert_eq!(avail, vec![TaskId::from(task_a.id)]); // task B is gone
    }

    #[test]
    fn stale_index_detected_after_in_place_edit() {
        use crate::store::files::FileStore;
        use crate::store::repo::Repo;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let task = store
            .create(
                "A".into(),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let avail = repo.index.available(None).unwrap();
        assert_eq!(avail, vec![TaskId::from(task.id)]);
        drop(repo);

        // Simulate external edit: change status directly in JSON
        let task_path = dir
            .path()
            .join(".tak/tasks")
            .join(format!("{}.json", TaskId::from(task.id)));
        let data = std::fs::read_to_string(&task_path).unwrap();
        let modified = data.replace("\"pending\"", "\"in_progress\"");
        // Sleep to ensure mtime changes on 1-second resolution filesystems
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&task_path, modified).unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let avail = repo.index.available(None).unwrap();
        assert!(
            avail.is_empty(),
            "task 1 is now in_progress, should not be available"
        );
    }

    #[test]
    fn rebuild_tolerates_duplicate_deps_and_tags() {
        let idx = Index::open_memory().unwrap();
        let now = Utc::now();
        let tasks = vec![
            Task {
                id: 1,
                title: "A".into(),
                description: None,
                status: Status::Pending,
                kind: Kind::Task,
                parent: None,
                depends_on: vec![],
                assignee: None,
                tags: vec!["x".into(), "x".into()],
                contract: Contract::default(),
                planning: Planning::default(),
                git: GitInfo::default(),
                execution: Execution::default(),
                learnings: vec![],
                created_at: now,
                updated_at: now,
                extensions: serde_json::Map::new(),
            },
            Task {
                id: 2,
                title: "B".into(),
                description: None,
                status: Status::Pending,
                kind: Kind::Task,
                parent: None,
                depends_on: vec![Dependency::simple(1), Dependency::simple(1)],
                assignee: None,
                tags: vec![],
                contract: Contract::default(),
                planning: Planning::default(),
                git: GitInfo::default(),
                execution: Execution::default(),
                learnings: vec![],
                created_at: now,
                updated_at: now,
                extensions: serde_json::Map::new(),
            },
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.available(None).unwrap(), tids(&[1]));
    }

    #[test]
    fn upsert_tolerates_duplicate_deps_and_tags() {
        let idx = Index::open_memory().unwrap();
        let now = Utc::now();
        let t1 = make_task(1, Status::Pending, vec![], None);
        idx.rebuild(&[t1]).unwrap();

        let t_duped = Task {
            id: 1,
            title: "A".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec!["x".into(), "x".into()],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        idx.upsert(&t_duped).unwrap();
    }

    #[test]
    fn dependents_of_returns_incoming_deps() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![1], None),
            make_task(3, Status::Pending, vec![1], None),
            make_task(4, Status::Pending, vec![2], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.dependents_of(1).unwrap(), tids(&[2, 3]));
        assert_eq!(idx.dependents_of(2).unwrap(), tids(&[4]));
        assert!(idx.dependents_of(4).unwrap().is_empty());
    }

    #[test]
    fn would_parent_cycle_detection() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![], Some(1)),
            make_task(3, Status::Pending, vec![], Some(2)),
        ];
        idx.rebuild(&tasks).unwrap();
        // Reparenting task 1 under task 3 would create 1 -> 3 -> 2 -> 1
        assert!(idx.would_parent_cycle(1, 3).unwrap());
        // Self-parenting is a cycle
        assert!(idx.would_parent_cycle(1, 1).unwrap());
        // Reparenting task 3 under task 1 is fine (it's already transitively there)
        assert!(!idx.would_parent_cycle(3, 1).unwrap());
    }

    #[test]
    fn would_parent_cycle_detection_with_text_task_ids() {
        let idx = Index::open_memory().unwrap();
        let id_a = 0xdddd_dddd_dddd_ddd4_u64;
        let id_b = 0xeeee_eeee_eeee_eee5_u64;
        let id_c = 0xffff_ffff_ffff_fff6_u64;
        let tasks = vec![
            make_task(id_a, Status::Pending, vec![], None),
            make_task(id_b, Status::Pending, vec![], Some(id_a)),
            make_task(id_c, Status::Pending, vec![], Some(id_b)),
        ];
        idx.rebuild(&tasks).unwrap();

        let a: TaskId = "ddddddddddddddd4".parse().unwrap();
        let c: TaskId = "fffffffffffffff6".parse().unwrap();

        // Reparenting a under c would create a -> c -> b -> a
        assert!(idx.would_parent_cycle(a.clone(), c.clone()).unwrap());
        // Self-parenting is a cycle
        assert!(idx.would_parent_cycle(a.clone(), a.clone()).unwrap());
        // Reparenting c under a is fine
        assert!(!idx.would_parent_cycle(c, a).unwrap());
    }

    #[test]
    fn available_ordered_by_priority_then_created_at_then_id() {
        use crate::model::Priority;

        let idx = Index::open_memory().unwrap();
        let base = Utc::now();

        let mut t1 = make_task(1, Status::Pending, vec![], None);
        t1.planning.priority = Some(Priority::Low);
        t1.created_at = base + chrono::Duration::seconds(20);
        t1.updated_at = t1.created_at;

        let mut t2 = make_task(2, Status::Pending, vec![], None);
        t2.planning.priority = Some(Priority::Critical);
        t2.created_at = base + chrono::Duration::seconds(30);
        t2.updated_at = t2.created_at;

        let mut t3 = make_task(3, Status::Pending, vec![], None);
        t3.created_at = base + chrono::Duration::seconds(10);
        t3.updated_at = t3.created_at;

        let mut t4 = make_task(4, Status::Pending, vec![], None);
        t4.planning.priority = Some(Priority::High);
        t4.created_at = base + chrono::Duration::seconds(40);
        t4.updated_at = t4.created_at;

        let mut t5 = make_task(5, Status::Pending, vec![], None);
        t5.created_at = base + chrono::Duration::seconds(10);
        t5.updated_at = t5.created_at;

        let mut t6 = make_task(6, Status::Pending, vec![], None);
        t6.planning.priority = Some(Priority::Critical);
        t6.created_at = base + chrono::Duration::seconds(5);
        t6.updated_at = t6.created_at;

        idx.rebuild(&[t1, t2, t3, t4, t5, t6]).unwrap();
        let avail = idx.available(None).unwrap();
        // Expected order: critical by created_at (6,2), high (4), low (1),
        // then unprioritized by created_at then id lexical fallback (3,5).
        assert_eq!(avail, tids(&[6, 2, 4, 1, 3, 5]));
    }
}
