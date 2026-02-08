use colored::Colorize;

use crate::error::Result;
use crate::model::{Priority, Risk, Status, Task};
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Pretty,
    Minimal,
}

/// Colorize a status string.
pub fn style_status(status: Status) -> String {
    let s = status.to_string();
    match status {
        Status::Pending => s.yellow().to_string(),
        Status::InProgress => s.blue().to_string(),
        Status::Done => s.green().to_string(),
        Status::Cancelled => s.red().to_string(),
    }
}

/// Colorize a priority string.
pub fn style_priority(p: &Priority) -> String {
    let s = p.to_string();
    match p {
        Priority::Critical => s.red().bold().to_string(),
        Priority::High => s.red().to_string(),
        Priority::Medium => s.yellow().to_string(),
        Priority::Low => s.green().to_string(),
    }
}

/// Colorize a risk string.
pub fn style_risk(r: &Risk) -> String {
    let s = r.to_string();
    match r {
        Risk::High => s.red().to_string(),
        Risk::Medium => s.yellow().to_string(),
        Risk::Low => s.green().to_string(),
    }
}

fn format_dependency(dep: &crate::model::Dependency) -> String {
    match (&dep.dep_type, &dep.reason) {
        (None, None) => format!("{}", dep.id),
        (Some(t), None) => format!("{} ({})", dep.id, t),
        (None, Some(r)) => format!("{} [{}]", dep.id, r),
        (Some(t), Some(r)) => format!("{} ({}) [{}]", dep.id, t, r),
    }
}

pub fn print_task(task: &Task, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(&task)?),
        Format::Pretty => print_task_pretty_table(task),
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

fn style_show_value(field: &str, raw_value: &str, padded: String) -> String {
    match field {
        "status" => match raw_value {
            "pending" => padded.yellow().to_string(),
            "in_progress" => padded.blue().to_string(),
            "done" => padded.green().to_string(),
            "cancelled" => padded.red().to_string(),
            _ => padded,
        },
        "priority" => match raw_value {
            "critical" => padded.red().bold().to_string(),
            "high" => padded.red().to_string(),
            "medium" => padded.yellow().to_string(),
            "low" => padded.green().to_string(),
            "-" => padded.dimmed().to_string(),
            _ => padded,
        },
        "risk" => match raw_value {
            "high" => padded.red().to_string(),
            "medium" => padded.yellow().to_string(),
            "low" => padded.green().to_string(),
            _ => padded,
        },
        "assignee" | "tags" | "learnings" => {
            if raw_value == "-" {
                padded.dimmed().to_string()
            } else {
                padded.cyan().to_string()
            }
        }
        "last error" | "blocked reason" => padded.red().to_string(),
        _ => padded,
    }
}

fn print_key_value_table(rows: &[(String, String)]) {
    let headers = ["FIELD", "VALUE"];
    let mut widths = [display_width(headers[0]), display_width(headers[1])];

    let normalized_rows: Vec<(String, Vec<String>)> = rows
        .iter()
        .map(|(field, value)| {
            widths[0] = widths[0].max(display_width(field));
            let lines: Vec<String> = if value.is_empty() {
                vec![String::new()]
            } else {
                value.lines().map(ToString::to_string).collect()
            };
            for line in &lines {
                widths[1] = widths[1].max(display_width(line));
            }
            (field.clone(), lines)
        })
        .collect();

    println!("{}", build_table_border('┌', '┬', '┐', &widths).dimmed());

    let header_cells = [
        format!("{:<width$}", headers[0], width = widths[0])
            .bold()
            .to_string(),
        format!("{:<width$}", headers[1], width = widths[1])
            .bold()
            .to_string(),
    ];
    print_table_row(&header_cells);

    println!("{}", build_table_border('├', '┼', '┤', &widths).dimmed());

    for (field, lines) in normalized_rows {
        for (line_idx, line) in lines.iter().enumerate() {
            let field_cell = if line_idx == 0 {
                format!("{:<width$}", field, width = widths[0])
                    .dimmed()
                    .to_string()
            } else {
                " ".repeat(widths[0])
            };

            let padded_value = format!("{:<width$}", line, width = widths[1]);
            let value_cell = style_show_value(&field, line, padded_value);

            let cells = [field_cell, value_cell];
            print_table_row(&cells);
        }
    }

    println!("{}", build_table_border('└', '┴', '┘', &widths).dimmed());
}

