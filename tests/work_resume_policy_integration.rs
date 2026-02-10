use chrono::Utc;
use tak::commands::work;
use tak::model::{Contract, Kind, Planning, Status};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::repo::Repo;
use tak::store::work::WorkStore;
use tempfile::tempdir;

fn create_task(repo_root: &std::path::Path, title: &str, depends_on: Vec<u64>) -> u64 {
    let repo = Repo::open(repo_root).unwrap();
    let task = repo
        .store
        .create(
            title.to_string(),
            Kind::Task,
            None,
            None,
            depends_on,
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();
    repo.index.upsert(&task).unwrap();
    task.id
}

fn mutate_status(
    repo_root: &std::path::Path,
    task_id: u64,
    status: Status,
    assignee: Option<&str>,
) {
    let repo = Repo::open(repo_root).unwrap();
    let mut task = repo.store.read(task_id).unwrap();
    task.status = status;
    task.assignee = assignee.map(ToString::to_string);
    task.updated_at = Utc::now();
    repo.store.write(&task).unwrap();
    repo.index.upsert(&task).unwrap();
}

#[test]
fn work_handoff_gate_skips_immediate_reclaim_once_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let handed_off_task_id = create_task(dir.path(), "handoff-ready", vec![]);
    let _other_task_id = create_task(dir.path(), "other", vec![]);

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(handed_off_task_id);
    work_store.save(&state).unwrap();

    work::start_or_resume_with_strategy_force(
        dir.path(),
        Some("agent-1".into()),
        None,
        None,
        None,
        None,
        None,
        false,
        Format::Json,
    )
    .unwrap();

    let after_first = work_store.status("agent-1").unwrap();
    assert!(after_first.active);
    assert!(after_first.current_task_id.is_none());

    work::start_or_resume_with_strategy_force(
        dir.path(),
        Some("agent-1".into()),
        None,
        None,
        None,
        None,
        None,
        false,
        Format::Json,
    )
    .unwrap();

    let after_second = work_store.status("agent-1").unwrap();
    assert_eq!(after_second.current_task_id, Some(handed_off_task_id));
}

#[test]
fn work_blocked_gate_waits_until_dependency_changes_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let dep_id = create_task(dir.path(), "dep", vec![]);
    let blocked_id = create_task(dir.path(), "blocked", vec![dep_id]);

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(blocked_id);
    work_store.save(&state).unwrap();

    work::start_or_resume_with_strategy_force(
        dir.path(),
        Some("agent-1".into()),
        None,
        None,
        None,
        None,
        None,
        false,
        Format::Json,
    )
    .unwrap();

    let after_first = work_store.status("agent-1").unwrap();
    assert!(after_first.active);
    assert!(after_first.current_task_id.is_none());

    mutate_status(dir.path(), dep_id, Status::Done, Some("agent-dep"));

    work::start_or_resume_with_strategy_force(
        dir.path(),
        Some("agent-1".into()),
        None,
        None,
        None,
        None,
        None,
        false,
        Format::Json,
    )
    .unwrap();

    let after_second = work_store.status("agent-1").unwrap();
    assert_eq!(after_second.current_task_id, Some(blocked_id));
}

#[test]
fn work_force_reclaim_bypasses_gate_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_task(dir.path(), "handoff-ready", vec![]);

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(task_id);
    work_store.save(&state).unwrap();

    work::start_or_resume_with_strategy_force(
        dir.path(),
        Some("agent-1".into()),
        None,
        None,
        None,
        None,
        None,
        true,
        Format::Json,
    )
    .unwrap();

    let after = work_store.status("agent-1").unwrap();
    assert_eq!(after.current_task_id, Some(task_id));
}
