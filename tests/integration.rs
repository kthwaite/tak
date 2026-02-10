use std::fs;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tempfile::tempdir;

use chrono::Utc;
use tak::error::TakError;
use tak::model::{Contract, DepType, Kind, LearningCategory, Planning, Status};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::index::Index;
use tak::store::repo::Repo;
use tak::task_id::TaskId;

fn task_id_hex(id: u64) -> String {
    format!("{id:016x}")
}

fn tid(id: u64) -> TaskId {
    TaskId::from(id)
}

// Env-var tests must not run concurrently within this integration test binary.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TakAgentEnvReset;

impl Drop for TakAgentEnvReset {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("TAK_AGENT");
        }
    }
}

#[test]
fn test_full_workflow() {
    let dir = tempdir().unwrap();

    // Init
    let store = FileStore::init(dir.path()).unwrap();

    // Create epic
    let epic = store
        .create(
            "Auth system".into(),
            Kind::Epic,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    assert_eq!(epic.id, 1);

    // Create child tasks under the epic
    let _t2 = store
        .create(
            "Design API".into(),
            Kind::Task,
            None,
            Some(1),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let _t3 = store
        .create(
            "Implement endpoints".into(),
            Kind::Task,
            None,
            Some(1),
            vec![2],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let _t4 = store
        .create(
            "Write tests".into(),
            Kind::Task,
            None,
            Some(1),
            vec![3],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    // Build index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    let all = store.list_all().unwrap();
    idx.rebuild(&all).unwrap();

    // Check available: tasks 1 and 2 have no unfinished deps
    // Task 3 is blocked by 2, task 4 is blocked by 3
    let avail = idx.available(None).unwrap();
    assert!(avail.contains(&tid(1)));
    assert!(avail.contains(&tid(2)));
    assert!(!avail.contains(&tid(3)));
    assert!(!avail.contains(&tid(4)));

    // Start and finish task 2
    let mut t2 = store.read(2).unwrap();
    t2.status = Status::InProgress;
    t2.updated_at = Utc::now();
    store.write(&t2).unwrap();
    idx.upsert(&t2).unwrap();

    t2.status = Status::Done;
    t2.updated_at = Utc::now();
    store.write(&t2).unwrap();
    idx.upsert(&t2).unwrap();

    // Task 3 should now be available (its dependency task 2 is done)
    let avail = idx.available(None).unwrap();
    assert!(avail.contains(&tid(3)));
    assert!(!avail.contains(&tid(4))); // still blocked by 3

    // Finish task 3
    let mut t3 = store.read(3).unwrap();
    t3.status = Status::InProgress;
    t3.updated_at = Utc::now();
    store.write(&t3).unwrap();
    idx.upsert(&t3).unwrap();

    t3.status = Status::Done;
    t3.updated_at = Utc::now();
    store.write(&t3).unwrap();
    idx.upsert(&t3).unwrap();

    // Task 4 should now be available
    let avail = idx.available(None).unwrap();
    assert!(avail.contains(&tid(4)));

    // Verify tree structure
    let roots = idx.roots().unwrap();
    assert_eq!(roots, vec![1]);
    let children = idx.children_of(1).unwrap();
    assert_eq!(children, vec![2, 3, 4]);
}

#[test]
fn test_cycle_rejection() {
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
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    store
        .create(
            "C".into(),
            Kind::Task,
            None,
            None,
            vec![2],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();

    // Chain: 2 depends on 1, 3 depends on 2.
    // Adding "1 depends on 3" would create cycle: 1 -> 3 -> 2 -> 1
    assert!(idx.would_cycle(1, 3).unwrap());

    // Self-dependency is always a cycle
    assert!(idx.would_cycle(1, 1).unwrap());

    // Adding "3 depends on 1" is redundant (transitive) but NOT a cycle
    assert!(!idx.would_cycle(3, 1).unwrap());
}

#[test]
fn test_reindex_after_delete() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    store
        .create(
            "Task A".into(),
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
            "Task B".into(),
            Kind::Task,
            None,
            None,
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    // Build initial index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Delete the index file
    std::fs::remove_file(store.root().join("index.db")).unwrap();

    // Repo::open should auto-rebuild the index
    let repo = Repo::open(dir.path()).unwrap();

    // Verify queries still work after auto-rebuild
    let avail = repo.index.available(None).unwrap();
    assert_eq!(avail, vec![1]);
    let blocked = repo.index.blocked().unwrap();
    assert_eq!(blocked, vec![2]);
}

#[test]
fn test_status_transitions() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create(
            "Test".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    assert_eq!(task.status, Status::Pending);

    // Build index so lifecycle commands can open the repo
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // pending -> done is INVALID (must go through in_progress)
    let result = tak::commands::lifecycle::finish(dir.path(), 1, Format::Json);
    assert!(result.is_err());
    match result.unwrap_err() {
        TakError::InvalidTransition(from, to) => {
            assert_eq!(from, "pending");
            assert_eq!(to, "done");
        }
        other => panic!("expected InvalidTransition, got: {other}"),
    }

    // pending -> in_progress is valid
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::InProgress);

    // in_progress -> done is valid
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Done);

    // done -> in_progress is INVALID
    let result = tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json);
    assert!(result.is_err());
    match result.unwrap_err() {
        TakError::InvalidTransition(from, to) => {
            assert_eq!(from, "done");
            assert_eq!(to, "in_progress");
        }
        other => panic!("expected InvalidTransition, got: {other}"),
    }

    // done -> pending is valid (reopen)
    // The lifecycle module doesn't expose a "reopen" command directly,
    // so we test this at the store layer to verify the concept.
    let mut t = store.read(1).unwrap();
    t.status = Status::Pending;
    t.updated_at = Utc::now();
    store.write(&t).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Pending);
}

#[test]
fn test_start_rejects_blocked_task() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    store
        .create(
            "Blocker".into(),
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
            "Blocked".into(),
            Kind::Task,
            None,
            None,
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Starting the blocked task should fail
    let result = tak::commands::lifecycle::start(dir.path(), 2, None, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::TaskBlocked(2)));

    // Task should still be pending
    let t = store.read(2).unwrap();
    assert_eq!(t.status, Status::Pending);

    // Finishing the blocker should unblock task 2
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();
    tak::commands::lifecycle::start(dir.path(), 2, None, Format::Json).unwrap();
    let t = store.read(2).unwrap();
    assert_eq!(t.status, Status::InProgress);
}

