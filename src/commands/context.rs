use std::path::Path;

use crate::error::Result;
use crate::json_ids::format_task_id;
use crate::output::Format;
use crate::store::repo::Repo;

pub fn run(
    repo_root: &Path,
    id: u64,
    set: Option<String>,
    clear: bool,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    // Verify task exists
    let _ = repo.store.read(id)?;
    let task_id = format_task_id(id);

    if clear {
        repo.sidecars.delete_context(id)?;
        match format {
            Format::Json => {
                println!(
                    "{}",
                    serde_json::json!({"id": task_id.clone(), "context": null})
                );
            }
            _ => eprintln!("Context cleared for task {task_id}"),
        }
        return Ok(());
    }

    if let Some(text) = set {
        repo.sidecars.write_context(id, &text)?;
        match format {
            Format::Json => {
                println!(
                    "{}",
                    serde_json::json!({"id": task_id.clone(), "context": text})
                );
            }
            _ => eprintln!("Context set for task {task_id}"),
        }
        return Ok(());
    }

    // Read mode
    match repo.sidecars.read_context(id)? {
        Some(text) => match format {
            Format::Json => {
                println!(
                    "{}",
                    serde_json::json!({"id": task_id.clone(), "context": text})
                );
            }
            _ => print!("{text}"),
        },
        None => match format {
            Format::Json => {
                println!("{}", serde_json::json!({"id": task_id, "context": null}));
            }
            _ => {
                eprintln!("No context notes for task {task_id}");
            }
        },
    }
    Ok(())
}
