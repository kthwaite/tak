use crate::model::Task;

pub fn print_task(task: &Task, pretty: bool) {
    if pretty {
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
    } else {
        println!("{}", serde_json::to_string(&task).unwrap());
    }
}

pub fn print_tasks(tasks: &[Task], pretty: bool) {
    if pretty {
        for task in tasks {
            print_task(task, true);
            println!();
        }
    } else {
        println!("{}", serde_json::to_string(tasks).unwrap());
    }
}
