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

pub fn print_task(task: &Task, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(&task)?),
        Format::Pretty => {
            println!(
                "{} {} ({})",
                format!("[{}]", task.id).cyan().bold(),
                task.title.bold(),
                style_status(task.status),
            );
            if let Some(ref desc) = task.description {
                println!("  {}", desc);
            }
            println!(
                "  {} {} {} {}",
                "kind:".dimmed(),
                task.kind,
                "status:".dimmed(),
                style_status(task.status),
            );
            if let Some(parent) = task.parent {
                println!("  {} {}", "parent:".dimmed(), parent);
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
                println!("  {} {}", "depends on:".dimmed(), dep_strs.join(", "));
            }
            if let Some(ref assignee) = task.assignee {
                println!("  {} {}", "assignee:".dimmed(), assignee.cyan());
            }
            if !task.tags.is_empty() {
                let colored_tags: Vec<String> =
                    task.tags.iter().map(|t| t.cyan().to_string()).collect();
                println!("  {} {}", "tags:".dimmed(), colored_tags.join(", "));
            }
            if !task.planning.is_empty() {
                if let Some(ref p) = task.planning.priority {
                    println!("  {} {}", "priority:".dimmed(), style_priority(p));
                }
                if let Some(ref e) = task.planning.estimate {
                    println!("  {} {}", "estimate:".dimmed(), e);
                }
                if let Some(ref r) = task.planning.risk {
                    println!("  {} {}", "risk:".dimmed(), style_risk(r));
                }
                if !task.planning.required_skills.is_empty() {
                    println!(
                        "  {} {}",
                        "skills:".dimmed(),
                        task.planning.required_skills.join(", ")
                    );
                }
            }
            if !task.git.is_empty() {
                if let Some(ref branch) = task.git.branch {
                    println!("  {} {}", "branch:".dimmed(), branch.green());
                }
                if let Some(ref sha) = task.git.start_commit {
                    println!(
                        "  {} {}",
                        "start:".dimmed(),
                        &sha[..7.min(sha.len())].yellow()
                    );
                }
                if let Some(ref sha) = task.git.end_commit {
                    println!(
                        "  {} {}",
                        "end:".dimmed(),
                        &sha[..7.min(sha.len())].yellow()
                    );
                }
                if !task.git.commits.is_empty() {
                    println!("  {}", "commits:".dimmed());
                    for c in &task.git.commits {
                        println!("    {}", c.dimmed());
                    }
                }
                if let Some(ref pr) = task.git.pr {
                    println!("  {} {}", "pr:".dimmed(), pr.cyan());
                }
            }
            if !task.execution.is_empty() {
                if task.execution.attempt_count > 0 {
                    println!(
                        "  {} {}",
                        "attempts:".dimmed(),
                        task.execution.attempt_count
                    );
                }
                if let Some(ref err) = task.execution.last_error {
                    println!("  {} {}", "last error:".dimmed(), err.red());
                }
                if let Some(ref summary) = task.execution.handoff_summary {
                    println!("  {} {}", "handoff:".dimmed(), summary);
                }
                if let Some(ref reason) = task.execution.blocked_reason {
                    println!("  {} {}", "blocked reason:".dimmed(), reason.red());
                }
            }
            if !task.contract.is_empty() {
                if let Some(ref obj) = task.contract.objective {
                    println!("  {} {}", "objective:".dimmed(), obj);
                }
                if !task.contract.acceptance_criteria.is_empty() {
                    println!("  {}", "acceptance criteria:".dimmed());
                    for ac in &task.contract.acceptance_criteria {
                        println!("    {} {}", "-".dimmed(), ac);
                    }
                }
                if !task.contract.verification.is_empty() {
                    println!("  {}", "verification:".dimmed());
                    for v in &task.contract.verification {
                        println!("    {} {}", "$".dimmed(), v.cyan());
                    }
                }
                if !task.contract.constraints.is_empty() {
                    println!("  {}", "constraints:".dimmed());
                    for c in &task.contract.constraints {
                        println!("    {} {}", "-".dimmed(), c);
                    }
                }
            }
            if !task.learnings.is_empty() {
                let ids: Vec<String> = task
                    .learnings
                    .iter()
                    .map(|id| format!("L{id}").magenta().to_string())
                    .collect();
                println!("  {} {}", "learnings:".dimmed(), ids.join(", "));
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
