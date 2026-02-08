use std::path::Path;
use std::process::Command;

use chrono::Utc;

use crate::error::Result;
use crate::output::Format;
use crate::store::repo::Repo;
use crate::store::sidecars::{CommandResult, VerificationResult};

/// Run the verification commands from a task's contract.
///
/// Each command is executed via `sh -c`. Reports pass/fail per command.
/// Returns exit code 0 if all pass, 1 if any fail.
/// Stores the result in `.tak/verification_results/{id}.json`.
pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(id)?;

    let commands = &task.contract.verification;

    if commands.is_empty() {
        let vr = VerificationResult {
            timestamp: Utc::now(),
            results: vec![],
            passed: true,
        };
        let _ = repo.sidecars.write_verification(id, &vr);

        match format {
            Format::Json => {
                println!("{}", serde_json::to_string(&vr).unwrap());
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

        let (passed, exit_code, stdout, stderr) = match output {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                (o.status.success(), code, out, err)
            }
            Err(e) => (false, -1, String::new(), e.to_string()),
        };

        if !passed {
            all_passed = false;
        }

        match format {
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
            Format::Json => {}
        }

        results.push(CommandResult {
            command: cmd.clone(),
            exit_code,
            stdout,
            stderr,
            passed,
        });
    }

    let vr = VerificationResult {
        timestamp: Utc::now(),
        results,
        passed: all_passed,
    };

    // Store the result
    let _ = repo.sidecars.write_verification(id, &vr);

    match format {
        Format::Json => {
            println!("{}", serde_json::to_string(&vr).unwrap());
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
