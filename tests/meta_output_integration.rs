use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn id_hex(id: u64) -> String {
    format!("{id:016x}")
}

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
fn meta_tasks_render_across_create_show_list_and_tree_formats() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    run_tak(repo_root, &["--format", "json", "init"]);

    let create_json = run_tak(
        repo_root,
        &["--format", "json", "create", "Meta Root", "--kind", "meta"],
    );
    let meta_root = parse_json(&create_json);
    assert_eq!(meta_root["kind"], "meta");

    let root_id = meta_root["id"].as_u64().expect("id should be numeric");
    let root_hex = id_hex(root_id);

    let create_pretty = run_tak(
        repo_root,
        &[
            "--format",
            "pretty",
            "create",
            "Meta Pretty",
            "--kind",
            "meta",
        ],
    );
    assert!(create_pretty.contains("Meta Pretty"));
    assert!(create_pretty.contains("meta"));

    let create_minimal = run_tak(
        repo_root,
        &[
            "--format",
            "minimal",
            "create",
            "Meta Mini",
            "--kind",
            "meta",
            "--parent",
            root_hex.as_str(),
        ],
    );
    let minimal_create_id = create_minimal
        .split_whitespace()
        .next()
        .expect("minimal create should include id");
    assert_eq!(minimal_create_id.len(), 16);
    assert!(minimal_create_id.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(create_minimal.contains("meta"));

    run_tak(
        repo_root,
        &[
            "--format",
            "json",
            "create",
            "Plain Child",
            "--kind",
            "task",
            "--parent",
            root_hex.as_str(),
        ],
    );

    let show_json = run_tak(repo_root, &["--format", "json", "show", root_hex.as_str()]);
    let shown = parse_json(&show_json);
    assert_eq!(shown["id"].as_u64(), Some(root_id));
    assert_eq!(shown["title"], "Meta Root");
    assert_eq!(shown["kind"], "meta");

    let show_pretty = run_tak(
        repo_root,
        &["--format", "pretty", "show", root_hex.as_str()],
    );
    assert!(show_pretty.contains("Meta Root"));
    assert!(show_pretty.contains("meta"));

    let show_minimal = run_tak(
        repo_root,
        &["--format", "minimal", "show", root_hex.as_str()],
    );
    assert!(show_minimal.contains(&root_hex));
    assert!(show_minimal.contains("meta"));
    assert!(show_minimal.contains("pending"));

    let list_json = run_tak(repo_root, &["--format", "json", "list", "--kind", "meta"]);
    let listed = parse_json(&list_json);
    let listed_array = listed.as_array().expect("list output should be an array");
    assert!(listed_array.len() >= 3);
    assert!(listed_array.iter().all(|task| task["kind"] == "meta"));
    assert!(listed_array.iter().any(|task| task["title"] == "Meta Root"));

    let list_pretty = run_tak(repo_root, &["--format", "pretty", "list", "--kind", "meta"]);
    assert!(list_pretty.contains("Meta Root"));
    assert!(list_pretty.contains("Meta Pretty"));
    assert!(list_pretty.contains("meta"));
    assert!(!list_pretty.contains("Plain Child"));

    let list_minimal = run_tak(
        repo_root,
        &["--format", "minimal", "list", "--kind", "meta"],
    );
    assert!(list_minimal.contains("ID"));
    assert!(list_minimal.contains("Meta Root"));
    assert!(list_minimal.contains("meta"));
    assert!(!list_minimal.contains("Plain Child"));

    let tree_json = run_tak(repo_root, &["--format", "json", "tree", root_hex.as_str()]);
    let tree = parse_json(&tree_json);
    assert_eq!(tree["id"], root_hex);
    assert_eq!(tree["title"], "Meta Root");
    assert_eq!(tree["kind"], "meta");

    let children = tree["children"]
        .as_array()
        .expect("tree children should be an array");
    assert!(
        children
            .iter()
            .any(|child| child["title"] == "Meta Mini" && child["kind"] == "meta")
    );
    assert!(
        children
            .iter()
            .any(|child| child["title"] == "Plain Child" && child["kind"] == "task")
    );

    let tree_pretty = run_tak(
        repo_root,
        &["--format", "pretty", "tree", root_hex.as_str()],
    );
    assert!(tree_pretty.contains("Meta Root"));
    assert!(tree_pretty.contains("(meta,"));
    assert!(tree_pretty.contains("Meta Mini"));
    assert!(tree_pretty.contains("Plain Child"));

    let tree_minimal = run_tak(
        repo_root,
        &["--format", "minimal", "tree", root_hex.as_str()],
    );
    assert!(tree_minimal.contains(&root_hex));
    assert!(tree_minimal.contains("meta"));
    assert!(tree_minimal.contains("Plain Child"));
}
