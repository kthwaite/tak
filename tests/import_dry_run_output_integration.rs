use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
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
fn import_dry_run_outputs_include_metadata_and_stable_ordering() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak(repo_root, &["--format", "json", "init"]);

    let plan_path = repo_root.join("plan.yaml");
    fs::write(
        &plan_path,
        r#"
epic: Dry-run Preview
tags: [import]
priority: high
estimate: l
features:
  - alias: infra
    title: Infrastructure
    tags: [backend]
    priority: medium
    tasks:
      - alias: schemas
        title: Define schemas
        tags: [schema]
        priority: low
        estimate: s
        required_skills: [serde]
        objective: Schema objective
        depends_on: [infra]
"#,
    )
    .unwrap();

    let json_output = run_tak(
        repo_root,
        &["--format", "json", "import", "plan.yaml", "--dry-run"],
    );
    let report = parse_json(&json_output);

    assert_eq!(report["dry_run"], true);
    assert_eq!(report["source"], "plan.yaml");

    let tasks = report["tasks"]
        .as_array()
        .expect("tasks should be an array");
    assert_eq!(tasks.len(), 3);

    let titles: Vec<&str> = tasks
        .iter()
        .map(|task| task["title"].as_str().expect("title should be a string"))
        .collect();
    assert_eq!(
        titles,
        vec!["Dry-run Preview", "Infrastructure", "Define schemas"]
    );

    let epic = &tasks[0];
    assert_eq!(epic["depth"], 0);
    assert_eq!(epic["priority"], "high");
    assert_eq!(epic["estimate"], "l");
    assert_eq!(epic["tags"], json!(["import"]));

    let feature = &tasks[1];
    assert_eq!(feature["depth"], 1);
    assert_eq!(feature["alias"], "infra");
    assert_eq!(feature["priority"], "medium");
    assert_eq!(feature["tags"], json!(["backend"]));

    let leaf = &tasks[2];
    assert_eq!(leaf["depth"], 2);
    assert_eq!(leaf["alias"], "schemas");
    assert_eq!(leaf["depends_on"], json!(["@infra"]));
    assert_eq!(leaf["priority"], "low");
    assert_eq!(leaf["estimate"], "s");
    assert_eq!(leaf["required_skills"], json!(["serde"]));
    assert_eq!(leaf["objective"], "Schema objective");

    let pretty_output = run_tak(
        repo_root,
        &["--format", "pretty", "import", "plan.yaml", "--dry-run"],
    );
    assert!(pretty_output.contains("Dry run: validated 3 tasks from plan.yaml"));
    assert!(pretty_output.contains("[epic] Dry-run Preview"));
    assert!(pretty_output.contains("p=high"));
    assert!(pretty_output.contains("est=l"));
    assert!(pretty_output.contains("[task] Define schemas"));
    assert!(pretty_output.contains("deps=@infra"));
    assert!(pretty_output.contains("objective=Schema objective"));

    let minimal_output = run_tak(
        repo_root,
        &["--format", "minimal", "import", "plan.yaml", "--dry-run"],
    );
    let mut minimal_lines = minimal_output.lines();
    assert_eq!(minimal_lines.next(), Some("dry-run 3 plan.yaml"));
    assert!(minimal_output.contains("epic"));
    assert!(minimal_output.contains("Define schemas [p=low est=s tags=1]"));

    let json_output_again = run_tak(
        repo_root,
        &["--format", "json", "import", "plan.yaml", "--dry-run"],
    );
    assert_eq!(json_output.trim(), json_output_again.trim());
}
