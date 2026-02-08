use std::path::Path;

use rusqlite::{Connection, params};

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
                priority INTEGER,
                estimate TEXT,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dependencies (
                task_id INTEGER NOT NULL REFERENCES tasks(id),
                depends_on_id INTEGER NOT NULL REFERENCES tasks(id),
                dep_type TEXT,
                reason TEXT,
                PRIMARY KEY (task_id, depends_on_id)
            );
            CREATE TABLE IF NOT EXISTS tags (
                task_id INTEGER NOT NULL REFERENCES tasks(id),
                tag TEXT NOT NULL,
                PRIMARY KEY (task_id, tag)
            );
            CREATE TABLE IF NOT EXISTS skills (
                task_id INTEGER NOT NULL REFERENCES tasks(id),
                skill TEXT NOT NULL,
                PRIMARY KEY (task_id, skill)
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_kind ON tasks(kind);
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
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
            tx.execute(
                "INSERT INTO tasks (id, title, description, status, kind, parent_id, assignee, priority, estimate, attempt_count, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    task.id, task.title, task.description,
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
            if let Some(parent) = task.parent {
                tx.execute(
                    "UPDATE tasks SET parent_id = ?1 WHERE id = ?2",
                    params![parent, task.id],
                )?;
            }
            for dep in &task.depends_on {
                tx.execute(
                    "INSERT OR IGNORE INTO dependencies (task_id, depends_on_id, dep_type, reason) VALUES (?1, ?2, ?3, ?4)",
                    params![task.id, dep.id, dep.dep_type.as_ref().map(|t| t.to_string()), dep.reason],
                )?;
            }
            for tag in &task.tags {
                tx.execute(
                    "INSERT OR IGNORE INTO tags (task_id, tag) VALUES (?1, ?2)",
                    params![task.id, tag],
                )?;
            }
            for skill in &task.planning.required_skills {
                tx.execute(
                    "INSERT OR IGNORE INTO skills (task_id, skill) VALUES (?1, ?2)",
                    params![task.id, skill],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, task: &Task) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO tasks (id, title, description, status, kind, parent_id, assignee, priority, estimate, attempt_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.id, task.title, task.description,
                task.status.to_string(), task.kind.to_string(),
                task.parent, task.assignee,
                task.planning.priority.map(|p| p.rank() as i64),
                task.planning.estimate.map(|e| e.to_string()),
                task.execution.attempt_count,
                task.created_at.to_rfc3339(), task.updated_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "DELETE FROM dependencies WHERE task_id = ?1",
            params![task.id],
        )?;
        for dep in &task.depends_on {
            tx.execute(
                "INSERT OR IGNORE INTO dependencies (task_id, depends_on_id, dep_type, reason) VALUES (?1, ?2, ?3, ?4)",
                params![task.id, dep.id, dep.dep_type.as_ref().map(|t| t.to_string()), dep.reason],
            )?;
        }
        tx.execute("DELETE FROM tags WHERE task_id = ?1", params![task.id])?;
        for tag in &task.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (task_id, tag) VALUES (?1, ?2)",
                params![task.id, tag],
            )?;
        }
        tx.execute("DELETE FROM skills WHERE task_id = ?1", params![task.id])?;
        for skill in &task.planning.required_skills {
            tx.execute(
                "INSERT OR IGNORE INTO skills (task_id, skill) VALUES (?1, ?2)",
                params![task.id, skill],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: u64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM skills WHERE task_id = ?1", params![id])?;
        tx.execute("DELETE FROM tags WHERE task_id = ?1", params![id])?;
        tx.execute("DELETE FROM dependencies WHERE task_id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM dependencies WHERE depends_on_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Return IDs of available (claimable) tasks.
    /// If `assignee` is Some, also include tasks already assigned to that person.
    /// If `assignee` is None, only unassigned tasks.
    pub fn available(&self, assignee: Option<&str>) -> Result<Vec<u64>> {
        let (sql, has_param) = match assignee {
            Some(_) => (
                "SELECT t.id FROM tasks t
                 WHERE t.status = 'pending'
                 AND (t.assignee IS NULL OR t.assignee = ?1)
                 AND NOT EXISTS (
                     SELECT 1 FROM dependencies d
                     JOIN tasks dep ON d.depends_on_id = dep.id
                     WHERE d.task_id = t.id
                     AND dep.status NOT IN ('done', 'cancelled')
                 )
                 ORDER BY COALESCE(t.priority, 4), t.id",
                true,
            ),
            None => (
                "SELECT t.id FROM tasks t
                 WHERE t.status = 'pending'
                 AND t.assignee IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM dependencies d
                     JOIN tasks dep ON d.depends_on_id = dep.id
                     WHERE d.task_id = t.id
                     AND dep.status NOT IN ('done', 'cancelled')
                 )
                 ORDER BY COALESCE(t.priority, 4), t.id",
                false,
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let ids = if has_param {
            stmt.query_map(params![assignee.unwrap()], |row| row.get(0))?
                .collect::<std::result::Result<Vec<u64>, _>>()?
        } else {
            stmt.query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<u64>, _>>()?
        };
        Ok(ids)
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
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    /// Check whether a specific task is blocked by unfinished dependencies.
    pub fn is_blocked(&self, id: u64) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT EXISTS(
                SELECT 1 FROM dependencies d
                JOIN tasks dep ON d.depends_on_id = dep.id
                WHERE d.task_id = ?1
                AND dep.status NOT IN ('done', 'cancelled')
            )",
        )?;
        let blocked: bool = stmt.query_row(params![id], |row| row.get(0))?;
        Ok(blocked)
    }

    /// Return IDs of tasks that depend on the given task.
    pub fn dependents_of(&self, id: u64) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id FROM dependencies WHERE depends_on_id = ?1 ORDER BY task_id",
        )?;
        let ids = stmt
            .query_map(params![id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn children_of(&self, parent_id: u64) -> Result<Vec<u64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE parent_id = ?1 ORDER BY id")?;
        let ids = stmt
            .query_map(params![parent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<u64>, _>>()?;
        Ok(ids)
    }

    pub fn roots(&self) -> Result<Vec<u64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE parent_id IS NULL ORDER BY id")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
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
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
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
        assert_eq!(idx.available(None).unwrap(), vec![1, 3]);
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
        assert_eq!(idx.available(None).unwrap(), vec![1]);
        tasks[0].status = Status::Done;
        idx.upsert(&tasks[0]).unwrap();
        assert_eq!(idx.available(None).unwrap(), vec![2]);
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
        assert_eq!(idx.available(None).unwrap(), vec![2, 3]);
        assert_eq!(idx.blocked().unwrap(), vec![1]);
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
        assert_eq!(idx.children_of(3).unwrap(), vec![1]);
        assert_eq!(idx.roots().unwrap(), vec![2, 3]);
    }

    #[test]
    fn stale_index_detected_after_file_change() {
        use crate::store::files::FileStore;
        use crate::store::repo::Repo;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        store
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
        store
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
        assert_eq!(avail, vec![1, 2]);
        drop(repo);

        // Simulate external change: delete task 2's file
        std::fs::remove_file(dir.path().join(".tak/tasks/2.json")).unwrap();

        // Re-open should detect staleness and rebuild
        let repo = Repo::open(dir.path()).unwrap();
        let avail = repo.index.available(None).unwrap();
        assert_eq!(avail, vec![1]); // task 2 is gone
    }

    #[test]
    fn stale_index_detected_after_in_place_edit() {
        use crate::store::files::FileStore;
        use crate::store::repo::Repo;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        store
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
        assert_eq!(avail, vec![1]);
        drop(repo);

        // Simulate external edit: change status directly in JSON
        let task_path = dir.path().join(".tak/tasks/1.json");
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
                created_at: now,
                updated_at: now,
                extensions: serde_json::Map::new(),
            },
        ];
        idx.rebuild(&tasks).unwrap();
        assert_eq!(idx.available(None).unwrap(), vec![1]);
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
        assert_eq!(idx.dependents_of(1).unwrap(), vec![2, 3]);
        assert_eq!(idx.dependents_of(2).unwrap(), vec![4]);
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
    fn available_ordered_by_priority_then_id() {
        use crate::model::Priority;

        let idx = Index::open_memory().unwrap();

        let mut t1 = make_task(1, Status::Pending, vec![], None);
        t1.planning.priority = Some(Priority::Low);

        let mut t2 = make_task(2, Status::Pending, vec![], None);
        t2.planning.priority = Some(Priority::Critical);

        let t3 = make_task(3, Status::Pending, vec![], None);
        // No priority — should sort last

        let mut t4 = make_task(4, Status::Pending, vec![], None);
        t4.planning.priority = Some(Priority::High);

        idx.rebuild(&[t1, t2, t3, t4]).unwrap();
        let avail = idx.available(None).unwrap();
        // Expected order: critical(2), high(4), low(1), none(3)
        assert_eq!(avail, vec![2, 4, 1, 3]);
    }
}