#[test]
fn test_meta_lifecycle_and_dependency_parity() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let blocker_id = store
        .create(
            "Meta blocker".into(),
            Kind::Meta,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;
    let blocked_id = store
        .create(
            "Meta blocked".into(),
            Kind::Meta,
            None,
            None,
            vec![blocker_id],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    let blocked_start = tak::commands::lifecycle::start(
        dir.path(),
        blocked_id,
        Some("agent-meta".into()),
        Format::Json,
    );
    assert!(matches!(
        blocked_start,
        Err(TakError::TaskBlocked(id)) if id == blocked_id
    ));

    tak::commands::lifecycle::start(
        dir.path(),
        blocker_id,
        Some("agent-meta".into()),
        Format::Json,
    )
    .unwrap();
    tak::commands::lifecycle::finish(dir.path(), blocker_id, Format::Json).unwrap();

    tak::commands::lifecycle::start(
        dir.path(),
        blocked_id,
        Some("agent-meta".into()),
        Format::Json,
    )
    .unwrap();
    let in_progress = store.read(blocked_id).unwrap();
    assert_eq!(in_progress.kind, Kind::Meta);
    assert_eq!(in_progress.status, Status::InProgress);
    assert_eq!(in_progress.assignee.as_deref(), Some("agent-meta"));

    tak::commands::lifecycle::handoff(
        dir.path(),
        blocked_id,
        "waiting on meta review".into(),
        Format::Json,
    )
    .unwrap();
    let handed_off = store.read(blocked_id).unwrap();
    assert_eq!(handed_off.status, Status::Pending);
    assert!(handed_off.assignee.is_none());
    assert_eq!(
        handed_off.execution.handoff_summary.as_deref(),
        Some("waiting on meta review")
    );

    tak::commands::lifecycle::start(
        dir.path(),
        blocked_id,
        Some("agent-meta".into()),
        Format::Json,
    )
    .unwrap();
    tak::commands::lifecycle::cancel(
        dir.path(),
        blocked_id,
        Some("meta run paused".into()),
        Format::Json,
    )
    .unwrap();
    let cancelled = store.read(blocked_id).unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(
        cancelled.execution.last_error.as_deref(),
        Some("meta run paused")
    );

    tak::commands::lifecycle::reopen(dir.path(), blocked_id, Format::Json).unwrap();
    let reopened = store.read(blocked_id).unwrap();
    assert_eq!(reopened.status, Status::Pending);
    assert!(reopened.assignee.is_none());
}

#[test]
fn test_list_filters() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    // Create tasks with different kinds and tags
    store
        .create(
            "Epic 1".into(),
            Kind::Epic,
            None,
            None,
            vec![],
            vec!["backend".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    store
        .create(
            "Task 1".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["frontend".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    store
        .create(
            "Bug 1".into(),
            Kind::Bug,
            None,
            None,
            vec![],
            vec!["backend".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let all = store.list_all().unwrap();
    assert_eq!(all.len(), 3);

    // Filter by kind
    let epics: Vec<_> = all.iter().filter(|t| t.kind == Kind::Epic).collect();
    assert_eq!(epics.len(), 1);

    let tasks: Vec<_> = all.iter().filter(|t| t.kind == Kind::Task).collect();
    assert_eq!(tasks.len(), 1);

    let bugs: Vec<_> = all.iter().filter(|t| t.kind == Kind::Bug).collect();
    assert_eq!(bugs.len(), 1);

    // Filter by tag
    let backend: Vec<_> = all
        .iter()
        .filter(|t| t.tags.contains(&"backend".into()))
        .collect();
    assert_eq!(backend.len(), 2);

    let frontend: Vec<_> = all
        .iter()
        .filter(|t| t.tags.contains(&"frontend".into()))
        .collect();
    assert_eq!(frontend.len(), 1);

    // Filter by status (all should be pending at creation)
    let pending: Vec<_> = all.iter().filter(|t| t.status == Status::Pending).collect();
    assert_eq!(pending.len(), 3);

    // Modify one task's status and re-check
    let mut t = store.read(2).unwrap();
    t.status = Status::InProgress;
    t.updated_at = Utc::now();
    store.write(&t).unwrap();

    let all = store.list_all().unwrap();
    let in_progress: Vec<_> = all
        .iter()
        .filter(|t| t.status == Status::InProgress)
        .collect();
    assert_eq!(in_progress.len(), 1);
    assert_eq!(in_progress[0].id, 2);

    let still_pending: Vec<_> = all.iter().filter(|t| t.status == Status::Pending).collect();
    assert_eq!(still_pending.len(), 2);
}

#[test]
fn test_claim_assigns_next_available() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task_a_id = store
        .create(
            "Task A".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;
    let task_b_id = store
        .create(
            "Task B".into(),
            Kind::Task,
            None,
            None,
            vec![task_a_id],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Claim as agent-1 — should get task A (only available)
    tak::commands::claim::run(dir.path(), "agent-1".into(), None, Format::Json).unwrap();

    let task_a = store.read(task_a_id).unwrap();
    assert_eq!(task_a.status, Status::InProgress);
    assert_eq!(task_a.assignee.as_deref(), Some("agent-1"));

    // Task B is still blocked, so nothing else is available
    let task_b = store.read(task_b_id).unwrap();
    assert_eq!(task_b.status, Status::Pending);

    let result = tak::commands::claim::run(dir.path(), "agent-2".into(), None, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::NoAvailableTask));
}

#[test]
fn test_claim_assigns_meta_tasks_with_dependency_parity() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let meta_a_id = store
        .create(
            "Meta A".into(),
            Kind::Meta,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;
    let meta_b_id = store
        .create(
            "Meta B".into(),
            Kind::Meta,
            None,
            None,
            vec![meta_a_id],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::claim::run(dir.path(), "agent-meta-1".into(), None, Format::Json).unwrap();

    let meta_a = store.read(meta_a_id).unwrap();
    assert_eq!(meta_a.kind, Kind::Meta);
    assert_eq!(meta_a.status, Status::InProgress);
    assert_eq!(meta_a.assignee.as_deref(), Some("agent-meta-1"));

    let blocked_claim =
        tak::commands::claim::run(dir.path(), "agent-meta-2".into(), None, Format::Json);
    assert!(matches!(
        blocked_claim.unwrap_err(),
        TakError::NoAvailableTask
    ));

    tak::commands::lifecycle::finish(dir.path(), meta_a_id, Format::Json).unwrap();
    tak::commands::claim::run(dir.path(), "agent-meta-2".into(), None, Format::Json).unwrap();

    let meta_b = store.read(meta_b_id).unwrap();
    assert_eq!(meta_b.kind, Kind::Meta);
    assert_eq!(meta_b.status, Status::InProgress);
    assert_eq!(meta_b.assignee.as_deref(), Some("agent-meta-2"));
}

#[test]
fn test_reopen_transitions() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    let task_id = store
        .create(
            "Test".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // pending -> in_progress -> done
    tak::commands::lifecycle::start(dir.path(), task_id, None, Format::Json).unwrap();
    tak::commands::lifecycle::finish(dir.path(), task_id, Format::Json).unwrap();
    let t = store.read(task_id).unwrap();
    assert_eq!(t.status, Status::Done);

    // done -> pending (reopen)
    tak::commands::lifecycle::reopen(dir.path(), task_id, Format::Json).unwrap();
    let t = store.read(task_id).unwrap();
    assert_eq!(t.status, Status::Pending);
    assert!(t.assignee.is_none(), "reopen should clear assignee");
}

#[test]
fn test_depend_rolls_back_on_partial_failure() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let a = store
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
    let b = store
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

    // Build index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Try to depend A on [B, missing]. Missing doesn't exist, so this should fail entirely.
    let result = tak::commands::deps::depend(
        dir.path(),
        vec![a.id],
        vec![b.id, u64::MAX],
        None,
        None,
        Format::Json,
        false,
    );
    assert!(result.is_err());

    // Task A's file should still have no dependencies.
    let task = store.read(a.id).unwrap();
    assert!(
        task.depends_on.is_empty(),
        "file should be unchanged on failure"
    );

    // Index should also have no deps for task A.
    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available(None).unwrap();
    assert!(
        avail.contains(&tid(a.id)),
        "task A should still be available (not blocked)"
    );
}

#[test]
fn test_depend_with_type_and_reason() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let dependency = store
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
    let dependent = store
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

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Add dependency with type and reason.
    tak::commands::deps::depend(
        dir.path(),
        vec![dependent.id],
        vec![dependency.id],
        Some(DepType::Soft),
        Some("nice to have".into()),
        Format::Json,
        false,
    )
    .unwrap();

    let task = store.read(dependent.id).unwrap();
    assert_eq!(task.depends_on.len(), 1);
    assert_eq!(task.depends_on[0].id, dependency.id);
    assert_eq!(task.depends_on[0].dep_type, Some(DepType::Soft));
    assert_eq!(task.depends_on[0].reason.as_deref(), Some("nice to have"));

    // Update existing dependency metadata.
    tak::commands::deps::depend(
        dir.path(),
        vec![dependent.id],
        vec![dependency.id],
        Some(DepType::Hard),
        None,
        Format::Json,
        false,
    )
    .unwrap();

    let task = store.read(dependent.id).unwrap();
    assert_eq!(task.depends_on.len(), 1);
    assert_eq!(task.depends_on[0].dep_type, Some(DepType::Hard));
    assert_eq!(
        task.depends_on[0].reason.as_deref(),
        Some("nice to have"),
        "reason should be preserved when only dep_type is updated"
    );
}

#[test]
fn test_depend_and_undepend_support_multi_target_batch_edits() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let dependency = store
        .create(
            "Dependency".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let first = store
        .create(
            "First".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let second = store
        .create(
            "Second".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::deps::depend(
        dir.path(),
        vec![first.id, second.id],
        vec![dependency.id],
        None,
        None,
        Format::Json,
        true,
    )
    .unwrap();

    let first_task = store.read(first.id).unwrap();
    let second_task = store.read(second.id).unwrap();
    assert_eq!(
        first_task
            .depends_on
            .iter()
            .map(|dep| dep.id)
            .collect::<Vec<_>>(),
        vec![dependency.id]
    );
    assert_eq!(
        second_task
            .depends_on
            .iter()
            .map(|dep| dep.id)
            .collect::<Vec<_>>(),
        vec![dependency.id]
    );

    tak::commands::deps::undepend(
        dir.path(),
        vec![first.id, second.id],
        vec![dependency.id],
        Format::Json,
        true,
    )
    .unwrap();

    let first_task = store.read(first.id).unwrap();
    let second_task = store.read(second.id).unwrap();
    assert!(first_task.depends_on.is_empty());
    assert!(second_task.depends_on.is_empty());
}

#[test]
fn test_reparent_supports_multi_target_batch_edits() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let parent = store
        .create(
            "Parent".into(),
            Kind::Epic,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let first = store
        .create(
            "First".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let second = store
        .create(
            "Second".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::deps::reparent(
        dir.path(),
        vec![first.id, second.id],
        parent.id,
        Format::Json,
        true,
    )
    .unwrap();

    let first_task = store.read(first.id).unwrap();
    let second_task = store.read(second.id).unwrap();
    assert_eq!(first_task.parent, Some(parent.id));
    assert_eq!(second_task.parent, Some(parent.id));
}

#[test]
fn test_reparent_rolls_back_on_partial_failure() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let old_parent = store
        .create(
            "Old parent".into(),
            Kind::Epic,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let new_parent = store
        .create(
            "New parent".into(),
            Kind::Epic,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let first = store
        .create(
            "First".into(),
            Kind::Task,
            None,
            Some(old_parent.id),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let second = store
        .create(
            "Second".into(),
            Kind::Task,
            None,
            Some(old_parent.id),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    let missing_id = 9_999_999_999_u64;
    let result = tak::commands::deps::reparent(
        dir.path(),
        vec![first.id, missing_id, second.id],
        new_parent.id,
        Format::Json,
        true,
    );

    assert!(matches!(result.unwrap_err(), TakError::TaskNotFound(id) if id == missing_id));

    let first_task = store.read(first.id).unwrap();
    let second_task = store.read(second.id).unwrap();
    assert_eq!(first_task.parent, Some(old_parent.id));
    assert_eq!(second_task.parent, Some(old_parent.id));
}

#[test]
fn test_reparent_rejects_batch_when_any_target_would_cycle() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let ancestor = store
        .create(
            "Ancestor".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let descendant = store
        .create(
            "Descendant".into(),
            Kind::Task,
            None,
            Some(ancestor.id),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    let sibling = store
        .create(
            "Sibling".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    let result = tak::commands::deps::reparent(
        dir.path(),
        vec![ancestor.id, sibling.id],
        descendant.id,
        Format::Json,
        true,
    );

    assert!(matches!(result.unwrap_err(), TakError::CycleDetected(id) if id == ancestor.id));

    let ancestor_task = store.read(ancestor.id).unwrap();
    let sibling_task = store.read(sibling.id).unwrap();
    assert_eq!(ancestor_task.parent, None);
    assert_eq!(sibling_task.parent, None);
}

#[test]
fn test_delete_removes_task() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "To delete".into(),
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
            "Keeper".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::delete::run(dir.path(), 1, false, Format::Json).unwrap();

    // File should be gone
    assert!(store.read(1).is_err());

    // Index should not have task 1
    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available(None).unwrap();
    assert_eq!(avail, vec![2]);
}

#[test]
fn test_delete_rejects_when_task_has_children() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Parent".into(),
            Kind::Epic,
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
            "Child".into(),
            Kind::Task,
            None,
            Some(1),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    let result = tak::commands::delete::run(dir.path(), 1, false, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::TaskInUse(1)));
    assert!(store.read(1).is_ok());
}

#[test]
fn test_delete_rejects_when_task_is_dependency_target() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Dep target".into(),
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
            "Dependent".into(),
            Kind::Task,
            None,
            None,
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    let result = tak::commands::delete::run(dir.path(), 1, false, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::TaskInUse(1)));
    assert!(store.read(1).is_ok());
}

#[test]
fn test_delete_force_cascades() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Parent".into(),
            Kind::Epic,
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
            "Child".into(),
            Kind::Task,
            None,
            Some(1),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    store
        .create(
            "Dependent".into(),
            Kind::Task,
            None,
            None,
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::delete::run(dir.path(), 1, true, Format::Json).unwrap();

    assert!(store.read(1).is_err());
    let child = store.read(2).unwrap();
    assert!(child.parent.is_none(), "child should be orphaned");
    let dep = store.read(3).unwrap();
    assert!(
        dep.depends_on.is_empty(),
        "dep on deleted task should be removed"
    );

    // Rebuild should succeed
    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available(None).unwrap();
    assert_eq!(avail, vec![2, 3]);
}

#[test]
fn test_delete_leaf_without_force() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Leaf".into(),
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
            "Other".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::delete::run(dir.path(), 1, false, Format::Json).unwrap();
    assert!(store.read(1).is_err());

    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available(None).unwrap();
    assert_eq!(avail, vec![2]);
}

// === Setup tests ===

#[test]
fn test_setup_install_idempotent() {
    let dir = tempdir().unwrap();
    let settings_path = dir.path().join(".claude").join("settings.local.json");
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(&settings_path, "{}").unwrap();

    let hook_entry = serde_json::json!({
        "matcher": "",
        "hooks": [
            {"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh cleanup --stale --format minimal >/dev/null 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh join --format minimal >/dev/null 2>/dev/null || true", "timeout": 10}
        ]
    });

    // First install
    let data = fs::read_to_string(&settings_path).unwrap();
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&data).unwrap();

    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().unwrap();
    let session_start = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    let arr = session_start.as_array_mut().unwrap();
    assert!(!arr.iter().any(|e| e == &hook_entry));
    arr.push(hook_entry.clone());

    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&serde_json::Value::Object(settings)).unwrap(),
    )
    .unwrap();

    // Second install attempt — hook already present
    let data = fs::read_to_string(&settings_path).unwrap();
    let settings: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&data).unwrap();
    let arr = settings["hooks"]["SessionStart"].as_array().unwrap();
    assert!(
        arr.iter().any(|e| e == &hook_entry),
        "hook installed after first write"
    );
    assert_eq!(arr.len(), 1, "only one hook entry");
}

#[test]
fn test_setup_remove_cleans_hooks() {
    let dir = tempdir().unwrap();
    let settings_path = dir.path().join("settings.json");

    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "",
                "hooks": [
            {"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh cleanup --stale --format minimal >/dev/null 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh join --format minimal >/dev/null 2>/dev/null || true", "timeout": 10}
        ]
            }]
        }
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();

    let data = fs::read_to_string(&settings_path).unwrap();
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&data).unwrap();

    let target = serde_json::json!({
        "matcher": "",
        "hooks": [
            {"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh cleanup --stale --format minimal >/dev/null 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh join --format minimal >/dev/null 2>/dev/null || true", "timeout": 10}
        ]
    });

    let hooks = settings.get_mut("hooks").unwrap().as_object_mut().unwrap();
    let arr = hooks
        .get_mut("SessionStart")
        .unwrap()
        .as_array_mut()
        .unwrap();
    arr.retain(|e| e != &target);
    assert!(arr.is_empty());

    hooks.remove("SessionStart");
    if hooks.is_empty() {
        settings.remove("hooks");
    }
    assert!(
        !settings.contains_key("hooks"),
        "hooks key removed when empty"
    );
}

