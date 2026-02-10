use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn run_tak(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_tak"))
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "tak {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn parse_json(output: &str) -> Value {
    serde_json::from_str(output.trim()).expect("output should be valid json")
}

fn list_titles_for_kind(repo_root: &Path, kind: &str) -> Vec<String> {
    let listed = run_tak(repo_root, &["--format", "json", "list", "--kind", kind]);
    parse_json(&listed)
        .as_array()
        .expect("kind-filtered list output should be an array")
        .iter()
        .map(|task| {
            task["title"]
                .as_str()
                .expect("title should be present")
                .to_string()
        })
        .collect()
}

#[test]
fn legacy_kind_filters_and_defaults_remain_stable_with_meta_present() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak(repo_root, &["--format", "json", "init"]);

    let default_task = parse_json(&run_tak(
        repo_root,
        &["--format", "json", "create", "Default Task"],
    ));
    assert_eq!(default_task["kind"], "task");

    parse_json(&run_tak(
        repo_root,
        &["--format", "json", "create", "Epic One", "--kind", "epic"],
    ));
    parse_json(&run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "create",
            "Feature One",
            "--kind",
            "feature",
        ],
    ));
    parse_json(&run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "create",
            "Task Explicit",
            "--kind",
            "task",
        ],
    ));
    parse_json(&run_tak(
        repo_root,
        &["--format", "json", "create", "Bug One", "--kind", "bug"],
    ));
    parse_json(&run_tak(
        repo_root,
        &["--format", "json", "create", "Meta One", "--kind", "meta"],
    ));

    let all_tasks = parse_json(&run_tak(repo_root, &["--format", "json", "list"]));
    let all_tasks = all_tasks
        .as_array()
        .expect("default list output should be an array");

    assert_eq!(all_tasks.len(), 6);

    let mut epic_count = 0;
    let mut feature_count = 0;
    let mut task_count = 0;
    let mut bug_count = 0;
    let mut meta_count = 0;

    for task in all_tasks {
        match task["kind"].as_str().expect("kind should be present") {
            "epic" => epic_count += 1,
            "feature" => feature_count += 1,
            "task" => task_count += 1,
            "bug" => bug_count += 1,
            "meta" => meta_count += 1,
            other => panic!("unexpected kind in default list: {other}"),
        }
    }

    assert_eq!(epic_count, 1);
    assert_eq!(feature_count, 1);
    assert_eq!(task_count, 2);
    assert_eq!(bug_count, 1);
    assert_eq!(meta_count, 1);

    let epic_titles = list_titles_for_kind(repo_root, "epic");
    assert_eq!(epic_titles, vec!["Epic One".to_string()]);

    let feature_titles = list_titles_for_kind(repo_root, "feature");
    assert_eq!(feature_titles, vec!["Feature One".to_string()]);

    let task_titles = list_titles_for_kind(repo_root, "task");
    assert_eq!(task_titles.len(), 2);
    assert!(task_titles.contains(&"Default Task".to_string()));
    assert!(task_titles.contains(&"Task Explicit".to_string()));

    let bug_titles = list_titles_for_kind(repo_root, "bug");
    assert_eq!(bug_titles, vec!["Bug One".to_string()]);

    let meta_titles = list_titles_for_kind(repo_root, "meta");
    assert_eq!(meta_titles, vec!["Meta One".to_string()]);
}
