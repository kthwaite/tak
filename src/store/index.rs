use std::path::Path;

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::model::Task;

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
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                kind TEXT NOT NULL DEFAULT 'task',
                parent_id INTEGER REFERENCES tasks(id),
                assignee TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dependencies (
                task_id INTEGER NOT NULL REFERENCES tasks(id),
                depends_on_id INTEGER NOT NULL REFERENCES tasks(id),
                PRIMARY KEY (task_id, depends_on_id)
            );
            CREATE TABLE IF NOT EXISTS tags (
                task_id INTEGER NOT NULL REFERENCES tasks(id),
                tag TEXT NOT NULL,
                PRIMARY KEY (task_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_kind ON tasks(kind);
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);",
        )?;
        Ok(())
    }

    pub fn rebuild(&self, tasks: &[Task]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch("DELETE FROM tags; DELETE FROM dependencies; DELETE FROM tasks;")?;

        // Pass 1: insert all task rows (avoids FK failures from forward-pointing deps)
        for task in tasks {
            tx.execute(
                "INSERT INTO tasks (id, title, description, status, kind, parent_id, assignee, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    task.id, task.title, task.description,
                    task.status.to_string(), task.kind.to_string(),
                    task.parent, task.assignee,
                    task.created_at.to_rfc3339(), task.updated_at.to_rfc3339(),
                ],
            )?;
        }

        // Pass 2: insert all dependencies and tags
        for task in tasks {
            for dep in &task.depends_on {
                tx.execute(
                    "INSERT INTO dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
                    params![task.id, dep],
                )?;
            }
            for tag in &task.tags {
                tx.execute(
                    "INSERT INTO tags (task_id, tag) VALUES (?1, ?2)",
                    params![task.id, tag],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, task: &Task) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO tasks (id, title, description, status, kind, parent_id, assignee, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                task.id, task.title, task.description,
                task.status.to_string(), task.kind.to_string(),
                task.parent, task.assignee,
                task.created_at.to_rfc3339(), task.updated_at.to_rfc3339(),
            ],
        )?;
        tx.execute("DELETE FROM dependencies WHERE task_id = ?1", params![task.id])?;
        for dep in &task.depends_on {
            tx.execute(
                "INSERT INTO dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
                params![task.id, dep],
            )?;
        }
        tx.execute("DELETE FROM tags WHERE task_id = ?1", params![task.id])?;
        for tag in &task.tags {
            tx.execute(
                "INSERT INTO tags (task_id, tag) VALUES (?1, ?2)",
                params![task.id, tag],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: u64) -> Result<()> {
        self.conn.execute("DELETE FROM tags WHERE task_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM dependencies WHERE task_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM dependencies WHERE depends_on_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn available(&self) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tasks t
             WHERE t.status = 'pending'
             AND t.assignee IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM dependencies d
                 JOIN tasks dep ON d.depends_on_id = dep.id
                 WHERE d.task_id = t.id
                 AND dep.status NOT IN ('done', 'cancelled')
             )
             ORDER BY t.id",
        )?;
        let ids = stmt.query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn available_for(&self, assignee: Option<&str>) -> Result<Vec<u64>> {
        match assignee {
            Some(name) => {
                let mut stmt = self.conn.prepare(
                    "SELECT t.id FROM tasks t
                     WHERE t.status = 'pending'
                     AND (t.assignee IS NULL OR t.assignee = ?1)
                     AND NOT EXISTS (
                         SELECT 1 FROM dependencies d
                         JOIN tasks dep ON d.depends_on_id = dep.id
                         WHERE d.task_id = t.id
                         AND dep.status NOT IN ('done', 'cancelled')
                     )
                     ORDER BY t.id",
                )?;
                let ids = stmt.query_map(params![name], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<u64>, _>>()?;
                Ok(ids)
            }
            None => self.available(),
        }
    }

    pub fn blocked(&self) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT t.id FROM tasks t
             JOIN dependencies d ON d.task_id = t.id
             JOIN tasks dep ON d.depends_on_id = dep.id
             WHERE t.status = 'pending'
             AND dep.status NOT IN ('done', 'cancelled')
             ORDER BY t.id",
        )?;
        let ids = stmt.query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn children_of(&self, parent_id: u64) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks WHERE parent_id = ?1 ORDER BY id",
        )?;
        let ids = stmt.query_map(params![parent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn roots(&self) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks WHERE parent_id IS NULL ORDER BY id",
        )?;
        let ids = stmt.query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn would_cycle(&self, task_id: u64, depends_on_id: u64) -> Result<bool> {
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
        let exists: bool = stmt.query_row(params![depends_on_id, task_id], |row| row.get(0))?;
        Ok(exists)
    }

    /// Check if making `child_id` a child of `parent_id` would create a parent-child cycle.
    pub fn would_parent_cycle(&self, child_id: u64, parent_id: u64) -> Result<bool> {
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
        let exists: bool = stmt.query_row(params![parent_id, child_id], |row| row.get(0))?;
        Ok(exists)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Status, Task};
    use chrono::Utc;

    fn make_task(id: u64, status: Status, depends_on: Vec<u64>, parent: Option<u64>) -> Task {
        let now = Utc::now();
        Task {
            id, title: format!("Task {}", id), description: None,
            status, kind: Kind::Task, parent, depends_on,
            assignee: None, tags: vec![], created_at: now, updated_at: now,
        }
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
        assert_eq!(idx.available().unwrap(), vec![1, 3]);
        assert_eq!(idx.blocked().unwrap(), vec![2]);
    }

    #[test]
    fn finishing_dep_unblocks_task() {
        let idx = Index::open_memory().unwrap();
        let mut tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![1], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.available().unwrap(), vec![1]);
        tasks[0].status = Status::Done;
        idx.upsert(&tasks[0]).unwrap();
        assert_eq!(idx.available().unwrap(), vec![2]);
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
    fn children_and_roots() {
        let idx = Index::open_memory().unwrap();
        let tasks = vec![
            make_task(1, Status::Pending, vec![], None),
            make_task(2, Status::Pending, vec![], Some(1)),
            make_task(3, Status::Pending, vec![], Some(1)),
            make_task(4, Status::Pending, vec![], None),
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.roots().unwrap(), vec![1, 4]);
        assert_eq!(idx.children_of(1).unwrap(), vec![2, 3]);
        assert_eq!(idx.children_of(4).unwrap(), Vec::<u64>::new());
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
        assert_eq!(idx.available().unwrap(), vec![2, 3]);
        assert_eq!(idx.blocked().unwrap(), vec![1]);
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
}