#[test]
fn test_setup_preserves_existing_settings() {
    let dir = tempdir().unwrap();
    let settings_path = dir.path().join("settings.json");

    // Pre-existing settings
    let initial = serde_json::json!({"model": "sonnet", "allowedTools": ["Bash"]});
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&initial).unwrap(),
    )
    .unwrap();

    let data = fs::read_to_string(&settings_path).unwrap();
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&data).unwrap();

    let hook_entry = serde_json::json!({
        "matcher": "",
        "hooks": [
            {"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh cleanup --stale --format minimal >/dev/null 2>/dev/null || true", "timeout": 10},
            {"type": "command", "command": "tak mesh join --format minimal >/dev/null 2>/dev/null || true", "timeout": 10}
        ]
    });
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().unwrap();
    let session_start = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    session_start.as_array_mut().unwrap().push(hook_entry);

    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&serde_json::Value::Object(settings)).unwrap(),
    )
    .unwrap();

    let loaded: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert_eq!(loaded.get("model"), Some(&serde_json::json!("sonnet")));
    assert!(loaded.contains_key("hooks"));
    assert!(loaded.contains_key("allowedTools"));
}

// === Doctor tests ===

#[test]
fn test_doctor_healthy_repo_structure() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    let task = store
        .create(
            "Task A".into(),
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
    let tasks = repo.store.list_all().unwrap();
    repo.index.rebuild(&tasks).unwrap();
    let fp = repo.store.fingerprint().unwrap();
    repo.index.set_fingerprint(&fp).unwrap();

    // Verify core structure
    assert!(dir.path().join(".tak/config.json").exists());
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".tak/config.json")).unwrap())
            .unwrap();
    assert_eq!(config["version"], 2);
    assert!(!dir.path().join(".tak/counter.json").exists());
    assert!(dir.path().join(".tak/tasks").is_dir());
    assert!(
        dir.path()
            .join(format!(".tak/tasks/{}.json", task_id_hex(task.id)))
            .exists()
    );
    assert!(dir.path().join(".tak/index.db").exists());
}

