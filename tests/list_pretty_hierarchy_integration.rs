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

#[test]
fn pretty_list_groups_parent_with_descendants_without_reordering_json() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak(repo_root, &["--format", "json", "init"]);

    let root_a = parse_json(&run_tak(
        repo_root,
        &["--format", "json", "create", "Root A", "--kind", "task"],
    ));
    let root_a_hex = root_a["id"]
        .as_str()
        .expect("root A id should be canonical hex")
        .to_string();

    run_tak(
        repo_root,
        &["--format", "json", "create", "Root B", "--kind", "task"],
    );

    run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "create",
            "Child A1",
            "--kind",
            "task",
            "--parent",
            root_a_hex.as_str(),
        ],
    );

    let pretty = run_tak(repo_root, &["--format", "pretty", "list"]);
    let idx_root_a = pretty
        .find("Root A")
        .expect("Root A should appear in pretty output");
    let idx_child = pretty
        .find("Child A1")
        .expect("Child A1 should appear in pretty output");
    let idx_root_b = pretty
        .find("Root B")
        .expect("Root B should appear in pretty output");

    assert!(
        idx_root_a < idx_child && idx_child < idx_root_b,
        "pretty output should keep Root A and Child A1 contiguous before Root B\n{pretty}"
    );

    let json_list = parse_json(&run_tak(repo_root, &["--format", "json", "list"]));
    let titles: Vec<&str> = json_list
        .as_array()
        .expect("list json output should be array")
        .iter()
        .map(|task| task["title"].as_str().expect("title should be string"))
        .collect();

    assert_eq!(titles, vec!["Root A", "Root B", "Child A1"]);
}
