use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn run_tak_cli(repo_root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tak-cli"))
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("tak-cli command should run")
}

fn run_success(repo_root: &Path, args: &[&str]) -> String {
    let output = run_tak_cli(repo_root, args);
    assert!(
        output.status.success(),
        "tak-cli {:?} failed\nstdout:\n{}\nstderr:\n{}",
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
fn json_create_show_and_list_roundtrip() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_success(repo_root, &["--format", "json", "init"]);

    let created = parse_json(&run_success(
        repo_root,
        &["--format", "json", "create", "Parity Smoke Task"],
    ));
    let id = created["id"]
        .as_str()
        .expect("id should be canonical hex")
        .to_string();
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|ch| ch.is_ascii_hexdigit()));

    let shown = parse_json(&run_success(
        repo_root,
        &["--format", "json", "show", id.as_str()],
    ));
    assert_eq!(shown["id"], id);
    assert_eq!(shown["title"], "Parity Smoke Task");
    assert_eq!(shown["status"], "pending");

    let listed = parse_json(&run_success(repo_root, &["--format", "json", "list"]));
    let tasks = listed.as_array().expect("list output should be array");
    assert!(tasks.iter().any(|task| task["id"] == id));
    assert!(
        tasks
            .iter()
            .any(|task| task["title"] == "Parity Smoke Task")
    );
}

#[test]
fn json_error_envelope_is_emitted_on_failures() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_success(repo_root, &["--format", "json", "init"]);

    let output = run_tak_cli(repo_root, &["--format", "json", "show", "ffffffffffffffff"]);
    assert!(
        !output.status.success(),
        "show should fail for missing task"
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    let envelope = parse_json(&stderr);

    assert!(
        envelope["error"].is_string(),
        "expected structured error code in stderr"
    );
    assert!(
        envelope["message"].is_string(),
        "expected structured error message in stderr"
    );
}
