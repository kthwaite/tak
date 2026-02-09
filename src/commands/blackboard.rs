use std::path::Path;

use colored::Colorize;

use crate::error::Result;
use crate::output::Format;
use crate::store::blackboard::{BlackboardNote, BlackboardStatus, BlackboardStore};
use crate::store::repo::Repo;

pub fn post(
    repo_root: &Path,
    from: &str,
    message: &str,
    tags: Vec<String>,
    task_ids: Vec<u64>,
    format: Format,
) -> Result<()> {
    if !task_ids.is_empty() {
        let repo = Repo::open(repo_root)?;
        for &id in &task_ids {
            repo.store.read(id)?;
        }
    }

    let store = BlackboardStore::open(&repo_root.join(".tak"));
    let note = store.post(from, message, tags, task_ids)?;
    print_note(&note, format)?;
    Ok(())
}

pub fn list(
    repo_root: &Path,
    status: Option<BlackboardStatus>,
    tag: Option<String>,
    task_id: Option<u64>,
    limit: Option<usize>,
    format: Format,
) -> Result<()> {
    let store = BlackboardStore::open(&repo_root.join(".tak"));
    let notes = store.list(status, tag.as_deref(), task_id, limit)?;
    print_notes(&notes, format)?;
    Ok(())
}

pub fn show(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let store = BlackboardStore::open(&repo_root.join(".tak"));
    let note = store.get(id)?;
    print_note(&note, format)?;
    Ok(())
}

pub fn close(
    repo_root: &Path,
    id: u64,
    by: &str,
    reason: Option<&str>,
    format: Format,
) -> Result<()> {
    let store = BlackboardStore::open(&repo_root.join(".tak"));
    let note = store.close(id, by, reason)?;
    print_note(&note, format)?;
    Ok(())
}

pub fn reopen(repo_root: &Path, id: u64, by: &str, format: Format) -> Result<()> {
    let store = BlackboardStore::open(&repo_root.join(".tak"));
    let note = store.reopen(id, by)?;
    print_note(&note, format)?;
    Ok(())
}

fn print_note(note: &BlackboardNote, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(note)?),
        Format::Pretty => {
            let status = style_status(note.status);
            println!(
                "{} {} {}",
                format!("[B{}]", note.id).magenta().bold(),
                status,
                format!("by {}", note.author).dimmed(),
            );
            println!("  {}", note.message);
            if !note.tags.is_empty() {
                println!("  {} {}", "tags:".dimmed(), note.tags.join(", ").cyan());
            }
            if !note.task_ids.is_empty() {
                let task_ids = note
                    .task_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  {} {}", "tasks:".dimmed(), task_ids);
            }
            println!(
                "  {} {}",
                "updated:".dimmed(),
                note.updated_at.to_rfc3339().dimmed()
            );
            if note.status == BlackboardStatus::Closed {
                if let Some(by) = note.closed_by.as_deref() {
                    println!("  {} {}", "closed by:".dimmed(), by);
                }
                if let Some(reason) = note.closed_reason.as_deref() {
                    println!("  {} {}", "reason:".dimmed(), reason);
                }
            }
        }
        Format::Minimal => {
            println!("{}", note.id);
        }
    }
    Ok(())
}

fn print_notes(notes: &[BlackboardNote], format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(notes)?),
        Format::Pretty => {
            if notes.is_empty() {
                println!("{}", "No blackboard notes.".dimmed());
            } else {
                for note in notes {
                    let status = style_status(note.status);
                    println!(
                        "{} {} {} {}",
                        format!("[B{}]", note.id).magenta().bold(),
                        status,
                        format!("{}:", note.author).cyan(),
                        note.message,
                    );
                    if !note.tags.is_empty() {
                        println!("  {} {}", "tags:".dimmed(), note.tags.join(", ").cyan());
                    }
                }
            }
        }
        Format::Minimal => {
            for note in notes {
                println!("{} {} {}", note.id, note.status, note.author);
            }
        }
    }
    Ok(())
}

fn style_status(status: BlackboardStatus) -> String {
    match status {
        BlackboardStatus::Open => "open".yellow().to_string(),
        BlackboardStatus::Closed => "closed".green().to_string(),
    }
}