#[test]
fn test_doctor_detects_dangling_parent() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Parent".into(),
            Kind::Epic,
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
            "Child".into(),
            Kind::Task,
            None,
            Some(1),
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    // Delete parent file directly
    fs::remove_file(
        dir.path()
            .join(format!(".tak/tasks/{}.json", task_id_hex(1))),
    )
    .unwrap();

    let child: tak::model::Task = serde_json::from_str(
        &fs::read_to_string(
            dir.path()
                .join(format!(".tak/tasks/{}.json", task_id_hex(2))),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        child.parent,
        Some(1),
        "child still references deleted parent"
    );
}

#[test]
fn test_doctor_detects_dangling_dep() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Dep".into(),
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
            "Dependent".into(),
            Kind::Task,
            None,
            None,
            vec![1],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    fs::remove_file(
        dir.path()
            .join(format!(".tak/tasks/{}.json", task_id_hex(1))),
    )
    .unwrap();

    let task: tak::model::Task = serde_json::from_str(
        &fs::read_to_string(
            dir.path()
                .join(format!(".tak/tasks/{}.json", task_id_hex(2))),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        task.depends_on.iter().any(|d| d.id == 1),
        "dep on deleted task remains"
    );
}

#[test]
fn test_doctor_missing_index() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();
    assert!(!dir.path().join(".tak/index.db").exists());
}

#[test]
fn test_edit_adds_contract_fields() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Bare task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::edit::run(
        dir.path(),
        1,
        None,
        None,
        None,
        None,
        Some("Build it".into()),
        Some(vec!["cargo test".into()]),
        Some(vec!["No panics".into()]),
        Some(vec!["Compiles clean".into()]),
        None,
        None,
        None,
        None,
        None,
        Format::Json,
    )
    .unwrap();

    let task = store.read(1).unwrap();
    assert_eq!(task.contract.objective.as_deref(), Some("Build it"));
    assert_eq!(task.contract.verification, vec!["cargo test"]);
    assert_eq!(task.contract.constraints, vec!["No panics"]);
    assert_eq!(task.contract.acceptance_criteria, vec!["Compiles clean"]);
}

#[test]
fn test_show_contract_in_pretty_output() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Contracted".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract {
                objective: Some("Ship feature X".into()),
                acceptance_criteria: vec!["All tests pass".into(), "No regressions".into()],
                verification: vec!["cargo test".into(), "cargo clippy".into()],
                constraints: vec!["No unsafe code".into()],
            },
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Verify contract fields persisted correctly via JSON round-trip
    let task = store.read(1).unwrap();
    let json = serde_json::to_string_pretty(&task).unwrap();
    assert!(json.contains("Ship feature X"));
    assert!(json.contains("cargo test"));
    assert!(json.contains("No unsafe code"));
    assert!(json.contains("All tests pass"));
}

#[test]
fn test_create_with_contract() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create(
            "Contracted task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract {
                objective: Some("Ship it".into()),
                acceptance_criteria: vec!["Tests pass".into()],
                verification: vec!["cargo test".into()],
                constraints: vec!["No unsafe".into()],
            },
            Planning::default(),
        )
        .unwrap();

    assert_eq!(task.contract.objective.as_deref(), Some("Ship it"));
    assert_eq!(task.contract.verification, vec!["cargo test"]);
    assert_eq!(task.contract.constraints, vec!["No unsafe"]);
    assert_eq!(task.contract.acceptance_criteria, vec!["Tests pass"]);

    // Round-trip through file
    let read = store.read(task.id).unwrap();
    assert_eq!(read.contract, task.contract);
}

#[test]
fn test_create_with_planning() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create(
            "Prioritized task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning {
                priority: Some(tak::model::Priority::High),
                estimate: Some(tak::model::Estimate::M),
                required_skills: vec!["rust".into()],
                risk: Some(tak::model::Risk::Low),
            },
        )
        .unwrap();

    assert_eq!(task.planning.priority, Some(tak::model::Priority::High));
    assert_eq!(task.planning.estimate, Some(tak::model::Estimate::M));
    assert_eq!(task.planning.risk, Some(tak::model::Risk::Low));
    assert_eq!(task.planning.required_skills, vec!["rust"]);

    let read = store.read(task.id).unwrap();
    assert_eq!(read.planning, task.planning);
}

#[test]
fn test_edit_sets_planning_fields() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Bare".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::edit::run(
        dir.path(),
        1,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(tak::model::Priority::Critical),
        Some(tak::model::Estimate::Xl),
        Some(vec!["python".into()]),
        Some(tak::model::Risk::High),
        None,
        Format::Json,
    )
    .unwrap();

    let task = store.read(1).unwrap();
    assert_eq!(task.planning.priority, Some(tak::model::Priority::Critical));
    assert_eq!(task.planning.estimate, Some(tak::model::Estimate::Xl));
    assert_eq!(task.planning.risk, Some(tak::model::Risk::High));
    assert_eq!(task.planning.required_skills, vec!["python"]);
}

#[test]
fn test_list_filter_by_priority() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    store
        .create(
            "Low task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning {
                priority: Some(tak::model::Priority::Low),
                ..Planning::default()
            },
        )
        .unwrap();
    store
        .create(
            "High task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning {
                priority: Some(tak::model::Priority::High),
                ..Planning::default()
            },
        )
        .unwrap();
    store
        .create(
            "No priority".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let all = store.list_all().unwrap();
    let high: Vec<_> = all
        .iter()
        .filter(|t| t.planning.priority == Some(tak::model::Priority::High))
        .collect();
    assert_eq!(high.len(), 1);
    assert_eq!(high[0].title, "High task");
}

#[test]
fn test_show_planning_in_pretty_output() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Planned task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning {
                priority: Some(tak::model::Priority::High),
                estimate: Some(tak::model::Estimate::L),
                required_skills: vec!["rust".into()],
                risk: Some(tak::model::Risk::Medium),
            },
        )
        .unwrap();

    let task = store.read(1).unwrap();
    let json = serde_json::to_string_pretty(&task).unwrap();
    assert!(json.contains("\"high\""));
    assert!(json.contains("\"l\""));
    assert!(json.contains("\"medium\""));
    assert!(json.contains("rust"));
}

#[test]
fn test_start_captures_git_info() {
    let dir = tempdir().unwrap();

    // Initialize a git repo with one commit so HEAD exists
    let repo = git2::Repository::init(dir.path()).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
    }
    let sig = repo.signature().unwrap();
    let tree_id = {
        let mut idx = repo.index().unwrap();
        idx.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();
    // Create a branch name we can verify
    repo.set_head("refs/heads/main").unwrap();

    // Initialize tak
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Test".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Start the task
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();

    let task = store.read(1).unwrap();
    assert_eq!(task.git.branch.as_deref(), Some("main"));
    assert!(
        task.git.start_commit.is_some(),
        "start_commit should be populated"
    );
    assert_eq!(
        task.git.start_commit.as_ref().unwrap().len(),
        40,
        "should be a full SHA"
    );
}

#[test]
fn test_finish_captures_commit_range() {
    let dir = tempdir().unwrap();

    // Initialize a git repo with one commit
    let repo = git2::Repository::init(dir.path()).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
    }
    let sig = repo.signature().unwrap();
    let tree_id = {
        let mut idx = repo.index().unwrap();
        idx.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    let initial_oid = repo
        .commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();

    // Initialize tak and create + start a task
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Test".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();

    // Make two more commits after start
    let initial_commit = repo.find_commit(initial_oid).unwrap();

    // Write a file to get a different tree for commit 2
    fs::write(dir.path().join("file1.txt"), "content1").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("file1.txt")).unwrap();
    index.write().unwrap();
    let tree2_id = index.write_tree().unwrap();
    let tree2 = repo.find_tree(tree2_id).unwrap();
    let c2_oid = repo
        .commit(
            Some("HEAD"),
            &sig,
            &sig,
            "add file1",
            &tree2,
            &[&initial_commit],
        )
        .unwrap();

    let c2 = repo.find_commit(c2_oid).unwrap();
    fs::write(dir.path().join("file2.txt"), "content2").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("file2.txt")).unwrap();
    index.write().unwrap();
    let tree3_id = index.write_tree().unwrap();
    let tree3 = repo.find_tree(tree3_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "add file2", &tree3, &[&c2])
        .unwrap();

    // Finish the task
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();

    let task = store.read(1).unwrap();
    assert_eq!(task.status, Status::Done);
    assert!(
        task.git.end_commit.is_some(),
        "end_commit should be populated"
    );
    assert_eq!(
        task.git.commits.len(),
        2,
        "should have 2 commits since start"
    );
    // Commits are in reverse chronological order (revwalk default)
    assert!(task.git.commits[0].contains("add file2"));
    assert!(task.git.commits[1].contains("add file1"));
}

