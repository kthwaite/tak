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

fn run_tak_error_json(repo_root: &Path, args: &[&str]) -> Value {
    let output = run_tak(repo_root, args);
    assert!(
        !output.status.success(),
        "expected tak {:?} to fail, but it succeeded:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stderr).expect("valid json error stderr")
}

#[test]
fn reservations_snapshot_and_overlap_wait_metadata_support_verify_guards() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "join",
            "--name",
            "owner-agent",
            "--session-id",
            "sid-owner",
        ],
    );
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "join",
            "--name",
            "helper-agent",
            "--session-id",
            "sid-helper",
        ],
    );

    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "reserve",
            "--name",
            "owner-agent",
            "--path",
            "src/store",
            "--reason",
            "task-verify-owner",
        ],
    );

    let reservations = run_tak_json(repo_root, &["mesh", "reservations"]);
    let rows = reservations.as_array().expect("array response");
    assert!(!rows.is_empty(), "expected at least one reservation row");

    let row = rows[0].as_object().expect("reservation object");
    assert!(row.contains_key("agent"));
    assert!(row.contains_key("path"));
    assert!(row.contains_key("created_at"));
    assert!(row.contains_key("expires_at"));
    assert!(row.contains_key("age_secs"));

    let filtered = run_tak_json(
        repo_root,
        &["mesh", "reservations", "--path", "src/store/mesh.rs"],
    );
    let filtered_rows = filtered.as_array().expect("filtered reservation array");
    assert_eq!(filtered_rows.len(), 1);
    assert_eq!(
        filtered_rows[0].get("path").and_then(Value::as_str),
        Some("src/store")
    );

    let wait_error = run_tak_error_json(
        repo_root,
        &["wait", "--path", "src/store/mesh.rs", "--timeout", "0"],
    );
    let message = wait_error
        .get("message")
        .and_then(Value::as_str)
        .expect("wait timeout message");
    assert!(message.contains("owner-agent"));
    assert!(message.contains("src/store"));
    assert!(message.contains("task-verify-owner"));
}

#[test]
fn inbox_feed_and_blackboard_json_match_coordination_db_field_names() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "join",
            "--name",
            "owner-agent",
            "--session-id",
            "sid-owner",
        ],
    );
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "join",
            "--name",
            "helper-agent",
            "--session-id",
            "sid-helper",
        ],
    );

    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "send",
            "--from",
            "owner-agent",
            "--to",
            "helper-agent",
            "--message",
            "hello-from-owner",
        ],
    );

    let inbox = run_tak_json(repo_root, &["mesh", "inbox", "--name", "helper-agent"]);
    let inbox_rows = inbox.as_array().expect("inbox array");
    assert_eq!(inbox_rows.len(), 1);
    let msg = inbox_rows[0].as_object().expect("inbox message object");
    assert!(msg.contains_key("from_agent"));
    assert!(msg.contains_key("to_agent"));
    assert!(msg.contains_key("created_at"));
    assert!(!msg.contains_key("from"));
    assert!(!msg.contains_key("timestamp"));

    run_tak_ok(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "owner-agent",
            "--message",
            "status update",
        ],
    );

    let notes = run_tak_json(repo_root, &["blackboard", "list", "--status", "open"]);
    let note_rows = notes.as_array().expect("blackboard array");
    assert!(!note_rows.is_empty());
    let note = note_rows[0].as_object().expect("blackboard note object");
    assert!(note.contains_key("from_agent"));
    assert!(note.contains_key("created_at"));
    assert!(note.contains_key("updated_at"));
    assert!(!note.contains_key("author"));

    let feed = run_tak_json(repo_root, &["mesh", "feed", "--limit", "20"]);
    let feed_rows = feed.as_array().expect("feed array");
    assert!(!feed_rows.is_empty());
    assert!(
        feed_rows
            .iter()
            .all(|event| event.get("event_type").is_some())
    );
    assert!(
        feed_rows
            .iter()
            .all(|event| event.get("created_at").is_some())
    );
    assert!(feed_rows.iter().all(|event| event.get("ts").is_none()));
}
