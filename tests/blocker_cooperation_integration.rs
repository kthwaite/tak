use std::thread;
use std::time::Duration;

use tempfile::tempdir;

use tak::commands::blackboard::BlackboardTemplate;
use tak::error::TakError;
use tak::model::{Contract, Kind, Planning, Status};
use tak::output::Format;
use tak::store::blackboard::{BlackboardStatus, BlackboardStore};
use tak::store::files::FileStore;
use tak::store::work::WorkClaimStrategy;

#[test]
fn cli_only_blocker_cooperation_flow_times_out_then_unblocks_and_claims_next_task() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    let store = FileStore::init(repo_root).unwrap();

    let blocker = store
        .create(
            "Release blocker reservation".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["coordination".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    let blocked = store
        .create(
            "Continue blocked integration work".into(),
            Kind::Task,
            None,
            None,
            vec![blocker.id],
            vec!["coordination".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    tak::commands::lifecycle::start(
        repo_root,
        blocker.id,
        Some("owner-agent".into()),
        Format::Json,
    )
    .unwrap();

    tak::commands::mesh::join(
        repo_root,
        Some("owner-agent"),
        Some("sid-owner"),
        Format::Json,
    )
    .unwrap();
    tak::commands::mesh::join(
        repo_root,
        Some("helper-agent"),
        Some("sid-helper"),
        Format::Json,
    )
    .unwrap();

    let reservation_reason = format!("task-{}", blocker.id);
    tak::commands::mesh::reserve(
        repo_root,
        "owner-agent",
        vec!["src/store/".into()],
        Some(reservation_reason.as_str()),
        Format::Json,
    )
    .unwrap();

    tak::commands::blackboard::post(
        repo_root,
        "owner-agent",
        "Waiting for helper-safe unblock before continuing",
        Some(BlackboardTemplate::Blocker),
        vec!["cli-only".into()],
        vec![blocked.id],
        Format::Json,
    )
    .unwrap();

    let board = BlackboardStore::open(&repo_root.join(".tak"));
    let notes = board
        .list(
            Some(BlackboardStatus::Open),
            Some("blocker"),
            Some(blocked.id),
            None,
        )
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].message.contains("template: blocker"));
    assert_eq!(notes[0].task_ids, vec![blocked.id]);

    let note_id = notes[0].id;

    let wait_timeout = tak::commands::wait::run(
        repo_root,
        Some("src/store/work.rs".into()),
        None,
        Some(0),
        Format::Json,
    )
    .unwrap_err();

    assert!(matches!(
        wait_timeout,
        TakError::WaitTimeout(msg)
            if msg.contains("owner-agent") && msg.contains("src/store/")
    ));

    let unblock_root = repo_root.to_path_buf();
    let blocker_id = blocker.id;
    let unblock = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        tak::commands::mesh::release(&unblock_root, "owner-agent", vec![], true, Format::Json)
            .unwrap();
        tak::commands::lifecycle::finish(&unblock_root, blocker_id, Format::Json).unwrap();
    });

    tak::commands::wait::run(
        repo_root,
        Some("src/store/work.rs".into()),
        None,
        Some(2),
        Format::Json,
    )
    .unwrap();
    tak::commands::wait::run(repo_root, None, Some(blocked.id), Some(2), Format::Json).unwrap();
    unblock.join().unwrap();

    let claimed = tak::commands::claim::claim_next(
        repo_root,
        "helper-agent",
        None,
        WorkClaimStrategy::PriorityThenAge,
    )
    .unwrap()
    .expect("helper should claim the newly unblocked task");

    assert_eq!(claimed.id, blocked.id);
    assert_eq!(claimed.assignee.as_deref(), Some("helper-agent"));
    assert_eq!(claimed.status, Status::InProgress);

    tak::commands::blackboard::close(
        repo_root,
        note_id,
        "owner-agent",
        Some("released reservation and finished blocker"),
        Format::Json,
    )
    .unwrap();

    let closed = board.get(note_id).unwrap();
    assert_eq!(closed.status, BlackboardStatus::Closed);
}