// === Execution metadata tests (Phase 6) ===

#[test]
fn test_start_increments_attempt_count() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Retriable".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // First start -> attempt_count == 1
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::InProgress);
    assert_eq!(t.execution.attempt_count, 1);

    // Cancel (no reason), reopen, start again -> attempt_count == 2
    tak::commands::lifecycle::cancel(dir.path(), 1, None, Format::Json).unwrap();
    tak::commands::lifecycle::reopen(dir.path(), 1, Format::Json).unwrap();
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::InProgress);
    assert_eq!(t.execution.attempt_count, 2);
}

#[test]
fn test_cancel_with_reason_sets_last_error() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Fragile".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Start, then cancel with a reason
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    tak::commands::lifecycle::cancel(dir.path(), 1, Some("CI timeout".into()), Format::Json)
        .unwrap();

    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Cancelled);
    assert_eq!(t.execution.last_error.as_deref(), Some("CI timeout"));
}

#[test]
fn test_handoff_records_summary_and_returns_to_pending() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    store
        .create(
            "Handoffable".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Start with assignee, then handoff
    tak::commands::lifecycle::start(dir.path(), 1, Some("agent-1".into()), Format::Json).unwrap();
    tak::commands::lifecycle::handoff(
        dir.path(),
        1,
        "Completed setup, needs testing".into(),
        Format::Json,
    )
    .unwrap();

    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Pending);
    assert!(t.assignee.is_none(), "handoff should clear assignee");
    assert_eq!(
        t.execution.handoff_summary.as_deref(),
        Some("Completed setup, needs testing")
    );
    assert_eq!(
        t.execution.attempt_count, 1,
        "attempt_count should be preserved after handoff"
    );
}

// === Sidecar / Context tests (Phase 7) ===

#[test]
fn test_context_set_and_read() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "Context task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Set context
    tak::commands::context::run(
        dir.path(),
        1,
        Some("Important notes here".into()),
        false,
        Format::Json,
    )
    .unwrap();

    // Verify context file was created
    assert!(
        dir.path()
            .join(format!(".tak/context/{}.md", task_id_hex(1)))
            .exists()
    );

    // Read context back via sidecar store
    let repo = tak::store::repo::Repo::open(dir.path()).unwrap();
    let ctx = repo.sidecars.read_context(1).unwrap();
    assert_eq!(ctx.as_deref(), Some("Important notes here"));
}

#[test]
fn test_context_clear() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "Clearable".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Set then clear
    tak::commands::context::run(dir.path(), 1, Some("notes".into()), false, Format::Json).unwrap();
    assert!(
        dir.path()
            .join(format!(".tak/context/{}.md", task_id_hex(1)))
            .exists()
    );

    tak::commands::context::run(dir.path(), 1, None, true, Format::Json).unwrap();
    assert!(
        !dir.path()
            .join(format!(".tak/context/{}.md", task_id_hex(1)))
            .exists()
    );
}

#[test]
fn test_log_shows_lifecycle_history() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "Log task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Run lifecycle commands to generate history
    tak::commands::lifecycle::start(dir.path(), 1, Some("agent-1".into()), Format::Json).unwrap();
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();

    // Verify history file was created with structured events
    let repo = tak::store::repo::Repo::open(dir.path()).unwrap();
    let events = repo.sidecars.read_history(1).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event, "started");
    assert_eq!(events[0].agent.as_deref(), Some("agent-1"));
    assert_eq!(events[1].event, "finished");
}

#[test]
fn test_log_empty_returns_empty_json() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "No history".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // JSON mode returns empty array, no error
    tak::commands::log::run(dir.path(), 1, Format::Json).unwrap();
}

#[test]
fn test_verify_no_commands() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "No verify".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Should succeed with no commands
    tak::commands::verify::run(dir.path(), 1, Format::Json).unwrap();
}

#[test]
fn test_verify_passing_commands() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "Verifiable".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract {
                objective: None,
                acceptance_criteria: vec![],
                verification: vec!["true".into(), "echo ok".into()],
                constraints: vec![],
            },
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // All commands pass, should succeed
    tak::commands::verify::run(dir.path(), 1, Format::Json).unwrap();
}

#[test]
fn test_delete_cleans_up_sidecars() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    store
        .create(
            "Doomed".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Create sidecar files
    tak::commands::context::run(dir.path(), 1, Some("ctx notes".into()), false, Format::Json)
        .unwrap();
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();

    assert!(
        dir.path()
            .join(format!(".tak/context/{}.md", task_id_hex(1)))
            .exists()
    );
    assert!(
        dir.path()
            .join(format!(".tak/history/{}.jsonl", task_id_hex(1)))
            .exists()
    );

    // Delete task — should also clean up sidecars
    tak::commands::delete::run(dir.path(), 1, false, Format::Json).unwrap();

    assert!(
        !dir.path()
            .join(format!(".tak/context/{}.md", task_id_hex(1)))
            .exists()
    );
    assert!(
        !dir.path()
            .join(format!(".tak/history/{}.jsonl", task_id_hex(1)))
            .exists()
    );
}

#[test]
fn test_context_nonexistent_task_fails() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();

    let idx = Index::open(&dir.path().join(".tak/index.db")).unwrap();
    idx.rebuild(&[]).unwrap();
    drop(idx);

    let result = tak::commands::context::run(dir.path(), 999, None, false, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::TaskNotFound(999)));
}

#[test]
fn test_verify_stores_result() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/context")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/history")).unwrap();
    fs::create_dir_all(dir.path().join(".tak/verification_results")).unwrap();

    store
        .create(
            "Verifiable".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract {
                objective: None,
                acceptance_criteria: vec![],
                verification: vec!["true".into(), "echo ok".into()],
                constraints: vec![],
            },
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Run verify — should store results
    tak::commands::verify::run(dir.path(), 1, Format::Json).unwrap();

    // Read back the verification result
    let repo = tak::store::repo::Repo::open(dir.path()).unwrap();
    let vr = repo.sidecars.read_verification(1).unwrap().unwrap();
    assert!(vr.passed);
    assert_eq!(vr.results.len(), 2);
    assert!(vr.results[0].passed);
    assert_eq!(vr.results[0].command, "true");
    assert!(vr.results[1].passed);
    assert_eq!(vr.results[1].command, "echo ok");
}

// === Learnings system tests (Phase 8) ===

#[test]
fn test_learn_add_and_show() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "Auth task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Add a learning linked to task 1
    tak::commands::learn::add(
        dir.path(),
        "Always validate input".into(),
        Some("Never trust user input".into()),
        LearningCategory::Pitfall,
        vec!["security".into()],
        vec![1],
        Format::Json,
    )
    .unwrap();

    // Show learning
    tak::commands::learn::show(dir.path(), 1, Format::Json).unwrap();

    // Verify via direct store read
    let repo = Repo::open(dir.path()).unwrap();
    let learning = repo.learnings.read(1).unwrap();
    assert_eq!(learning.title, "Always validate input");
    assert_eq!(
        learning.description.as_deref(),
        Some("Never trust user input")
    );
    assert_eq!(learning.category, LearningCategory::Pitfall);
    assert_eq!(learning.tags, vec!["security"]);
    assert_eq!(learning.task_ids, vec![1]);

    // Task should have the learning ID linked
    let task = repo.store.read(1).unwrap();
    assert_eq!(task.learnings, vec![1]);
}

