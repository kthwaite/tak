use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run_tak(repo_root: &Path, args: &[&str]) -> Output {
    let binary = assert_cmd::cargo::cargo_bin!("tak");
    let mut cmd = Command::new(binary);
    cmd.current_dir(repo_root).env("NO_COLOR", "1").args(args);
    cmd.output().expect("tak command executes")
}

fn run_tak_ok(repo_root: &Path, args: &[&str]) -> Output {
    let output = run_tak(repo_root, args);
    assert!(
        output.status.success(),
        "tak {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_tak_json(repo_root: &Path, args: &[&str]) -> Value {
    let output = run_tak_ok(repo_root, args);
    serde_json::from_slice(&output.stdout).expect("valid json stdout")
}

fn hex_id(id: u64) -> String {
    format!("{id:016x}")
}

#[test]
fn pretty_log_and_verify_messages_render_canonical_hex_ids() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["--format", "json", "init"]);

    let created = run_tak_json(
        repo_root,
        &["--format", "json", "create", "No-op verify task"],
    );
    let task_hex = created
        .get("id")
        .and_then(Value::as_str)
        .expect("task id in create output")
        .to_string();
    let task_decimal = u64::from_str_radix(&task_hex, 16)
        .expect("hex id should parse")
        .to_string();

    let log_output = run_tak_ok(
        repo_root,
        &["--format", "pretty", "log", task_decimal.as_str()],
    );
    let log_stderr = String::from_utf8_lossy(&log_output.stderr);
    assert!(
        log_stderr.contains(&format!("No history for task {task_hex}")),
        "stderr should contain canonical id, got: {log_stderr}"
    );
    assert!(
        !log_stderr.contains(&format!("No history for task {task_decimal}")),
        "stderr should not contain decimal id, got: {log_stderr}"
    );

    let verify_output = run_tak_ok(
        repo_root,
        &["--format", "pretty", "verify", task_decimal.as_str()],
    );
    let verify_stderr = String::from_utf8_lossy(&verify_output.stderr);
    assert!(
        verify_stderr.contains(&format!("No verification commands for task {task_hex}")),
        "stderr should contain canonical id, got: {verify_stderr}"
    );
    assert!(
        !verify_stderr.contains(&format!("No verification commands for task {task_decimal}")),
        "stderr should not contain decimal id, got: {verify_stderr}"
    );
}

#[test]
fn doctor_data_integrity_messages_use_canonical_hex_ids() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["--format", "json", "init"]);

    let created = run_tak_json(
        repo_root,
        &["--format", "json", "create", "Doctor target task"],
    );
    let task_hex = created
        .get("id")
        .and_then(Value::as_str)
        .expect("task id in create output")
        .to_string();

    let task_path = repo_root
        .join(".tak")
        .join("tasks")
        .join(format!("{task_hex}.json"));
    let mut task_json: Value =
        serde_json::from_str(&fs::read_to_string(&task_path).expect("task file exists"))
            .expect("task json parse");

    task_json["parent"] = Value::from(999_u64);
    task_json["depends_on"] = serde_json::json!([{"id": 1000_u64}]);

    fs::write(
        &task_path,
        serde_json::to_string_pretty(&task_json).expect("serialize task json"),
    )
    .unwrap();

    let doctor = run_tak_json(repo_root, &["--format", "json", "doctor"]);
    let checks = doctor
        .get("checks")
        .and_then(Value::as_array)
        .expect("doctor checks array");

    let messages: Vec<&str> = checks
        .iter()
        .filter_map(|check| check.get("message").and_then(Value::as_str))
        .collect();

    let parent_msg = format!("task {}: parent {} not found", task_hex, hex_id(999));
    let dep_msg = format!("task {}: depends on {}, not found", task_hex, hex_id(1000));

    assert!(messages.iter().any(|msg| *msg == parent_msg));
    assert!(messages.iter().any(|msg| *msg == dep_msg));
}
