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

pub fn print_task(task: &Task, format: Format) {
    match format {
        Format::Json => println!("{}", serde_json::to_string(&task).unwrap()),
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
            let title: String = if task.title.len() > 12 {
                format!("{}...", &task.title[..9])
            } else {
                task.title.clone()
            };
            println!("{:>4} {:12} {:6} {:10} {}", task.id, title, task.kind, task.status, assignee);
        }
    }
}

pub fn print_tasks(tasks: &[Task], format: Format) {
    match format {
        Format::Json => println!("{}", serde_json::to_string(tasks).unwrap()),
        Format::Pretty => {
            for task in tasks {
                print_task(task, Format::Pretty);
                println!();
            }
        }
        Format::Minimal => {
            println!("{:>4} {:12} {:6} {:10} {}", "ID", "TITLE", "KIND", "STATUS", "ASSIGNEE");
            println!("{}", "-".repeat(50));
            for task in tasks {
                let assignee = task.assignee.as_deref().unwrap_or("-");
                let title: String = if task.title.len() > 12 {
                    format!("{}...", &task.title[..9])
                } else {
                    task.title.clone()
                };
                println!("{:>4} {:12} {:6} {:10} {}", task.id, title, task.kind, task.status, assignee);
            }
        }
    }
}