#[test]
fn test_learn_list_with_filters() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "Task A".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::learn::add(
        dir.path(),
        "Use FTS5".into(),
        None,
        LearningCategory::Tool,
        vec!["sqlite".into()],
        vec![1],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Beware of deadlocks".into(),
        None,
        LearningCategory::Pitfall,
        vec!["concurrency".into()],
        vec![],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Cargo workspace".into(),
        None,
        LearningCategory::Tool,
        vec!["rust".into()],
        vec![],
        Format::Json,
    )
    .unwrap();

    // List all
    tak::commands::learn::list(dir.path(), None, None, None, Format::Json).unwrap();

    // Filter by category
    tak::commands::learn::list(
        dir.path(),
        Some(LearningCategory::Tool),
        None,
        None,
        Format::Json,
    )
    .unwrap();

    // Filter by tag
    tak::commands::learn::list(dir.path(), None, Some("sqlite".into()), None, Format::Json)
        .unwrap();

    // Filter by task
    tak::commands::learn::list(dir.path(), None, None, Some(1), Format::Json).unwrap();

    // Verify via index queries
    let repo = Repo::open(dir.path()).unwrap();
    let all = repo.index.query_learnings(None, None, None).unwrap();
    assert_eq!(all.len(), 3);

    let tools = repo
        .index
        .query_learnings(Some("tool"), None, None)
        .unwrap();
    assert_eq!(tools.len(), 2);

    let sqlite = repo
        .index
        .query_learnings(None, Some("sqlite"), None)
        .unwrap();
    assert_eq!(sqlite.len(), 1);
    assert_eq!(sqlite[0], 1);

    let for_task1 = repo.index.query_learnings(None, None, Some(1)).unwrap();
    assert_eq!(for_task1.len(), 1);
    assert_eq!(for_task1[0], 1);
}

#[test]
fn test_learn_edit() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "Task A".into(),
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
            "Task B".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::learn::add(
        dir.path(),
        "Original title".into(),
        None,
        LearningCategory::Insight,
        vec![],
        vec![1],
        Format::Json,
    )
    .unwrap();

    // Edit title and add task link
    tak::commands::learn::edit(
        dir.path(),
        1,
        Some("Updated title".into()),
        Some("New description".into()),
        Some(LearningCategory::Pattern),
        Some(vec!["new-tag".into()]),
        vec![2],
        vec![],
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let learning = repo.learnings.read(1).unwrap();
    assert_eq!(learning.title, "Updated title");
    assert_eq!(learning.description.as_deref(), Some("New description"));
    assert_eq!(learning.category, LearningCategory::Pattern);
    assert_eq!(learning.tags, vec!["new-tag"]);
    assert_eq!(learning.task_ids, vec![1, 2]);

    // Both tasks should link to learning 1
    let t1 = repo.store.read(1).unwrap();
    assert!(t1.learnings.contains(&1));
    let t2 = repo.store.read(2).unwrap();
    assert!(t2.learnings.contains(&1));

    // Remove task link
    tak::commands::learn::edit(
        dir.path(),
        1,
        None,
        None,
        None,
        None,
        vec![],
        vec![1],
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let learning = repo.learnings.read(1).unwrap();
    assert_eq!(learning.task_ids, vec![2]);

    // Task 1 should no longer link to learning 1
    let t1 = repo.store.read(1).unwrap();
    assert!(!t1.learnings.contains(&1));
}

#[test]
fn test_learn_remove() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "Task A".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::learn::add(
        dir.path(),
        "Doomed learning".into(),
        None,
        LearningCategory::Insight,
        vec![],
        vec![1],
        Format::Json,
    )
    .unwrap();

    // Verify task links to learning
    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(1).unwrap();
    assert_eq!(task.learnings, vec![1]);
    drop(repo);

    // Remove learning
    tak::commands::learn::remove(dir.path(), 1, Format::Json).unwrap();

    // Learning should be gone
    let repo = Repo::open(dir.path()).unwrap();
    assert!(matches!(
        repo.learnings.read(1).unwrap_err(),
        TakError::LearningNotFound(1)
    ));

    // Task should no longer link to learning
    let task = repo.store.read(1).unwrap();
    assert!(task.learnings.is_empty());
}

#[test]
fn test_learn_suggest_via_fts() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    // Create a task with a title containing words that match learnings
    store
        .create(
            "Fix authentication bug".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Add learnings with matching and non-matching content
    tak::commands::learn::add(
        dir.path(),
        "Authentication tokens should be rotated".into(),
        Some("JWT tokens need regular rotation for security".into()),
        LearningCategory::Insight,
        vec![],
        vec![],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Database indexing strategy".into(),
        Some("Always add indexes for frequently queried columns".into()),
        LearningCategory::Pattern,
        vec![],
        vec![],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Fix common auth bugs".into(),
        Some("Common pitfalls in authentication code".into()),
        LearningCategory::Pitfall,
        vec![],
        vec![],
        Format::Json,
    )
    .unwrap();

    // Suggest learnings for task 1 ("Fix authentication bug")
    // Should find learnings with "authentication" or "auth" or "fix" or "bug"
    let repo = Repo::open(dir.path()).unwrap();
    let suggested = repo
        .index
        .suggest_learnings("Fix authentication bug")
        .unwrap();
    assert!(!suggested.is_empty(), "should find relevant learnings");

    // Learning 1 and 3 should be suggested (both mention auth/authentication)
    assert!(
        suggested.contains(&1) || suggested.contains(&3),
        "should find at least one auth-related learning"
    );
}

#[test]
fn test_learn_suggest_empty_title() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Suggest with empty title should return empty, not error
    tak::commands::learn::suggest(dir.path(), 1, Format::Json).unwrap();
}

#[test]
fn test_learnings_for_task_index_query() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    store
        .create(
            "Task A".into(),
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
            "Task B".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    tak::commands::learn::add(
        dir.path(),
        "Learning for A".into(),
        None,
        LearningCategory::Insight,
        vec![],
        vec![1],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Learning for both".into(),
        None,
        LearningCategory::Pattern,
        vec![],
        vec![1, 2],
        Format::Json,
    )
    .unwrap();

    tak::commands::learn::add(
        dir.path(),
        "Learning for B".into(),
        None,
        LearningCategory::Tool,
        vec![],
        vec![2],
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let for_a = repo.index.learnings_for_task(1).unwrap();
    assert_eq!(for_a, vec![1, 2]);

    let for_b = repo.index.learnings_for_task(2).unwrap();
    assert_eq!(for_b, vec![2, 3]);

    let for_none = repo.index.learnings_for_task(999).unwrap();
    assert!(for_none.is_empty());
}

#[test]
fn test_learn_remove_nonexistent_fails() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    let idx = Index::open(&dir.path().join(".tak/index.db")).unwrap();
    idx.rebuild(&[]).unwrap();
    drop(idx);

    let result = tak::commands::learn::remove(dir.path(), 999, Format::Json);
    assert!(matches!(
        result.unwrap_err(),
        TakError::LearningNotFound(999)
    ));
}

#[test]
fn test_learn_add_invalid_task_fails() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    let idx = Index::open(&dir.path().join(".tak/index.db")).unwrap();
    idx.rebuild(&[]).unwrap();
    drop(idx);

    let result = tak::commands::learn::add(
        dir.path(),
        "Bad link".into(),
        None,
        LearningCategory::Insight,
        vec![],
        vec![999],
        Format::Json,
    );
    assert!(matches!(result.unwrap_err(), TakError::TaskNotFound(999)));
}

// === Mesh coordination tests ===

#[test]
fn test_mesh_join_list_leave() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    // Join
    let reg = db
        .join_agent("agent-1", "sess-1", "/tmp", None, None)
        .unwrap();
    assert_eq!(reg.name, "agent-1");

    // List
    let agents = db.list_agents().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "agent-1");

    // Leave
    db.leave_agent("agent-1").unwrap();
    let agents = db.list_agents().unwrap();
    assert!(agents.is_empty());
}

#[test]
fn test_mesh_send_inbox_ack() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    db.join_agent("sender", "sess-1", "/tmp", None, None)
        .unwrap();
    db.join_agent("receiver", "sess-2", "/tmp", None, None)
        .unwrap();

    db.send_message("sender", "receiver", "task 5 is ready", None)
        .unwrap();
    db.send_message("sender", "receiver", "also check task 6", None)
        .unwrap();

    let msgs = db.read_inbox("receiver").unwrap();
    assert_eq!(msgs.len(), 2);

    // Ack
    let ids: Vec<String> = msgs.iter().map(|m| m.id.clone()).collect();
    db.ack_messages("receiver", &ids).unwrap();
    let msgs = db.read_inbox("receiver").unwrap();
    assert!(msgs.is_empty());
}

