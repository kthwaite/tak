use std::path::Path;

use tak::commands::create;
use tak::model::{Contract, Kind, Planning, Task};
use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::repo::Repo;
use tempfile::tempdir;

fn task_by_title(repo_root: &Path, title: &str) -> Task {
    let repo = Repo::open(repo_root).unwrap();
    repo.store
        .list_all()
        .unwrap()
        .into_iter()
        .find(|task| task.title == title)
        .unwrap_or_else(|| panic!("task with title '{title}' not found"))
}

#[test]
fn meta_task_captures_origin_idea_from_dependency() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    create::run(
        dir.path(),
        "Idea intake".into(),
        Kind::Idea,
        None,
        None,
        vec![],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let idea = task_by_title(dir.path(), "Idea intake");

    create::run(
        dir.path(),
        "Meta refinement".into(),
        Kind::Meta,
        None,
        None,
        vec![idea.id],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let meta = task_by_title(dir.path(), "Meta refinement");

    assert_eq!(meta.origin_idea_id(), Some(idea.id));
    assert!(meta.refinement_task_ids().is_empty());
}

#[test]
fn promoted_execution_tasks_capture_and_inherit_traceability_links() {
    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    create::run(
        dir.path(),
        "Idea intake".into(),
        Kind::Idea,
        None,
        None,
        vec![],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let idea = task_by_title(dir.path(), "Idea intake");

    create::run(
        dir.path(),
        "Meta refinement".into(),
        Kind::Meta,
        None,
        None,
        vec![idea.id],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let meta = task_by_title(dir.path(), "Meta refinement");

    create::run(
        dir.path(),
        "Promoted feature".into(),
        Kind::Feature,
        None,
        None,
        vec![meta.id],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let feature = task_by_title(dir.path(), "Promoted feature");

    assert_eq!(feature.origin_idea_id(), Some(idea.id));
    assert_eq!(feature.refinement_task_ids(), vec![meta.id]);

    let feature_json = serde_json::to_value(&feature).unwrap();
    assert_eq!(feature_json["origin_idea_id"], idea.id);
    assert_eq!(
        feature_json["refinement_task_ids"],
        serde_json::json!([meta.id])
    );

    create::run(
        dir.path(),
        "Promoted child task".into(),
        Kind::Task,
        None,
        Some(feature.id),
        vec![],
        vec![],
        Contract::default(),
        Planning::default(),
        Format::Json,
    )
    .unwrap();
    let child = task_by_title(dir.path(), "Promoted child task");

    assert_eq!(child.origin_idea_id(), Some(idea.id));
    assert_eq!(child.refinement_task_ids(), vec![meta.id]);
}
