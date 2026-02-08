use std::path::Path;
use std::process::Command;

use crate::error::Result;
use crate::output::Format;
use crate::store::repo::Repo;

/// Run the verification commands from a task's contract.
///
/// Each command is executed via `sh -c`. Reports pass/fail per command.
/// Returns exit code 0 if all pass, 1 if any fail.
pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(id)?;

    let commands = &task.contract.verification;

    if commands.is_empty() {
        match format {
            Format::Json => {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": id,
                        "results": [],
                        "all_passed": true,
                    })
                );
            }
            _ => eprintln!("No verification commands for task {id}"),
        }
        return Ok(());
    }

    let mut results = Vec::new();
    let mut all_passed = true;

    for cmd in commands {
        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(repo_root)
            .output();

        let (passed, exit_code, stderr) = match output {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                (o.status.success(), code, err)
            }
            Err(e) => (false, -1, e.to_string()),
        };

        if !passed {
            all_passed = false;
        }

        match format {
            Format::Json => {
                results.push(serde_json::json!({
                    "command": cmd,
                    "passed": passed,
                    "exit_code": exit_code,
                    "stderr": if stderr.is_empty() { None } else { Some(&stderr) },
                }));
            }
            Format::Pretty => {
                let icon = if passed { "PASS" } else { "FAIL" };
                println!("  [{icon}] $ {cmd}");
                if !stderr.is_empty() {
                    for line in stderr.lines() {
                        println!("         {line}");
                    }
                }
            }
            Format::Minimal => {
                let icon = if passed { "ok" } else { "FAIL" };
                println!("{icon} {cmd}");
            }
        }
    }

    match format {
        Format::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "id": id,
                    "results": results,
                    "all_passed": all_passed,
                })
            );
        }
        Format::Pretty => {
            if all_passed {
                println!("  All verification commands passed.");
            } else {
                println!("  Some verification commands failed.");
            }
        }
        Format::Minimal => {}
    }

    if !all_passed {
        std::process::exit(1);
    }

    Ok(())
}
