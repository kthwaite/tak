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

#[test]
fn mesh_feed_hides_heartbeat_by_default_and_allows_explicit_opt_in() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "agent-a", "--session-id", "sid-a"],
    );
    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "agent-b", "--session-id", "sid-b"],
    );

    run_tak_ok(repo_root, &["mesh", "heartbeat", "--name", "agent-a"]);
    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "send",
            "--from",
            "agent-a",
            "--to",
            "agent-b",
            "--message",
            "ping",
        ],
    );

    let default_feed = run_tak_json(repo_root, &["mesh", "feed", "--limit", "20"]);
    let default_rows = default_feed.as_array().expect("array output");
    assert!(
        default_rows
            .iter()
            .all(|event| event.get("event_type").and_then(Value::as_str) != Some("mesh.heartbeat"))
    );
    assert!(
        default_rows
            .iter()
            .any(|event| event.get("event_type").and_then(Value::as_str) == Some("mesh.send"))
    );

    let include_heartbeat = run_tak_json(
        repo_root,
        &["mesh", "feed", "--limit", "20", "--include-heartbeat"],
    );
    let include_rows = include_heartbeat.as_array().expect("array output");
    assert!(
        include_rows
            .iter()
            .any(|event| event.get("event_type").and_then(Value::as_str) == Some("mesh.heartbeat"))
    );

    let heartbeat_only = run_tak_json(
        repo_root,
        &["mesh", "feed", "--event-type", "heartbeat", "--limit", "20"],
    );
    let heartbeat_rows = heartbeat_only.as_array().expect("array output");
    assert!(!heartbeat_rows.is_empty());
    assert!(
        heartbeat_rows
            .iter()
            .all(|event| event.get("event_type").and_then(Value::as_str) == Some("mesh.heartbeat"))
    );
}

#[test]
fn mesh_feed_event_type_and_recent_filters_compose() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak_ok(repo_root, &["init"]);
    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "agent-a", "--session-id", "sid-a"],
    );
    run_tak_ok(
        repo_root,
        &["mesh", "join", "--name", "agent-b", "--session-id", "sid-b"],
    );

    run_tak_ok(
        repo_root,
        &[
            "mesh",
            "send",
            "--from",
            "agent-a",
            "--to",
            "agent-b",
            "--message",
            "stale ping",
        ],
    );

    let db = CoordinationDb::from_repo(repo_root).unwrap();
    let old_ts = (Utc::now() - Duration::minutes(10)).to_rfc3339();
    db.conn()
        .execute(
            "UPDATE events SET created_at = ?1 WHERE event_type = 'mesh.send'",
            params![old_ts],
        )
        .unwrap();

    let recent_send = run_tak_json(
        repo_root,
        &[
            "mesh",
            "feed",
            "--event-type",
            "mesh.send",
            "--recent-secs",
            "60",
            "--include-heartbeat",
        ],
    );
    let recent_send_rows = recent_send.as_array().expect("array output");
    assert!(recent_send_rows.is_empty());

    let join_only_limited = run_tak_json(
        repo_root,
        &[
            "mesh",
            "feed",
            "--event-type",
            "mesh.join",
            "--recent-secs",
            "60",
            "--limit",
            "1",
        ],
    );
    let join_rows = join_only_limited.as_array().expect("array output");
    assert_eq!(join_rows.len(), 1);
    assert_eq!(
        join_rows[0].get("event_type").and_then(Value::as_str),
        Some("mesh.join")
    );
}
