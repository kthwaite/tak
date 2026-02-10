use std::path::Path;
use std::process::{Command, Output};

use chrono::{Duration, Utc};
use rusqlite::params;
use serde_json::Value;
use tak::store::coordination_db::CoordinationDb;
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

fn create_task(repo_root: &Path, title: &str) -> String {
    let created = run_tak_json(repo_root, &["create", title]);
    created
        .get("id")
        .and_then(Value::as_str)
        .expect("task id")
        .to_string()
}

#[test]
fn blackboard_list_supports_from_and_recent_filters_with_status_tag_task() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    let task_id = create_task(repo_root, "coordination filter task");

    run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "alice",
            "--message",
            "old status",
            "--tag",
            "coordination",
            "--task",
            task_id.as_str(),
        ],
    );
    run_tak_json(
        repo_root,
        &[
            "blackboard",
            "post",
            "--from",
            "bob",
            "--message",
            "fresh status",
            "--tag",
            "coordination",
            "--task",
            task_id.as_str(),
        ],
    );

    let db = CoordinationDb::from_repo(repo_root).unwrap();
    let old_ts = (Utc::now() - Duration::minutes(10)).to_rfc3339();
    db.conn()
        .execute(
            "UPDATE notes SET created_at = ?1, updated_at = ?1 WHERE id = 1",
            params![old_ts],
        )
        .unwrap();

    let by_author = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "list",
            "--status",
            "open",
            "--tag",
            "coordination",
            "--task",
            task_id.as_str(),
            "--from",
            "alice",
        ],
    );
    let by_author_rows = by_author.as_array().expect("array output");
    assert_eq!(by_author_rows.len(), 1);
    assert_eq!(
        by_author_rows[0].get("from_agent").and_then(Value::as_str),
        Some("alice")
    );

    let recent_author = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "list",
            "--status",
            "open",
            "--tag",
            "coordination",
            "--task",
            task_id.as_str(),
            "--from",
            "alice",
            "--recent-secs",
            "60",
        ],
    );
    let recent_author_rows = recent_author.as_array().expect("array output");
    assert!(recent_author_rows.is_empty());

    let recent_any = run_tak_json(
        repo_root,
        &[
            "blackboard",
            "list",
            "--status",
            "open",
            "--tag",
            "coordination",
            "--task",
            task_id.as_str(),
            "--recent-secs",
            "60",
        ],
    );
    let recent_any_rows = recent_any.as_array().expect("array output");
    assert_eq!(recent_any_rows.len(), 1);
    assert_eq!(
        recent_any_rows[0].get("from_agent").and_then(Value::as_str),
        Some("bob")
    );
}

#[test]
fn mesh_inbox_filters_do_not_change_ack_semantics() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);

    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "alice", "--session-id", "sid-a"],
    );
    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "bob", "--session-id", "sid-b"],
    );
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "join",
            "--name",
            "helper",
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
            "alice",
            "--to",
            "helper",
            "--message",
            "old ping",
        ],
    );
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "send",
            "--from",
            "bob",
            "--to",
            "helper",
            "--message",
            "fresh ping",
        ],
    );

    let db = CoordinationDb::from_repo(repo_root).unwrap();
    let old_ts = (Utc::now() - Duration::minutes(10)).to_rfc3339();
    db.conn()
        .execute(
            "UPDATE messages SET created_at = ?1 WHERE to_agent = 'helper' AND from_agent = 'alice'",
            params![old_ts],
        )
        .unwrap();

    let by_sender = run_tak_json(
        repo_root,
        &["mesh", "inbox", "--name", "helper", "--from", "alice"],
    );
    let by_sender_rows = by_sender.as_array().expect("array output");
    assert_eq!(by_sender_rows.len(), 1);
    assert_eq!(
        by_sender_rows[0].get("from_agent").and_then(Value::as_str),
        Some("alice")
    );

    let recent_sender = run_tak_json(
        repo_root,
        &[
            "mesh",
            "inbox",
            "--name",
            "helper",
            "--from",
            "alice",
            "--recent-secs",
            "60",
        ],
    );
    let recent_sender_rows = recent_sender.as_array().expect("array output");
    assert!(recent_sender_rows.is_empty());

    run_tak_json(
        repo_root,
        &[
            "mesh", "inbox", "--name", "helper", "--from", "alice", "--ack",
        ],
    );

    let after_ack = run_tak_json(repo_root, &["mesh", "inbox", "--name", "helper"]);
    let after_ack_rows = after_ack.as_array().expect("array output");
    assert!(after_ack_rows.is_empty());
}
