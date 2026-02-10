use std::sync::{Arc, Barrier};

use chrono::{Duration, Utc};
use tak::commands::takeover;
use tak::error::TakError;
use tak::model::{Contract, Kind, Planning, Status};
use tak::output::Format;
use tak::store::coordination_db::CoordinationDb;
use tak::store::files::FileStore;
use tak::store::repo::Repo;
use tempfile::tempdir;

fn create_in_progress_task(repo_root: &std::path::Path, title: &str, assignee: &str) -> u64 {
    let repo = Repo::open(repo_root).unwrap();
    let mut task = repo
        .store
        .create(
            title.to_string(),
            Kind::Task,
            None,
            None,
            vec![],
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    task.status = Status::InProgress;
    task.assignee = Some(assignee.to_string());
    task.updated_at = Utc::now();
    repo.store.write(&task).unwrap();
    repo.index.upsert(&task).unwrap();
    task.id
}

fn set_registration_last_seen(repo_root: &std::path::Path, owner: &str, secs_ago: i64) {
    let db_path = repo_root
        .join(".tak")
        .join("runtime")
        .join("coordination.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ts = (Utc::now() - Duration::seconds(secs_ago)).to_rfc3339();
    conn.execute(
        "UPDATE agents SET updated_at = ?1 WHERE name = ?2",
        rusqlite::params![ts, owner],
    )
    .unwrap();
}

#[test]
fn takeover_succeeds_for_stale_owner_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_in_progress_task(dir.path(), "takeover-stale-owner", "owner-1");

    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    db.join_agent("owner-1", "sid-owner", "/tmp", None, None).unwrap();
    set_registration_last_seen(dir.path(), "owner-1", 3600);

    takeover::run(
        dir.path(),
        task_id,
        "agent-2".into(),
        Some(300),
        false,
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert_eq!(task.status, Status::InProgress);
    assert_eq!(task.assignee.as_deref(), Some("agent-2"));

    let history = repo.sidecars.read_history(task_id).unwrap();
    assert!(history.iter().any(|event| event.event == "takeover"));
}

#[test]
fn takeover_rejects_active_owner_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_in_progress_task(dir.path(), "takeover-active-owner", "owner-1");

    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    db.join_agent("owner-1", "sid-owner", "/tmp", None, None).unwrap();

    let err = takeover::run(
        dir.path(),
        task_id,
        "agent-2".into(),
        Some(300),
        false,
        Format::Minimal,
    )
    .unwrap_err();

    match err {
        TakError::Locked(message) => assert!(message.contains("owner 'owner-1' is active")),
        other => panic!("unexpected error: {other:?}"),
    }

    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert_eq!(task.assignee.as_deref(), Some("owner-1"));
}

#[test]
fn takeover_concurrent_attempts_allow_single_winner_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_in_progress_task(dir.path(), "takeover-race", "owner-1");

    let db = CoordinationDb::from_repo(dir.path()).unwrap();
    db.join_agent("owner-1", "sid-owner", "/tmp", None, None).unwrap();
    db.join_agent("agent-a", "sid-a", "/tmp", None, None).unwrap();
    db.join_agent("agent-b", "sid-b", "/tmp", None, None).unwrap();
    set_registration_last_seen(dir.path(), "owner-1", 3600);

    let repo_root = Arc::new(dir.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(3));

    let make_runner = |agent: &'static str| {
        let repo_root = Arc::clone(&repo_root);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            takeover::run(
                repo_root.as_path(),
                task_id,
                agent.to_string(),
                Some(300),
                false,
                Format::Minimal,
            )
        })
    };

    let t1 = make_runner("agent-a");
    let t2 = make_runner("agent-b");
    barrier.wait();

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();

    let ok_count = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
    assert_eq!(ok_count, 1, "expected exactly one successful takeover");

    let err = if let Err(err) = r1 {
        err
    } else {
        r2.unwrap_err()
    };
    match err {
        TakError::Locked(message) => assert!(message.contains("is active")),
        other => panic!("unexpected error: {other:?}"),
    }

    let repo = Repo::open(repo_root.as_path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert!(
        matches!(task.assignee.as_deref(), Some("agent-a") | Some("agent-b")),
        "unexpected final owner: {:?}",
        task.assignee
    );
}
