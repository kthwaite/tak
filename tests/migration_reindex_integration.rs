use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::Value;
use tempfile::tempdir;

use tak::commands;
use tak::model::{Contract, Kind, LearningCategory, Planning, Status};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::learnings::LearningStore;
use tak::store::repo::Repo;
use tak::task_id::TaskId;

#[derive(Clone, Copy)]
struct FixtureIds {
    root: u64,
    dep_done: u64,
    child: u64,
    downstream: u64,
    learning: u64,
}

fn tid(id: u64) -> TaskId {
    TaskId::from(id)
}

fn create_task(store: &FileStore, title: &str, parent: Option<u64>, depends_on: Vec<u64>) -> u64 {
    store
        .create(
            title.into(),
            Kind::Task,
            None,
            parent,
            depends_on,
            vec![],
            Contract::default(),
            Planning::default(),
        )
        .unwrap()
        .id
}

fn canonical_task_path(store: &FileStore, id: u64) -> PathBuf {
    store.root().join("tasks").join(format!("{}.json", tid(id)))
}

fn legacy_task_path(store: &FileStore, id: u64) -> PathBuf {
    store.root().join("tasks").join(format!("{id}.json"))
}

fn write_legacy_sidecars(tak_root: &Path, task_id: u64) {
    fs::create_dir_all(tak_root.join("context")).unwrap();
    fs::create_dir_all(tak_root.join("history")).unwrap();
    fs::create_dir_all(tak_root.join("verification_results")).unwrap();
    fs::create_dir_all(tak_root.join("artifacts").join(task_id.to_string())).unwrap();

    fs::write(
        tak_root.join("context").join(format!("{task_id}.md")),
        "legacy context",
    )
    .unwrap();
    fs::write(
        tak_root.join("history").join(format!("{task_id}.jsonl")),
        "{\"event\":\"legacy\"}\n",
    )
    .unwrap();
    fs::write(
        tak_root
            .join("verification_results")
            .join(format!("{task_id}.json")),
        "{\"passed\":true}",
    )
    .unwrap();
    fs::write(
        tak_root
            .join("artifacts")
            .join(task_id.to_string())
            .join("artifact.txt"),
        "legacy artifact",
    )
    .unwrap();
}

fn setup_legacy_fixture() -> (tempfile::TempDir, FileStore, FixtureIds) {
    let dir = tempdir().unwrap();
    let store = FileStore::init(dir.path()).unwrap();

    let root = create_task(&store, "root", None, vec![]);
    let dep_done = create_task(&store, "dep-done", None, vec![]);
    let child = create_task(&store, "child", Some(root), vec![dep_done]);
    let downstream = create_task(&store, "downstream", None, vec![child]);

    let mut dep_task = store.read(dep_done).unwrap();
    dep_task.status = Status::Done;
    dep_task.updated_at = Utc::now();
    store.write(&dep_task).unwrap();

    let learning_store = LearningStore::open(store.root());
    let learning = learning_store
        .create(
            "Graph semantics".into(),
            Some("Learning linked to migrated tasks".into()),
            LearningCategory::Insight,
            vec!["migration".into()],
            vec![child],
        )
        .unwrap();

    write_legacy_sidecars(store.root(), child);

    for id in [root, dep_done, child, downstream] {
        fs::rename(
            canonical_task_path(&store, id),
            legacy_task_path(&store, id),
        )
        .unwrap();
    }

    (
        dir,
        store,
        FixtureIds {
            root,
            dep_done,
            child,
            downstream,
            learning: learning.id,
        },
    )
}

fn assert_graph_queries(repo: &Repo, ids: FixtureIds) {
    assert_eq!(
        repo.index.available(None).unwrap(),
        vec![tid(ids.root), tid(ids.child)]
    );
    assert_eq!(repo.index.blocked().unwrap(), vec![tid(ids.downstream)]);
    assert_eq!(
        repo.index.children_of(ids.root).unwrap(),
        vec![tid(ids.child)]
    );
    assert_eq!(
        repo.index.dependents_of(ids.child).unwrap(),
        vec![tid(ids.downstream)]
    );
    assert_eq!(
        repo.index.learnings_for_task(ids.child).unwrap(),
        vec![ids.learning]
    );
    assert!(repo
        .index
        .learnings_for_task(ids.dep_done)
        .unwrap()
        .is_empty());
}

#[test]
fn migrate_ids_apply_preserves_graph_semantics() {
    let (dir, store, ids) = setup_legacy_fixture();

    // Baseline semantics on legacy numeric filenames.
    let before = Repo::open(dir.path()).unwrap();
    assert_graph_queries(&before, ids);

    commands::migrate_ids::run(dir.path(), false, false, Format::Json).unwrap();

    let config: Value =
        serde_json::from_str(&fs::read_to_string(store.root().join("config.json")).unwrap())
            .unwrap();
    assert_eq!(config["version"], serde_json::json!(3));

    for id in [ids.root, ids.dep_done, ids.child, ids.downstream] {
        assert!(!legacy_task_path(&store, id).exists());
        assert!(canonical_task_path(&store, id).exists());
    }

    let legacy_context = store
        .root()
        .join("context")
        .join(format!("{}.md", ids.child));
    let legacy_history = store
        .root()
        .join("history")
        .join(format!("{}.jsonl", ids.child));
    let legacy_verification = store
        .root()
        .join("verification_results")
        .join(format!("{}.json", ids.child));
    let legacy_artifacts = store.root().join("artifacts").join(ids.child.to_string());

    let canonical_context = store
        .root()
        .join("context")
        .join(format!("{}.md", tid(ids.child)));
    let canonical_history = store
        .root()
        .join("history")
        .join(format!("{}.jsonl", tid(ids.child)));
    let canonical_verification = store
        .root()
        .join("verification_results")
        .join(format!("{}.json", tid(ids.child)));
    let canonical_artifacts = store.root().join("artifacts").join(tid(ids.child).as_str());

    assert!(!legacy_context.exists());
    assert!(!legacy_history.exists());
    assert!(!legacy_verification.exists());
    assert!(!legacy_artifacts.exists());

    assert!(canonical_context.exists());
    assert!(canonical_history.exists());
    assert!(canonical_verification.exists());
    assert!(canonical_artifacts.join("artifact.txt").exists());

    let mut audit_files = fs::read_dir(store.root().join("migrations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    audit_files.sort();
    assert_eq!(audit_files.len(), 1);

    let audit: Value = serde_json::from_str(&fs::read_to_string(&audit_files[0]).unwrap()).unwrap();
    assert_eq!(audit["id_map"].as_array().unwrap().len(), 4);
    assert_eq!(audit["config_version_after"], serde_json::json!(3));

    let after = Repo::open(dir.path()).unwrap();
    assert_graph_queries(&after, ids);
}

#[test]
fn reindex_command_rebuilds_migrated_repository_index() {
    let (dir, store, ids) = setup_legacy_fixture();

    commands::migrate_ids::run(dir.path(), false, false, Format::Json).unwrap();

    fs::remove_file(store.root().join("index.db")).unwrap();
    assert!(!store.root().join("index.db").exists());

    commands::reindex::run(dir.path()).unwrap();
    assert!(store.root().join("index.db").exists());

    let repo = Repo::open(dir.path()).unwrap();
    assert_graph_queries(&repo, ids);
}
