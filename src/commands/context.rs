use std::path::Path;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::repo::Repo;

/// Read or write context notes for a task.
///
/// - `tak context ID` — print context to stdout
/// - `tak context ID --set TEXT` — overwrite context
/// - `tak context ID --clear` — delete context file
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

    if clear {
        repo.sidecars.delete_context(id)?;
        match format {
            Format::Json => {
                println!("{}", serde_json::json!({"id": id, "context": null}));
            }
            _ => eprintln!("Context cleared for task {id}"),
        }
        return Ok(());
    }

    if let Some(text) = set {
        repo.sidecars.write_context(id, &text)?;
        match format {
            Format::Json => {
                println!("{}", serde_json::json!({"id": id, "context": text}));
            }
            _ => eprintln!("Context set for task {id}"),
        }
        return Ok(());
    }

    // Read mode
    match repo.sidecars.read_context(id)? {
        Some(text) => match format {
            Format::Json => {
                println!("{}", serde_json::json!({"id": id, "context": text}));
            }
            _ => print!("{text}"),
        },
        None => match format {
            Format::Json => {
                println!("{}", serde_json::json!({"id": id, "context": null}));
            }
            _ => {
                return Err(TakError::NoContext(id));
            }
        },
    }
    Ok(())
}