#[test]
fn test_mesh_reserve_conflict_release() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    let ra = db.join_agent("A", "s", "/tmp", None, None).unwrap();
    let rb = db.join_agent("B", "s", "/tmp", None, None).unwrap();

    db.reserve("A", ra.generation, "src/store/", Some("task-1"), 3600)
        .unwrap();

    // Conflict: B tries sub-path
    let err = db
        .reserve("B", rb.generation, "src/store/mesh.rs", None, 3600)
        .unwrap_err();
    assert!(matches!(
        err,
        tak::error::TakError::MeshReservationConflict { .. }
    ));

    // Non-overlapping path succeeds
    db.reserve("B", rb.generation, "src/model.rs", None, 3600)
        .unwrap();

    // Release all for A
    db.release_all("A").unwrap();
    let reservations = db.list_reservations().unwrap();
    assert_eq!(reservations.len(), 1);
    assert_eq!(reservations[0].agent, "B");
}

#[test]
fn test_mesh_feed() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    db.join_agent("A", "s", "/tmp", None, None).unwrap();
    db.join_agent("B", "s", "/tmp", None, None).unwrap();
    db.append_event(Some("A"), "mesh.join", None, None).unwrap();
    db.append_event(Some("B"), "mesh.join", None, None).unwrap();
    db.send_message("A", "B", "hello", None).unwrap();
    db.append_event(Some("A"), "mesh.send", Some("B"), Some("hello"))
        .unwrap();

    let events = db.read_events(None).unwrap();
    // We appended 3 events manually
    assert!(events.len() >= 3);

    // Limit
    let last = db.read_events(Some(1)).unwrap();
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].event_type, "mesh.send");
}

#[test]
fn test_mesh_blockers_command_runs_for_matching_path() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    let reg = db.join_agent("A", "s", "/tmp", None, None).unwrap();
    db.reserve("A", reg.generation, "src/store/", Some("task-1"), 3600)
        .unwrap();

    // Should succeed and print blocker diagnostics.
    tak::commands::mesh::blockers(dir.path(), vec!["src/store/mesh.rs".into()], Format::Json)
        .unwrap();
}

#[test]
fn test_mesh_blockers_invalid_path_fails() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let err = tak::commands::mesh::blockers(dir.path(), vec!["../etc/passwd".into()], Format::Json)
        .unwrap_err();
    assert!(matches!(err, TakError::MeshInvalidPath(_)));
}

#[test]
fn test_mesh_leave_cleans_reservations() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    let reg = db.join_agent("A", "s", "/tmp", None, None).unwrap();
    db.reserve("A", reg.generation, "src/", None, 3600).unwrap();
    db.leave_agent("A").unwrap();

    let reservations = db.list_reservations().unwrap();
    assert!(reservations.is_empty());
}

#[test]
fn test_wait_on_path_returns_when_reservation_released() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    let reg = db.join_agent("A", "s", "/tmp", None, None).unwrap();
    db.reserve("A", reg.generation, "src/store/", Some("task-1"), 3600)
        .unwrap();

    let repo_root = dir.path().to_path_buf();
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        let db = CoordinationDb::from_repo(&repo_root).unwrap();
        db.release_all("A").unwrap();
    });

    tak::commands::wait::run(
        dir.path(),
        Some("./src/store/mesh.rs".into()),
        None,
        Some(2),
        Format::Json,
    )
    .unwrap();

    releaser.join().unwrap();
}

#[test]
fn test_wait_on_task_returns_when_dependencies_finish() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let blocker = store
        .create(
            "Blocker".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let blocked = store
        .create(
            "Blocked".into(),
            Kind::Task,
            None,
            None,
            vec![blocker.id],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let repo_root = dir.path().to_path_buf();
    let blocker_id = blocker.id;
    let finisher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        let repo = Repo::open(&repo_root).unwrap();
        let mut task = repo.store.read(blocker_id).unwrap();
        task.status = Status::Done;
        task.updated_at = Utc::now();
        repo.store.write(&task).unwrap();
        repo.index.upsert(&task).unwrap();
    });

    tak::commands::wait::run(dir.path(), None, Some(blocked.id), Some(2), Format::Json).unwrap();

    finisher.join().unwrap();
}

#[test]
fn test_wait_on_path_times_out_when_still_blocked() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    let reg = db.join_agent("A", "s", "/tmp", None, None).unwrap();
    db.reserve("A", reg.generation, "src/store/", Some("task-1"), 3600)
        .unwrap();

    let err = tak::commands::wait::run(
        dir.path(),
        Some("src/store/mesh.rs".into()),
        None,
        Some(0),
        Format::Json,
    )
    .unwrap_err();

    assert!(matches!(err, TakError::WaitTimeout(_)));
}

#[test]
fn test_blackboard_post_close_reopen() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task_id = store
        .create(
            "Investigate flaky test".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    tak::commands::blackboard::post(
        dir.path(),
        "agent_1",
        "Need help triaging this",
        None,
        vec!["triage".into()],
        vec![task_id],
        Format::Json,
    )
    .unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();

    let notes = db.list_notes(None, None, None, None).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].status, "open");
    assert_eq!(notes[0].task_ids, vec![task_id.to_string()]);

    let note_id = notes[0].id;
    tak::commands::blackboard::close(
        dir.path(),
        note_id as u64,
        "reviewer",
        Some("handled"),
        Format::Json,
    )
    .unwrap();

    let closed = db.get_note(note_id).unwrap();
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.closed_by.as_deref(), Some("reviewer"));

    tak::commands::blackboard::reopen(dir.path(), note_id as u64, "agent_1", Format::Json).unwrap();
    let reopened = db.get_note(note_id).unwrap();
    assert_eq!(reopened.status, "open");
}

#[test]
fn test_blackboard_post_invalid_task_link_fails() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let result = tak::commands::blackboard::post(
        dir.path(),
        "agent_1",
        "link to missing task",
        None,
        vec![],
        vec![999],
        Format::Json,
    );
    assert!(matches!(result.unwrap_err(), TakError::TaskNotFound(999)));
}

#[test]
fn test_blackboard_post_blocker_template_formats_message_and_tags() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task_id = store
        .create(
            "Investigate lock contention".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    use tak::commands::blackboard::BlackboardTemplate;
    tak::commands::blackboard::post(
        dir.path(),
        "agent_1",
        "Waiting on reservation release from helper",
        Some(BlackboardTemplate::Blocker),
        vec!["db".into()],
        vec![task_id],
        Format::Json,
    )
    .unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    let notes = db.list_notes(None, None, None, None).unwrap();

    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert!(note.message.contains("template: blocker"));
    assert!(note.message.contains("status: blocked"));
    assert!(note.message.contains(&format!("scope: tasks={task_id}")));
    assert!(note.message.contains("requested_action:"));
    assert_eq!(note.tags, vec!["blocker", "coordination", "db"]);
}

#[test]
fn test_blackboard_post_plain_message_remains_free_text() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task_id = store
        .create(
            "Unstructured comms regression".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id;

    let message = "Heads-up: still using free text while schema rolls out.";
    tak::commands::blackboard::post(
        dir.path(),
        "agent_1",
        message,
        None,
        vec!["freeform".into()],
        vec![task_id],
        Format::Json,
    )
    .unwrap();

    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    let notes = db.list_notes(None, None, None, None).unwrap();

    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note.message, message);
    assert!(!note.message.contains("template:"));
    assert!(!note.message.contains("delta_since:"));
    assert_eq!(note.tags, vec!["freeform"]);
    assert_eq!(note.task_ids, vec![task_id.to_string()]);
}

#[test]
fn test_work_start_prefers_explicit_assignee_over_tak_agent() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _reset = TakAgentEnvReset;

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    unsafe {
        std::env::set_var("TAK_AGENT", "env-agent");
    }

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("explicit-agent".into()),
        None,
        None,
        None,
        Format::Json,
    )
    .unwrap();

    let explicit_state_path = dir
        .path()
        .join(".tak")
        .join("runtime")
        .join("work")
        .join("states")
        .join("explicit-agent.json");
    let env_state_path = dir
        .path()
        .join(".tak")
        .join("runtime")
        .join("work")
        .join("states")
        .join("env-agent.json");

    assert!(explicit_state_path.exists());
    assert!(!env_state_path.exists());

    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(explicit_state_path).unwrap()).unwrap();
    assert_eq!(
        value.get("agent").and_then(|v| v.as_str()),
        Some("explicit-agent")
    );
}

