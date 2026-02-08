use std::path::Path;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::repo::Repo;

/// Display the history log for a task.
///
/// - `tak log ID` — print history to stdout
pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    // Verify task exists
    let _ = repo.store.read(id)?;

    match repo.sidecars.read_history(id)? {
        Some(text) => match format {
            Format::Json => {
                let entries: Vec<&str> = text.lines().collect();
                println!("{}", serde_json::json!({"id": id, "history": entries}));
            }
            _ => print!("{text}"),
        },
        None => match format {
            Format::Json => {
                let empty: Vec<String> = vec![];
                println!("{}", serde_json::json!({"id": id, "history": empty}));
            }
            _ => {
                return Err(TakError::NoHistory(id));
            }
        },
    }
    Ok(())
}