fn print_task_pretty_table(task: &Task) {
    let mut rows: Vec<(String, String)> = vec![
        ("id".to_string(), task.id.to_string()),
        ("title".to_string(), task.title.clone()),
        ("kind".to_string(), task.kind.to_string()),
        ("status".to_string(), task.status.to_string()),
    ];

    if let Some(ref desc) = task.description {
        rows.push(("description".to_string(), desc.clone()));
    }
    if let Some(parent) = task.parent {
        rows.push(("parent".to_string(), parent.to_string()));
    }
    if !task.depends_on.is_empty() {
        let deps = task
            .depends_on
            .iter()
            .map(format_dependency)
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(("depends on".to_string(), deps));
    }

    if let Some(ref assignee) = task.assignee {
        rows.push(("assignee".to_string(), assignee.clone()));
    } else {
        rows.push(("assignee".to_string(), "-".to_string()));
    }

    if task.tags.is_empty() {
        rows.push(("tags".to_string(), "-".to_string()));
    } else {
        rows.push(("tags".to_string(), task.tags.join(", ")));
    }

    if let Some(priority) = task.planning.priority {
        rows.push(("priority".to_string(), priority.to_string()));
    }
    if let Some(estimate) = task.planning.estimate {
        rows.push(("estimate".to_string(), estimate.to_string()));
    }
    if let Some(risk) = task.planning.risk {
        rows.push(("risk".to_string(), risk.to_string()));
    }
    if !task.planning.required_skills.is_empty() {
        rows.push((
            "skills".to_string(),
            task.planning.required_skills.join(", "),
        ));
    }

    if let Some(ref branch) = task.git.branch {
        rows.push(("branch".to_string(), branch.clone()));
    }
    if let Some(ref sha) = task.git.start_commit {
        rows.push(("start".to_string(), sha[..7.min(sha.len())].to_string()));
    }
    if let Some(ref sha) = task.git.end_commit {
        rows.push(("end".to_string(), sha[..7.min(sha.len())].to_string()));
    }
    if !task.git.commits.is_empty() {
        rows.push(("commits".to_string(), task.git.commits.join("\n")));
    }
    if let Some(ref pr) = task.git.pr {
        rows.push(("pr".to_string(), pr.clone()));
    }

    if task.execution.attempt_count > 0 {
        rows.push((
            "attempts".to_string(),
            task.execution.attempt_count.to_string(),
        ));
    }
    if let Some(ref err) = task.execution.last_error {
        rows.push(("last error".to_string(), err.clone()));
    }
    if let Some(ref summary) = task.execution.handoff_summary {
        rows.push(("handoff".to_string(), summary.clone()));
    }
    if let Some(ref reason) = task.execution.blocked_reason {
        rows.push(("blocked reason".to_string(), reason.clone()));
    }

    if let Some(ref obj) = task.contract.objective {
        rows.push(("objective".to_string(), obj.clone()));
    }
    if !task.contract.acceptance_criteria.is_empty() {
        rows.push((
            "acceptance criteria".to_string(),
            task.contract.acceptance_criteria.join("\n"),
        ));
    }
    if !task.contract.verification.is_empty() {
        rows.push((
            "verification".to_string(),
            task.contract.verification.join("\n"),
        ));
    }
    if !task.contract.constraints.is_empty() {
        rows.push((
            "constraints".to_string(),
            task.contract.constraints.join("\n"),
        ));
    }

    if task.learnings.is_empty() {
        rows.push(("learnings".to_string(), "-".to_string()));
    } else {
        let ids = task
            .learnings
            .iter()
            .map(|id| format!("L{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(("learnings".to_string(), ids));
    }

    rows.push(("created".to_string(), task.created_at.to_rfc3339()));
    rows.push(("updated".to_string(), task.updated_at.to_rfc3339()));

    print_key_value_table(&rows);
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() > max_len {
        let keep = max_len.saturating_sub(3);
        let truncated: String = text.chars().take(keep).collect();
        format!("{}...", truncated)
    } else {
        text.to_string()
    }
}

pub fn truncate_title(title: &str, max_len: usize) -> String {
    truncate_text(title, max_len)
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn build_table_border(left: char, middle: char, right: char, widths: &[usize]) -> String {
    let mut line = String::new();
    line.push(left);
    for (idx, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(*width + 2));
        if idx + 1 < widths.len() {
            line.push(middle);
        }
    }
    line.push(right);
    line
}

fn print_table_row(cells: &[String]) {
    let mut line = String::from("│");
    for cell in cells {
        line.push(' ');
        line.push_str(cell);
        line.push(' ');
        line.push('│');
    }
    println!("{}", line);
}

fn style_status_cell(status: Status, padded: String) -> String {
    match status {
        Status::Pending => padded.yellow().to_string(),
        Status::InProgress => padded.blue().to_string(),
        Status::Done => padded.green().to_string(),
        Status::Cancelled => padded.red().to_string(),
    }
}

fn style_priority_cell(priority: Option<Priority>, padded: String) -> String {
    match priority {
        Some(Priority::Critical) => padded.red().bold().to_string(),
        Some(Priority::High) => padded.red().to_string(),
        Some(Priority::Medium) => padded.yellow().to_string(),
        Some(Priority::Low) => padded.green().to_string(),
        None => padded.dimmed().to_string(),
    }
}

fn print_tasks_pretty_table(tasks: &[Task]) {
    const TITLE_MAX_WIDTH: usize = 48;
    const ASSIGNEE_MAX_WIDTH: usize = 20;
    const TAGS_MAX_WIDTH: usize = 28;

    let headers = [
        "ID", "TITLE", "KIND", "STATUS", "ASSIGNEE", "PRIORITY", "TAGS",
    ];

    let rows: Vec<[String; 7]> = tasks
        .iter()
        .map(|task| {
            let assignee =
                truncate_text(task.assignee.as_deref().unwrap_or("-"), ASSIGNEE_MAX_WIDTH);
            let priority = task
                .planning
                .priority
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let tags = if task.tags.is_empty() {
                "-".to_string()
            } else {
                task.tags.join(", ")
            };

            [
                task.id.to_string(),
                truncate_text(&task.title, TITLE_MAX_WIDTH),
                task.kind.to_string(),
                task.status.to_string(),
                assignee,
                priority,
                truncate_text(&tags, TAGS_MAX_WIDTH),
            ]
        })
        .collect();

    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(display_width(cell));
        }
    }

    println!("{}", build_table_border('┌', '┬', '┐', &widths).dimmed());

    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(idx, header)| {
            format!("{:<width$}", header, width = widths[idx])
                .bold()
                .to_string()
        })
        .collect();
    print_table_row(&header_cells);

    println!("{}", build_table_border('├', '┼', '┤', &widths).dimmed());

    for (task, row) in tasks.iter().zip(rows.iter()) {
        let id = format!("{:>width$}", &row[0], width = widths[0])
            .cyan()
            .bold()
            .to_string();
        let title = format!("{:<width$}", &row[1], width = widths[1]);
        let kind = format!("{:<width$}", &row[2], width = widths[2]);
        let status = style_status_cell(
            task.status,
            format!("{:<width$}", &row[3], width = widths[3]),
        );

        let assignee_text = format!("{:<width$}", &row[4], width = widths[4]);
        let assignee = if task.assignee.is_some() {
            assignee_text.cyan().to_string()
        } else {
            assignee_text.dimmed().to_string()
        };

        let priority = style_priority_cell(
            task.planning.priority,
            format!("{:<width$}", &row[5], width = widths[5]),
        );

        let tags_text = format!("{:<width$}", &row[6], width = widths[6]);
        let tags = if task.tags.is_empty() {
            tags_text.dimmed().to_string()
        } else {
            tags_text.cyan().to_string()
        };

        let cells = [id, title, kind, status, assignee, priority, tags];
        print_table_row(&cells);
    }

    println!("{}", build_table_border('└', '┴', '┘', &widths).dimmed());
}

pub fn print_tasks(tasks: &[Task], format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(tasks)?),
        Format::Pretty => print_tasks_pretty_table(tasks),
        Format::Minimal => {
            println!(
                "{:>4} {:12} {:6} {:10} ASSIGNEE",
                "ID".bold(),
                "TITLE".bold(),
                "KIND".bold(),
                "STATUS".bold()
            );
            println!("{}", "-".repeat(50).dimmed());
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