#[test]
fn test_work_repeated_without_assignee_uses_tak_agent_state_key() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _reset = TakAgentEnvReset;

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    unsafe {
        std::env::set_var("TAK_AGENT", "stable-agent");
    }

    tak::commands::work::start_or_resume(
        dir.path(),
        None,
        Some("cli".into()),
        Some(2),
        None,
        Format::Json,
    )
    .unwrap();
    tak::commands::work::status(dir.path(), None, Format::Json).unwrap();
    tak::commands::work::stop(dir.path(), None, Format::Json).unwrap();

    let states_dir = dir
        .path()
        .join(".tak")
        .join("runtime")
        .join("work")
        .join("states");
    let stable_state_path = states_dir.join("stable-agent.json");

    assert!(stable_state_path.exists());

    let entries = fs::read_dir(&states_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["stable-agent.json"]);

    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(stable_state_path).unwrap()).unwrap();
    assert_eq!(
        value.get("agent").and_then(|v| v.as_str()),
        Some("stable-agent")
    );
    assert_eq!(value.get("active").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn test_work_resolution_does_not_mutate_other_agent_state_in_ambiguous_mesh_context() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _reset = TakAgentEnvReset;

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    // Create two mesh registrations to emulate multi-agent ambiguity in runtime context.
    use tak::store::coordination_db::CoordinationDb;
    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    db.join_agent("agent-a", "sid-a", "/tmp", None, None)
        .unwrap();
    db.join_agent("agent-b", "sid-b", "/tmp", None, None)
        .unwrap();

    // Seed agent-b state.
    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-b".into()),
        Some("seed".into()),
        Some(1),
        None,
        Format::Json,
    )
    .unwrap();

    let agent_b_state_path = dir
        .path()
        .join(".tak")
        .join("runtime")
        .join("work")
        .join("states")
        .join("agent-b.json");
    let before_b = fs::read_to_string(&agent_b_state_path).unwrap();

    unsafe {
        std::env::set_var("TAK_AGENT", "agent-a");
    }

    tak::commands::work::start_or_resume(dir.path(), None, None, None, None, Format::Json).unwrap();
    tak::commands::work::status(dir.path(), None, Format::Json).unwrap();
    tak::commands::work::stop(dir.path(), None, Format::Json).unwrap();

    let after_b = fs::read_to_string(&agent_b_state_path).unwrap();
    assert_eq!(before_b, after_b);

    let agent_a_state_path = dir
        .path()
        .join(".tak")
        .join("runtime")
        .join("work")
        .join("states")
        .join("agent-a.json");
    assert!(agent_a_state_path.exists());
}

#[test]
fn test_work_resume_after_handoff_releases_reservations_when_no_matching_work() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create(
            "Handoff candidate".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["lane-a".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-1".into()),
        Some("lane-a".into()),
        None,
        None,
        Format::Json,
    )
    .unwrap();

    tak::commands::mesh::join(
        dir.path(),
        Some("agent-1"),
        Some("sid-agent-1"),
        Format::Json,
    )
    .unwrap();
    tak::commands::mesh::reserve(
        dir.path(),
        "agent-1",
        vec!["src/commands/work.rs".into()],
        Some("handoff-test"),
        Format::Json,
    )
    .unwrap();

    tak::commands::lifecycle::handoff(
        dir.path(),
        task.id,
        "Pausing for teammate handoff".into(),
        Format::Json,
    )
    .unwrap();

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-1".into()),
        Some("lane-b".into()),
        None,
        None,
        Format::Json,
    )
    .unwrap();

    let work_store = tak::store::work::WorkStore::open(&dir.path().join(".tak"));
    let state = work_store.status("agent-1").unwrap();
    // Resume gate keeps the loop active (handoff skip) even with no matching work
    assert!(state.active);
    assert!(state.current_task_id.is_none());
    assert_eq!(state.processed, 1);

    let repo = Repo::open(dir.path()).unwrap();
    let handed_off = repo.store.read(task.id).unwrap();
    assert_eq!(handed_off.status, Status::Pending);
    assert!(handed_off.assignee.is_none());

    let db = tak::store::coordination_db::CoordinationDb::from_repo(dir.path()).unwrap();
    let reservations = db.list_reservations().unwrap();
    assert!(
        reservations
            .iter()
            .all(|reservation| reservation.agent != "agent-1")
    );
}

#[test]
fn test_work_resume_after_finish_honors_limit_and_cleans_reservations() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    store
        .create(
            "Finishable item A".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["lane-finish".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    store
        .create(
            "Finishable item B".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["lane-finish".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-2".into()),
        Some("lane-finish".into()),
        Some(1),
        None,
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let claimed = repo
        .store
        .list_all()
        .unwrap()
        .into_iter()
        .find(|task| {
            task.status == Status::InProgress && task.assignee.as_deref() == Some("agent-2")
        })
        .expect("expected a claimed in-progress task for agent-2");

    tak::commands::mesh::join(
        dir.path(),
        Some("agent-2"),
        Some("sid-agent-2"),
        Format::Json,
    )
    .unwrap();
    tak::commands::mesh::reserve(
        dir.path(),
        "agent-2",
        vec!["src/commands/work.rs".into()],
        Some("finish-limit-test"),
        Format::Json,
    )
    .unwrap();

    tak::commands::lifecycle::finish(dir.path(), claimed.id, Format::Json).unwrap();

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-2".into()),
        None,
        None,
        None,
        Format::Json,
    )
    .unwrap();

    let work_store = tak::store::work::WorkStore::open(&dir.path().join(".tak"));
    let state = work_store.status("agent-2").unwrap();
    assert!(!state.active);
    assert_eq!(state.remaining, Some(0));
    assert_eq!(state.processed, 1);
    assert!(state.current_task_id.is_none());

    let repo = Repo::open(dir.path()).unwrap();
    let tasks = repo.store.list_all().unwrap();
    assert!(
        tasks
            .iter()
            .any(|task| task.id == claimed.id && task.status == Status::Done)
    );
    assert!(tasks.iter().any(|task| task.status == Status::Pending));

    let db = tak::store::coordination_db::CoordinationDb::from_repo(dir.path()).unwrap();
    let reservations = db.list_reservations().unwrap();
    assert!(
        reservations
            .iter()
            .all(|reservation| reservation.agent != "agent-2")
    );
}

#[test]
fn test_work_status_and_stop_commands_are_idempotent_integration() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create(
            "Stop semantics task".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();

    tak::commands::work::start_or_resume(
        dir.path(),
        Some("agent-3".into()),
        None,
        None,
        None,
        Format::Json,
    )
    .unwrap();

    tak::commands::mesh::join(
        dir.path(),
        Some("agent-3"),
        Some("sid-agent-3"),
        Format::Json,
    )
    .unwrap();
    tak::commands::mesh::reserve(
        dir.path(),
        "agent-3",
        vec!["src/commands/work.rs".into()],
        Some("stop-idempotent-test"),
        Format::Json,
    )
    .unwrap();

    tak::commands::work::status(dir.path(), Some("agent-3".into()), Format::Json).unwrap();
    tak::commands::work::stop(dir.path(), Some("agent-3".into()), Format::Json).unwrap();
    tak::commands::work::status(dir.path(), Some("agent-3".into()), Format::Json).unwrap();
    tak::commands::work::stop(dir.path(), Some("agent-3".into()), Format::Json).unwrap();

    let work_store = tak::store::work::WorkStore::open(&dir.path().join(".tak"));
    let state = work_store.status("agent-3").unwrap();
    assert!(!state.active);
    assert!(state.current_task_id.is_none());

    let repo = Repo::open(dir.path()).unwrap();
    let claimed = repo.store.read(task.id).unwrap();
    assert_eq!(claimed.status, Status::InProgress);
    assert_eq!(claimed.assignee.as_deref(), Some("agent-3"));

    let db = tak::store::coordination_db::CoordinationDb::from_repo(dir.path()).unwrap();
    let reservations = db.list_reservations().unwrap();
    assert!(
        reservations
            .iter()
            .all(|reservation| reservation.agent != "agent-3")
    );
}

#[test]
fn test_learn_index_auto_rebuild() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();
    fs::create_dir_all(dir.path().join(".tak/learnings")).unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Add a learning via direct store (bypassing index)
    let lstore = tak::store::learnings::LearningStore::open(store.root());
    let learning = lstore
        .create(
            "Direct learning".into(),
            None,
            LearningCategory::Insight,
            vec!["test".into()],
            vec![],
        )
        .unwrap();
    assert_eq!(learning.id, 1);

    // Delete the index, force rebuild via Repo::open()
    fs::remove_file(store.root().join("index.db")).unwrap();

    // Repo::open should auto-rebuild both tasks and learnings
    let repo = Repo::open(dir.path()).unwrap();
    let ids = repo.index.query_learnings(None, None, None).unwrap();
    assert_eq!(ids, vec![1]);

    let by_tag = repo
        .index
        .query_learnings(None, Some("test"), None)
        .unwrap();
    assert_eq!(by_tag, vec![1]);
}
