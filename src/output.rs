use crate::error::Result;
use crate::model::Task;
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Pretty,
    Minimal,
}

pub fn print_task(task: &Task, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(&task)?),
        Format::Pretty => {
            println!("[{}] {} ({})", task.id, task.title, task.status);
            if let Some(ref desc) = task.description {
                println!("  {}", desc);
            }
            println!("  kind: {} | status: {}", task.kind, task.status);
            if let Some(parent) = task.parent {
                println!("  parent: {}", parent);
            }
            if !task.depends_on.is_empty() {
                let dep_strs: Vec<String> = task
                    .depends_on
                    .iter()
                    .map(|d| match (&d.dep_type, &d.reason) {
                        (None, None) => format!("{}", d.id),
                        (Some(t), None) => format!("{} ({})", d.id, t),
                        (None, Some(r)) => format!("{} [{}]", d.id, r),
                        (Some(t), Some(r)) => format!("{} ({}) [{}]", d.id, t, r),
                    })
                    .collect();
                println!("  depends on: {}", dep_strs.join(", "));
            }
            if let Some(ref assignee) = task.assignee {
                println!("  assignee: {}", assignee);
            }
            if !task.tags.is_empty() {
                println!("  tags: {}", task.tags.join(", "));
            }
        }
        Format::Minimal => {
            let assignee = task.assignee.as_deref().unwrap_or("-");
            let title = truncate_title(&task.title, 12);
            println!(
                "{:>4} {:12} {:6} {:10} {}",
                task.id, title, task.kind, task.status, assignee
            );
        }
    }
    Ok(())
}

pub fn truncate_title(title: &str, max_len: usize) -> String {
    if title.chars().count() > max_len {
        let truncated: String = title.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    } else {
        title.to_string()
    }
}

pub fn print_tasks(tasks: &[Task], format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(tasks)?),
        Format::Pretty => {
            for task in tasks {
                print_task(task, Format::Pretty)?;
                println!();
            }
        }
        Format::Minimal => {
            println!(
                "{:>4} {:12} {:6} {:10} ASSIGNEE",
                "ID", "TITLE", "KIND", "STATUS"
            );
            println!("{}", "-".repeat(50));
            for task in tasks {
                let assignee = task.assignee.as_deref().unwrap_or("-");
                let title = truncate_title(&task.title, 12);
                println!(
                    "{:>4} {:12} {:6} {:10} {}",
                    task.id, title, task.kind, task.status, assignee
                );
            }
        }
    }
    Ok(())
}
