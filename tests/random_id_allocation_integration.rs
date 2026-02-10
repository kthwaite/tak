use std::collections::HashSet;

use tempfile::tempdir;

use tak::model::{Contract, Kind, Planning};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::repo::Repo;
use tak::task_id::TaskId;

#[test]
fn create_command_uses_random_hex_ids_and_preserves_graph_links() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    FileStore::init(repo_root).unwrap();

    tak::commands::create::run(
        repo_root,
        "Epic".into(),
        Kind::Epic,
        None,
        None,
        vec![],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(repo_root).unwrap();
    let epic = repo
        .store
        .list_all()
        .unwrap()
        .into_iter()
        .find(|task| task.title == "Epic")
        .expect("epic should exist");

    tak::commands::create::run(
        repo_root,
        "Child".into(),
        Kind::Task,
        None,
        Some(epic.id),
        vec![],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();

    tak::commands::create::run(
        repo_root,
        "Blocked".into(),
        Kind::Task,
        None,
        None,
        vec![epic.id],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();

    let repo = Repo::open(repo_root).unwrap();
    let tasks = repo.store.list_all().unwrap();
    assert_eq!(tasks.len(), 3);

    let epic = tasks.iter().find(|task| task.title == "Epic").unwrap();
    let child = tasks.iter().find(|task| task.title == "Child").unwrap();
    let blocked = tasks.iter().find(|task| task.title == "Blocked").unwrap();

    assert_eq!(child.parent, Some(epic.id));
    assert_eq!(blocked.depends_on.len(), 1);
    assert_eq!(blocked.depends_on[0].id, epic.id);

    let mut seen = HashSet::new();
    for task in &tasks {
        assert!(seen.insert(task.id), "duplicate id generated: {}", task.id);

        let canonical = TaskId::from(task.id);
        assert_eq!(canonical.as_str().len(), TaskId::HEX_LEN);
        assert!(canonical.as_str().bytes().all(|b| b.is_ascii_hexdigit()));

        let canonical_path = repo_root
            .join(".tak")
            .join("tasks")
            .join(format!("{canonical}.json"));
        assert!(canonical_path.exists(), "missing canonical task file");

        let legacy_path = repo_root
            .join(".tak")
            .join("tasks")
            .join(format!("{}.json", task.id));
        assert!(
            !legacy_path.exists(),
            "legacy numeric filename should not be created"
        );
    }

    assert!(!repo_root.join(".tak/counter.json").exists());
    assert!(!repo_root.join(".tak/counter.lock").exists());
    assert!(repo_root.join(".tak/task-id.lock").exists());

    let available = repo.index.available(None).unwrap();
    assert!(available.contains(&TaskId::from(epic.id)));
    assert!(available.contains(&TaskId::from(child.id)));
    assert!(!available.contains(&TaskId::from(blocked.id)));
}

#[test]
fn filestore_bulk_create_keeps_ids_unique_and_counterless() {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let mut seen = HashSet::new();
    for index in 0..64 {
        let task = store
            .create(
                format!("Task {index}"),
                Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        assert!(seen.insert(task.id), "duplicate id generated: {}", task.id);

        let canonical = TaskId::from(task.id);
        let canonical_path = store.root().join("tasks").join(format!("{canonical}.json"));
        assert!(
            canonical_path.exists(),
            "missing canonical file for task {index}"
        );
    }

    assert_eq!(seen.len(), 64);
    assert_eq!(store.list_all().unwrap().len(), 64);
    assert!(!store.root().join("counter.json").exists());
    assert!(!store.root().join("counter.lock").exists());
    assert!(store.root().join("task-id.lock").exists());
}
