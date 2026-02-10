use chrono::Utc;
use tak::commands::work;
use tak::model::{Contract, Kind, Planning, Status};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::mesh::MeshStore;
use tak::store::repo::Repo;
use tak::store::work::WorkStore;
use tempfile::tempdir;

fn create_task(repo_root: &std::path::Path, title: &str) -> u64 {
    let repo = Repo::open(repo_root).unwrap();
    let task = repo
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
    repo.index.upsert(&task).unwrap();
    task.id
}

fn set_task_status(
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

fn set_task_in_progress(repo_root: &std::path::Path, task_id: u64, assignee: &str) {
    set_task_status(repo_root, task_id, Status::InProgress, Some(assignee));
}

#[test]
fn work_done_finishes_current_task_and_releases_reservations_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_task(dir.path(), "work-done");
    set_task_in_progress(dir.path(), task_id, "agent-1");

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(task_id);
    work_store.save(&state).unwrap();

    let mesh = MeshStore::open(&dir.path().join(".tak"));
    mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
    mesh.reserve(
        "agent-1",
        vec!["src/commands/work.rs".into()],
        Some("work-done-integration"),
    )
    .unwrap();

    work::done(dir.path(), Some("agent-1".into()), false, Format::Json).unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert_eq!(task.status, Status::Done);

    let state = work_store.status("agent-1").unwrap();
    assert!(state.active);
    assert!(state.current_task_id.is_none());

    let reservations = mesh.list_reservations().unwrap();
    assert!(
        reservations
            .iter()
            .all(|reservation| reservation.agent != "agent-1")
    );
}

#[test]
fn work_done_pause_deactivates_loop_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_task(dir.path(), "work-done-pause");
    set_task_in_progress(dir.path(), task_id, "agent-1");

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(task_id);
    work_store.save(&state).unwrap();

    work::done(dir.path(), Some("agent-1".into()), true, Format::Json).unwrap();

    let state = work_store.status("agent-1").unwrap();
    assert!(!state.active);
    assert!(state.current_task_id.is_none());
}

#[test]
fn work_done_is_idempotent_on_repeated_invocation_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_task(dir.path(), "work-done-idempotent");
    set_task_in_progress(dir.path(), task_id, "agent-1");

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(task_id);
    work_store.save(&state).unwrap();

    work::done(dir.path(), Some("agent-1".into()), false, Format::Json).unwrap();
    work::done(dir.path(), Some("agent-1".into()), false, Format::Json).unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert_eq!(task.status, Status::Done);

    let state = work_store.status("agent-1").unwrap();
    assert!(state.active);
    assert!(state.current_task_id.is_none());
}

#[test]
fn work_done_detaches_stale_current_pointer_and_releases_reservations_integration() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let task_id = create_task(dir.path(), "work-done-stale-pointer");
    set_task_status(dir.path(), task_id, Status::Pending, None);

    let work_store = WorkStore::open(&dir.path().join(".tak"));
    let mut state = work_store
        .activate("agent-1", None, None, None, None, None)
        .unwrap()
        .state;
    state.current_task_id = Some(task_id);
    work_store.save(&state).unwrap();

    let mesh = MeshStore::open(&dir.path().join(".tak"));
    mesh.join(Some("agent-1"), Some("sid-1")).unwrap();
    mesh.reserve(
        "agent-1",
        vec!["src/commands/work.rs".into()],
        Some("work-done-stale-pointer"),
    )
    .unwrap();

    work::done(dir.path(), Some("agent-1".into()), false, Format::Json).unwrap();

    let repo = Repo::open(dir.path()).unwrap();
    let task = repo.store.read(task_id).unwrap();
    assert_eq!(task.status, Status::Pending);

    let state = work_store.status("agent-1").unwrap();
    assert!(state.current_task_id.is_none());

    let reservations = mesh.list_reservations().unwrap();
    assert!(
        reservations
            .iter()
            .all(|reservation| reservation.agent != "agent-1")
    );
}
