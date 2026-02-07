use crate::error::Result;
use crate::model::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Pretty,
    Minimal,
}

impl Format {
    pub fn from_str_with_flag(format: &str, pretty_flag: bool) -> Self {
        if pretty_flag {
            return Self::Pretty;
        }
        match format {
            "pretty" => Self::Pretty,
            "minimal" => Self::Minimal,
            _ => Self::Json,
        }
    }
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
                println!("  depends on: {:?}", task.depends_on);
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
            println!("{:>4} {:12} {:6} {:10} {}", task.id, title, task.kind, task.status, assignee);
        }
    }
    Ok(())
}

fn truncate_title(title: &str, max_len: usize) -> String {
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
            println!("{:>4} {:12} {:6} {:10} {}", "ID", "TITLE", "KIND", "STATUS", "ASSIGNEE");
            println!("{}", "-".repeat(50));
            for task in tasks {
                let assignee = task.assignee.as_deref().unwrap_or("-");
                let title = truncate_title(&task.title, 12);
                println!("{:>4} {:12} {:6} {:10} {}", task.id, title, task.kind, task.status, assignee);
            }
        }
    }
    Ok(())
}
