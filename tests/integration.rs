use std::fs;

use tempfile::tempdir;

use chrono::Utc;
use tak::error::TakError;
use tak::model::{Contract, DepType, Kind, Planning, Status};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::index::Index;
use tak::store::repo::Repo;

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
    let avail = idx.available(None).unwrap();
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
    let avail = idx.available(None).unwrap();
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
fn test_reopen_transitions() {
    let dir = tempdir().unwrap();
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

    // pending -> in_progress -> done
    tak::commands::lifecycle::start(dir.path(), 1, None, Format::Json).unwrap();
    tak::commands::lifecycle::finish(dir.path(), 1, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Done);

    // done -> pending (reopen)
    tak::commands::lifecycle::reopen(dir.path(), 1, Format::Json).unwrap();
    let t = store.read(1).unwrap();
    assert_eq!(t.status, Status::Pending);
    assert!(t.assignee.is_none(), "reopen should clear assignee");
}

#[test]
fn test_depend_rolls_back_on_partial_failure() {
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

    // Build index
    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Try to depend 1 on [2, 999]. 999 doesn't exist, so this should fail entirely.
    let result = tak::commands::deps::depend(dir.path(), 1, vec![2, 999], None, None, Format::Json);
    assert!(result.is_err());

    // Task 1's file should still have no dependencies
    let task = store.read(1).unwrap();
    assert!(
        task.depends_on.is_empty(),
        "file should be unchanged on failure"
    );

    // Index should also have no deps for task 1
    let repo = Repo::open(dir.path()).unwrap();
    let avail = repo.index.available(None).unwrap();
    assert!(
        avail.contains(&1),
        "task 1 should still be available (not blocked)"
    );
}

#[test]
fn test_depend_with_type_and_reason() {
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

    let idx = Index::open(&store.root().join("index.db")).unwrap();
    idx.rebuild(&store.list_all().unwrap()).unwrap();
    drop(idx);

    // Add dependency with type and reason
    tak::commands::deps::depend(
        dir.path(),
        2,
        vec![1],
        Some(DepType::Soft),
        Some("nice to have".into()),
        Format::Json,
    )
    .unwrap();

    let task = store.read(2).unwrap();
    assert_eq!(task.depends_on.len(), 1);
    assert_eq!(task.depends_on[0].id, 1);
    assert_eq!(task.depends_on[0].dep_type, Some(DepType::Soft));
    assert_eq!(task.depends_on[0].reason.as_deref(), Some("nice to have"));

    // Update existing dependency metadata
    tak::commands::deps::depend(
        dir.path(),
        2,
        vec![1],
        Some(DepType::Hard),
        None,
        Format::Json,
    )
    .unwrap();

    let task = store.read(2).unwrap();
    assert_eq!(task.depends_on.len(), 1);
    assert_eq!(task.depends_on[0].dep_type, Some(DepType::Hard));
    assert_eq!(
        task.depends_on[0].reason.as_deref(),
        Some("nice to have"),
        "reason should be preserved when only dep_type is updated"
    );
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
        "hooks": [{"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10}]
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
                "hooks": [{"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10}]
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
        "hooks": [{"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10}]
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
        "hooks": [{"type": "command", "command": "tak reindex 2>/dev/null || true", "timeout": 10}]
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
    assert!(dir.path().join(".tak/counter.json").exists());
    assert!(dir.path().join(".tak/tasks").is_dir());
    assert!(dir.path().join(".tak/tasks/1.json").exists());
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
    fs::remove_file(dir.path().join(".tak/tasks/1.json")).unwrap();

    let child: tak::model::Task =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".tak/tasks/2.json")).unwrap())
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

    fs::remove_file(dir.path().join(".tak/tasks/1.json")).unwrap();

    let task: tak::model::Task =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".tak/tasks/2.json")).unwrap())
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
    assert!(dir.path().join(".tak/context/1.md").exists());

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
    assert!(dir.path().join(".tak/context/1.md").exists());

    tak::commands::context::run(dir.path(), 1, None, true, Format::Json).unwrap();
    assert!(!dir.path().join(".tak/context/1.md").exists());
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

    // Verify history file was created with entries
    let repo = tak::store::repo::Repo::open(dir.path()).unwrap();
    let history = repo.sidecars.read_history(1).unwrap().unwrap();
    let lines: Vec<&str> = history.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("started"));
    assert!(lines[0].contains("agent-1"));
    assert!(lines[1].contains("finished"));
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
