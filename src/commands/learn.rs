use std::path::Path;

use chrono::Utc;

use crate::error::Result;
use crate::model::{Learning, LearningCategory};
use crate::output::Format;
use crate::store::repo::Repo;

pub fn add(
    repo_root: &Path,
    title: String,
    description: Option<String>,
    category: LearningCategory,
    tags: Vec<String>,
    task_ids: Vec<u64>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    // Validate task_ids exist
    for &tid in &task_ids {
        repo.store.read(tid)?;
    }

    let learning = repo.learnings.create(title, description, category, tags, task_ids.clone())?;
    repo.index.upsert_learning(&learning)?;

    // Update learning fingerprint
    let fp = repo.learnings.fingerprint()?;
    repo.index.set_learning_fingerprint(&fp)?;

    // Link learning to tasks (add learning ID to each task's learnings field)
    for &tid in &task_ids {
        let mut task = repo.store.read(tid)?;
        if !task.learnings.contains(&learning.id) {
            task.learnings.push(learning.id);
            task.learnings.sort();
            task.learnings.dedup();
            task.updated_at = Utc::now();
            repo.store.write(&task)?;
            repo.index.upsert(&task)?;
        }
    }

    print_learning(&learning, format)?;
    Ok(())
}

pub fn list(
    repo_root: &Path,
    category: Option<LearningCategory>,
    tag: Option<String>,
    task_id: Option<u64>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    let ids = repo.index.query_learnings(
        category.as_ref().map(|c| c.to_string()).as_deref(),
        tag.as_deref(),
        task_id,
    )?;

    let learnings: Vec<Learning> = ids
        .into_iter()
        .map(|id| repo.learnings.read(id))
        .collect::<Result<_>>()?;

    print_learnings(&learnings, format)?;
    Ok(())
}

pub fn show(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let learning = repo.learnings.read(id)?;
    print_learning(&learning, format)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn edit(
    repo_root: &Path,
    id: u64,
    title: Option<String>,
    description: Option<String>,
    category: Option<LearningCategory>,
    tags: Option<Vec<String>>,
    add_task: Vec<u64>,
    remove_task: Vec<u64>,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let mut learning = repo.learnings.read(id)?;

    // Validate new task_ids exist
    for &tid in &add_task {
        repo.store.read(tid)?;
    }

    let mut changed = false;

    if let Some(t) = title {
        learning.title = t;
        changed = true;
    }
    if let Some(d) = description {
        learning.description = Some(d);
        changed = true;
    }
    if let Some(c) = category {
        learning.category = c;
        changed = true;
    }
    if let Some(t) = tags {
        learning.tags = t;
        learning.tags.retain(|s| !s.trim().is_empty());
        learning.tags.sort();
        learning.tags.dedup();
        changed = true;
    }

    // Track old task_ids for unlinking
    let old_task_ids = learning.task_ids.clone();

    if !add_task.is_empty() {
        for tid in &add_task {
            if !learning.task_ids.contains(tid) {
                learning.task_ids.push(*tid);
            }
        }
        learning.task_ids.sort();
        learning.task_ids.dedup();
        changed = true;
    }
    if !remove_task.is_empty() {
        learning.task_ids.retain(|t| !remove_task.contains(t));
        changed = true;
    }

    if changed {
        learning.updated_at = Utc::now();
        repo.learnings.write(&learning)?;
        repo.index.upsert_learning(&learning)?;

        let fp = repo.learnings.fingerprint()?;
        repo.index.set_learning_fingerprint(&fp)?;

        // Update task linkage: add learning ID to newly linked tasks
        for &tid in &learning.task_ids {
            if !old_task_ids.contains(&tid) {
                let mut task = repo.store.read(tid)?;
                if !task.learnings.contains(&id) {
                    task.learnings.push(id);
                    task.learnings.sort();
                    task.learnings.dedup();
                    task.updated_at = Utc::now();
                    repo.store.write(&task)?;
                    repo.index.upsert(&task)?;
                }
            }
        }

        // Remove learning ID from unlinked tasks
        for &tid in &old_task_ids {
            if !learning.task_ids.contains(&tid)
                && let Ok(mut task) = repo.store.read(tid)
            {
                task.learnings.retain(|&l| l != id);
                task.updated_at = Utc::now();
                repo.store.write(&task)?;
                repo.index.upsert(&task)?;
            }
        }
    }

    print_learning(&learning, format)?;
    Ok(())
}

pub fn remove(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let learning = repo.learnings.read(id)?;

    // Remove learning ID from all linked tasks
    for &tid in &learning.task_ids {
        if let Ok(mut task) = repo.store.read(tid) {
            task.learnings.retain(|&l| l != id);
            task.updated_at = Utc::now();
            repo.store.write(&task)?;
            repo.index.upsert(&task)?;
        }
    }

    repo.index.delete_learning(id)?;
    repo.learnings.delete(id)?;

    let fp = repo.learnings.fingerprint()?;
    repo.index.set_learning_fingerprint(&fp)?;

    print_learning(&learning, format)?;
    Ok(())
}

pub fn suggest(repo_root: &Path, task_id: u64, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(task_id)?;

    let ids = repo.index.suggest_learnings(&task.title)?;

    let learnings: Vec<Learning> = ids
        .into_iter()
        .map(|id| repo.learnings.read(id))
        .collect::<Result<_>>()?;

    print_learnings(&learnings, format)?;
    Ok(())
}

// --- Output helpers ---

pub fn print_learning(learning: &Learning, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(learning)?),
        Format::Pretty => {
            println!("[L{}] {} ({})", learning.id, learning.title, learning.category);
            if let Some(ref desc) = learning.description {
                println!("  {}", desc);
            }
            if !learning.tags.is_empty() {
                println!("  tags: {}", learning.tags.join(", "));
            }
            if !learning.task_ids.is_empty() {
                let ids: Vec<String> = learning.task_ids.iter().map(|id| id.to_string()).collect();
                println!("  tasks: {}", ids.join(", "));
            }
        }
        Format::Minimal => {
            let title = crate::output::truncate_title(&learning.title, 20);
            println!(
                "L{:>3} {:20} {:8}",
                learning.id, title, learning.category
            );
        }
    }
    Ok(())
}

fn print_learnings(learnings: &[Learning], format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(learnings)?),
        Format::Pretty => {
            for learning in learnings {
                print_learning(learning, Format::Pretty)?;
                println!();
            }
        }
        Format::Minimal => {
            println!("{:>4} {:20} CATEGORY", "ID", "TITLE");
            println!("{}", "-".repeat(40));
            for learning in learnings {
                print_learning(learning, Format::Minimal)?;
            }
        }
    }
    Ok(())
}
