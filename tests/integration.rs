use tempfile::tempdir;

use chrono::Utc;
use tak::error::TakError;
use tak::model::{Kind, Status};
use tak::store::files::FileStore;
use tak::store::index::Index;
use tak::output::Format;
use tak::store::repo::Repo;

#[test]
fn test_full_workflow() {
    let dir = tempdir().unwrap();

    // Init
    let store = FileStore::init(dir.path()).unwrap();

    // Create epic
    let epic = store
        .create("Auth system".into(), Kind::Epic, None, None, vec![], vec![])
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
        )
        .unwrap();

    // Build index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    let all = store.list_all().unwrap();
    idx.rebuild(&all).unwrap();

    // Check available: tasks 1 and 2 have no unfinished deps
    // Task 3 is blocked by 2, task 4 is blocked by 3
    let avail = idx.available().unwrap();
    assert!(avail.contains(&1));
    assert!(avail.contains(&2));
    assert!(!avail.contains(&3));
    assert!(!avail.contains(&4));

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
    let avail = idx.available().unwrap();
    assert!(avail.contains(&3));
    assert!(!avail.contains(&4)); // still blocked by 3

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
    let avail = idx.available().unwrap();
    assert!(avail.contains(&4));

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
        .create("A".into(), Kind::Task, None, None, vec![], vec![])
        .unwrap();
    store
        .create("B".into(), Kind::Task, None, None, vec![1], vec![])
        .unwrap();
    store
        .create("C".into(), Kind::Task, None, None, vec![2], vec![])
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
        .create("Task A".into(), Kind::Task, None, None, vec![], vec![])
        .unwrap();
    store
        .create("Task B".into(), Kind::Task, None, None, vec![1], vec![])
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
    let avail = repo.index.available().unwrap();
    assert_eq!(avail, vec![1]);
    let blocked = repo.index.blocked().unwrap();
    assert_eq!(blocked, vec![2]);
}

#[test]
fn test_status_transitions() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let task = store
        .create("Test".into(), Kind::Task, None, None, vec![], vec![])
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

    store.create("Task A".into(), Kind::Task, None, None, vec![], vec![]).unwrap();
    store.create("Task B".into(), Kind::Task, None, None, vec![1], vec![]).unwrap();

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Claim as agent-1 — should get task 1 (only available)
    tak::commands::claim::run(dir.path(), "agent-1".into(), None, Format::Json).unwrap();

    let t1 = store.read(1).unwrap();
    assert_eq!(t1.status, Status::InProgress);
    assert_eq!(t1.assignee.as_deref(), Some("agent-1"));

    // Task 2 is still blocked, nothing available
    let result = tak::commands::claim::run(dir.path(), "agent-2".into(), None, Format::Json);
    assert!(matches!(result.unwrap_err(), TakError::NoAvailableTask));
}

#[test]
fn test_depend_rolls_back_on_partial_failure() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    store.create("A".into(), Kind::Task, None, None, vec![], vec![]).unwrap();
    store.create("B".into(), Kind::Task, None, None, vec![], vec![]).unwrap();

    // Build index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Try to depend 1 on [2, 999]. 999 doesn't exist, so this should fail entirely.
    let result = tak::commands::deps::depend(dir.path(), 1, vec![2, 999], Format::Json);
    assert!(result.is_err());

    // Task 1's file should still have no dependencies
    let task = store.read(1).unwrap();
    assert!(task.depends_on.is_empty(), "file should be unchanged on failure");

    // Index should also have no deps for task 1
    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available().unwrap();
    assert!(avail.contains(&1), "task 1 should still be available (not blocked)");
}
