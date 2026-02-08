use std::path::Path;

use colored::Colorize;

use crate::error::Result;
use crate::output::Format;
use crate::store::repo::Repo;

/// Display the history log for a task.
///
/// - `tak log ID` -- print history to stdout
pub fn run(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    // Verify task exists
    let _ = repo.store.read(id)?;

    let events = repo.sidecars.read_history(id)?;

    if events.is_empty() {
        match format {
            Format::Json => {
                println!("[]");
            }
            _ => {
                eprintln!("No history for task {id}");
            }
        }
        return Ok(());
    }

    match format {
        Format::Json => {
            println!("{}", serde_json::to_string(&events).unwrap());
        }
        _ => {
            for evt in &events {
                let ts = evt.timestamp.format("%Y-%m-%d %H:%M:%S");
                let agent_part = match &evt.agent {
                    Some(a) => format!(" ({})", a.cyan()),
                    None => String::new(),
                };
                println!("  {} {}{}", ts.to_string().dimmed(), evt.event, agent_part);
            }
        }
    }
    Ok(())
}
