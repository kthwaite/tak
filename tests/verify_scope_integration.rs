use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::tempdir;

fn run_tak(repo_root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tak"))
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("tak command should run")
}

fn run_tak_with_env(repo_root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tak"));
    command
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("tak command should run")
}

fn assert_success(output: &Output, args: &[&str]) {
    assert!(
        output.status.success(),
        "tak {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should contain JSON")
}

fn stderr_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).expect("stderr should contain JSON")
}

#[test]
fn verify_scope_blocks_then_unblocks_and_preserves_sidecar_contract() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path();

    let init_args = ["--format", "json", "init"];
    let init_output = run_tak(repo_root, &init_args);
    assert_success(&init_output, &init_args);

    let create_args = [
        "--format",
        "json",
        "create",
        "Scoped verify target",
        "--verify",
        "true",
    ];
    let create_output = run_tak(repo_root, &create_args);
    assert_success(&create_output, &create_args);
    let created = stdout_json(&create_output);
    let task_id = created["id"]
        .as_str()
        .expect("create output should include string id")
        .to_string();

    let join_owner_args = [
        "--format",
        "json",
        "mesh",
        "join",
        "--name",
        "owner-agent",
        "--session-id",
        "sid-owner",
    ];
    let join_owner = run_tak(repo_root, &join_owner_args);
    assert_success(&join_owner, &join_owner_args);

    let join_peer_args = [
        "--format",
        "json",
        "mesh",
        "join",
        "--name",
        "peer-agent",
        "--session-id",
        "sid-peer",
    ];
    let join_peer = run_tak(repo_root, &join_peer_args);
    assert_success(&join_peer, &join_peer_args);

    let reserve_args = [
        "--format",
        "json",
        "mesh",
        "reserve",
        "--name",
        "peer-agent",
        "--path",
        "src/store",
        "--reason",
        "peer-work",
    ];
    let reserve_output = run_tak(repo_root, &reserve_args);
    assert_success(&reserve_output, &reserve_args);

    let verify_args = [
        "--format",
        "json",
        "verify",
        task_id.as_str(),
        "--path",
        "src/store/mesh.rs",
    ];
    let blocked = run_tak_with_env(repo_root, &verify_args, &[("TAK_AGENT", "owner-agent")]);
    assert!(
        !blocked.status.success(),
        "scoped verify should block on overlap"
    );

    let blocked_err = stderr_json(&blocked);
    assert_eq!(blocked_err["error"], "verify_scope_blocked");
    let blocked_message = blocked_err["message"]
        .as_str()
        .expect("error message should be a string");
    assert!(blocked_message.contains("peer-agent"));
    assert!(blocked_message.contains("src/store"));
    assert!(blocked_message.contains("peer-work"));
    assert!(blocked_message.contains("tak mesh blockers --path src/store"));
    assert!(blocked_message.contains("tak wait --path src/store --timeout 120"));
    assert!(
        blocked_message.contains("tak mesh reserve --name owner-agent --path src/store/mesh.rs")
    );

    let verify_sidecar_path = repo_root
        .join(".tak")
        .join("verification_results")
        .join(format!("{task_id}.json"));
    assert!(
        !verify_sidecar_path.exists(),
        "blocked scoped verify should not persist verification results"
    );

    let release_args = [
        "--format",
        "json",
        "mesh",
        "release",
        "--name",
        "peer-agent",
        "--all",
    ];
    let release_output = run_tak(repo_root, &release_args);
    assert_success(&release_output, &release_args);

    let unblocked = run_tak_with_env(repo_root, &verify_args, &[("TAK_AGENT", "owner-agent")]);
    assert_success(&unblocked, &verify_args);

    let unblocked_payload = stdout_json(&unblocked);
    assert_eq!(unblocked_payload["passed"], json!(true));
    assert_eq!(
        unblocked_payload["scope"]["selector"],
        json!("explicit_paths")
    );
    assert_eq!(unblocked_payload["scope"]["blocked"], json!(false));
    assert_eq!(
        unblocked_payload["scope"]["effective_paths"],
        json!(["src/store/mesh.rs"])
    );

    assert!(verify_sidecar_path.exists());
    let sidecar_json: Value =
        serde_json::from_str(&fs::read_to_string(&verify_sidecar_path).unwrap())
            .expect("verification sidecar should be valid json");
    assert_eq!(sidecar_json["passed"], json!(true));
    assert_eq!(sidecar_json["results"].as_array().unwrap().len(), 1);
    assert!(
        sidecar_json.get("scope").is_none(),
        "verification sidecar contract should remain unchanged"
    );
}
