use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run_tak(repo_root: &Path, args: &[&str]) -> Output {
    let binary = assert_cmd::cargo::cargo_bin!("tak");
    let mut cmd = Command::new(binary);
    cmd.current_dir(repo_root);
    cmd.arg("--format").arg("json");
    cmd.args(args);
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

fn run_tak_err_json(repo_root: &Path, args: &[&str]) -> Value {
    let output = run_tak(repo_root, args);
    assert!(
        !output.status.success(),
        "expected tak {:?} to fail, but it succeeded:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_line = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(json_line).expect("valid json error line in stderr")
}

fn create_task(repo_root: &Path, title: &str) -> String {
    let task = run_tak_json(repo_root, &["create", title]);
    task.get("id")
        .and_then(Value::as_str)
        .expect("task id")
        .to_string()
}

#[test]
fn blackboard_post_template_status_sets_note_type_metadata() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);

    let posted = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Working through status lane",
            "--template",
            "status",
        ],
    );
    assert_eq!(
        posted.get("note_type").and_then(Value::as_str),
        Some("status")
    );

    let listed = run_tak_json(repo_root, &["blackboard", "list", "--status", "open"]);
    let rows = listed.as_array().expect("note list array");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("note_type").and_then(Value::as_str),
        Some("status")
    );
}

#[test]
fn blackboard_post_completion_tag_sets_note_type_metadata() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);

    let posted = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Task completed",
            "--tag",
            "completion",
        ],
    );
    assert_eq!(
        posted.get("note_type").and_then(Value::as_str),
        Some("completion")
    );
}

#[test]
fn blackboard_post_rejects_conflicting_note_type_hints() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);

    let err = run_tak_err_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Conflicting note type hints",
            "--template",
            "status",
            "--tag",
            "blocker",
        ],
    );
    assert_eq!(
        err.get("error").and_then(Value::as_str),
        Some("blackboard_invalid_message")
    );
}

#[test]
fn blackboard_status_posts_auto_supersede_latest_open_status_for_same_task() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    let task_id = create_task(repo_root, "task for status supersede");

    run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Initial status",
            "--template",
            "status",
            "--task",
            task_id.as_str(),
        ],
    );

    let second = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Follow-up status",
            "--template",
            "status",
            "--task",
            task_id.as_str(),
        ],
    );
    assert_eq!(
        second.get("supersedes_note_id").and_then(Value::as_i64),
        Some(1)
    );

    let first = run_tak_json(repo_root, &["blackboard", "show", "1"]);
    assert_eq!(first.get("status").and_then(Value::as_str), Some("closed"));
    assert_eq!(
        first.get("superseded_by_note_id").and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        first.get("closed_reason").and_then(Value::as_str),
        Some("superseded by B2")
    );
}

#[test]
fn blackboard_completion_since_note_supersedes_threadless_status_note() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);

    run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Initial status",
            "--template",
            "status",
        ],
    );

    let completion = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Done",
            "--tag",
            "completion",
            "--since-note",
            "1",
        ],
    );

    assert_eq!(
        completion.get("supersedes_note_id").and_then(Value::as_i64),
        Some(1)
    );

    let first = run_tak_json(repo_root, &["blackboard", "show", "1"]);
    assert_eq!(first.get("status").and_then(Value::as_str), Some("closed"));
    assert_eq!(
        first.get("superseded_by_note_id").and_then(Value::as_i64),
        Some(2)
    );
}
