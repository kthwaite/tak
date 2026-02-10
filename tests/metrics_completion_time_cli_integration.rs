use std::path::Path;
use std::process::Command;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tak::model::{Contract, Kind, Planning, Status};
use tak::store::coordination::CoordinationLinks;
use tak::store::files::FileStore;
use tak::store::sidecars::{HistoryEvent, SidecarStore};
use tempfile::tempdir;

fn run_tak_raw(repo_root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tak"))
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("command should run")
}

fn run_tak(repo_root: &Path, args: &[&str]) -> String {
    let output = run_tak_raw(repo_root, args);

    assert!(
        output.status.success(),
        "tak {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn run_tak_error_json(repo_root: &Path, args: &[&str]) -> Value {
    let output = run_tak_raw(repo_root, args);

    assert!(
        !output.status.success(),
        "tak {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    parse_json(&stderr)
}

fn parse_json(output: &str) -> Value {
    serde_json::from_str(output.trim()).expect("output should be valid json")
}

fn ts(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn append_event(sidecars: &SidecarStore, task_id: u64, event: &str, timestamp: DateTime<Utc>) {
    let entry = HistoryEvent {
        id: None,
        timestamp,
        event: event.to_string(),
        agent: Some("tester".into()),
        detail: serde_json::Map::new(),
        links: CoordinationLinks::default(),
    };
    sidecars.append_history(task_id, &entry).unwrap();
}

fn seed_done_task_with_history(repo_root: &Path) -> u64 {
    let store = FileStore::init(repo_root).unwrap();
    let sidecars = SidecarStore::open(&repo_root.join(".tak"));

    let mut task = store
        .create(
            "Deterministic completion sample".into(),
            Kind::Task,
            None,
            None,
            vec![],
            vec!["metrics".into()],
            Contract::default(),
            Planning::default(),
        )
        .unwrap();

    task.status = Status::Done;
    task.created_at = ts("2026-02-01T00:00:00Z");
    task.updated_at = ts("2026-02-05T00:00:00Z");
    store.write(&task).unwrap();

    append_event(&sidecars, task.id, "started", ts("2026-02-03T00:00:00Z"));
    append_event(&sidecars, task.id, "finished", ts("2026-02-05T00:00:00Z"));

    task.id
}

fn approx_eq(left: f64, right: f64) {
    assert!(
        (left - right).abs() < 1e-6,
        "expected {left} ~= {right} (diff={})",
        (left - right).abs()
    );
}

#[test]
fn metrics_completion_time_cycle_json_contract() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let output = run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
            "--metric",
            "cycle",
            "--tag",
            "metrics",
        ],
    );

    let report = parse_json(&output);

    assert_eq!(report["metric"], "cycle");
    assert_eq!(report["bucket"], "day");
    assert_eq!(report["window"]["from"], "2026-02-01");
    assert_eq!(report["window"]["to"], "2026-02-06");

    let series = report["series"]
        .as_array()
        .expect("series should be an array");
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["bucket"], "2026-02-05");
    assert_eq!(series[0]["samples"], 1);

    approx_eq(
        series[0]["avg_hours"]
            .as_f64()
            .expect("avg_hours should be a number"),
        48.0,
    );
    approx_eq(
        report["summary"]["avg_hours"]
            .as_f64()
            .expect("summary avg_hours should be a number"),
        48.0,
    );
    assert_eq!(report["summary"]["samples"], 1);
}

#[test]
fn metrics_completion_time_lead_uses_created_timestamp() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let output = run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
            "--metric",
            "lead",
        ],
    );

    let report = parse_json(&output);
    assert_eq!(report["metric"], "lead");

    // lead time = finished_at - created_at = 4 days = 96h
    approx_eq(
        report["summary"]["avg_hours"]
            .as_f64()
            .expect("summary avg_hours should be a number"),
        96.0,
    );
    assert_eq!(report["summary"]["samples"], 1);
}

#[test]
fn metrics_completion_time_pretty_and_minimal_are_human_readable() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let pretty = run_tak(
        repo_root,
        &[
            "--format",
            "pretty",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
            "--metric",
            "cycle",
        ],
    );
    assert!(pretty.contains("Completion-time metrics"));
    assert!(pretty.contains("BUCKET"));
    assert!(!pretty.trim_start().starts_with('{'));

    let minimal = run_tak(
        repo_root,
        &[
            "--format",
            "minimal",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
            "--metric",
            "cycle",
        ],
    );
    assert!(minimal.contains("completion-time metric=cycle"));
    assert!(!minimal.trim_start().starts_with('{'));
}

#[test]
fn metrics_burndown_pretty_and_minimal_are_human_readable() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let pretty = run_tak(
        repo_root,
        &[
            "--format",
            "pretty",
            "metrics",
            "burndown",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
        ],
    );
    assert!(pretty.contains("Burndown metrics"));
    assert!(pretty.contains("DATE"));
    assert!(!pretty.trim_start().starts_with('{'));

    let minimal = run_tak(
        repo_root,
        &[
            "--format",
            "minimal",
            "metrics",
            "burndown",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--bucket",
            "day",
        ],
    );
    assert!(minimal.contains("burndown bucket=day"));
    assert!(!minimal.trim_start().starts_with('{'));
}

#[test]
fn metrics_burndown_rejects_inverted_window() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let error = run_tak_error_json(
        repo_root,
        &[
            "--format",
            "json",
            "metrics",
            "burndown",
            "--from",
            "2026-02-10",
            "--to",
            "2026-02-01",
        ],
    );

    assert_eq!(error["error"], "metrics_invalid_query");
    assert!(
        error["message"]
            .as_str()
            .expect("message should be string")
            .contains("inverted")
    );
}

#[test]
fn metrics_completion_time_rejects_include_cancelled_flag() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let error = run_tak_error_json(
        repo_root,
        &[
            "--format",
            "json",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-06",
            "--include-cancelled",
        ],
    );

    assert_eq!(error["error"], "metrics_invalid_query");
    assert!(
        error["message"]
            .as_str()
            .expect("message should be string")
            .contains("include-cancelled")
    );
}

#[test]
fn metrics_burndown_rejects_excessive_day_window() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();
    seed_done_task_with_history(repo_root);

    let error = run_tak_error_json(
        repo_root,
        &[
            "--format",
            "json",
            "metrics",
            "burndown",
            "--from",
            "2024-01-01",
            "--to",
            "2025-12-31",
            "--bucket",
            "day",
        ],
    );

    assert_eq!(error["error"], "metrics_invalid_query");
    assert!(
        error["message"]
            .as_str()
            .expect("message should be string")
            .contains("too large")
    );
}
