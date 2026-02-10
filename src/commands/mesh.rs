use std::path::Path;

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::mesh::{MeshStore, Reservation};
use crate::store::paths::{normalize_reservation_path, normalized_paths_conflict};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BlockerRecord {
    owner: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    age_secs: i64,
}

fn collect_blockers(
    reservations: &[Reservation],
    normalized_targets: &[String],
) -> Vec<BlockerRecord> {
    let now = Utc::now();

    let mut blockers = reservations
        .iter()
        .flat_map(|reservation| {
            reservation.paths.iter().filter_map(|held_path| {
                let matches_targets = normalized_targets.is_empty()
                    || normalized_targets
                        .iter()
                        .any(|target| normalized_paths_conflict(target, held_path));

                if !matches_targets {
                    return None;
                }

                Some(BlockerRecord {
                    owner: reservation.agent.clone(),
                    path: held_path.clone(),
                    reason: reservation.reason.clone(),
                    age_secs: (now - reservation.since).num_seconds().max(0),
                })
            })
        })
        .collect::<Vec<_>>();

    blockers.sort_by(|a, b| {
        b.age_secs
            .cmp(&a.age_secs)
            .then_with(|| a.owner.cmp(&b.owner))
            .then_with(|| a.path.cmp(&b.path))
    });

    blockers
}

fn normalize_targets(raw_targets: Vec<String>, repo_root: &Path) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for path in raw_targets {
        let canonical = normalize_reservation_path(&path, repo_root)
            .map_err(|_| TakError::MeshInvalidPath(path.clone()))?;
        normalized.push(canonical);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

pub fn join(
    repo_root: &Path,
    name: Option<&str>,
    session_id: Option<&str>,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let reg = store.join(name, session_id)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&reg)?),
        Format::Pretty => {
            println!("Joined mesh as '{}'", reg.name.cyan().bold());
            println!("  {} {}", "session:".dimmed(), reg.session_id);
        }
        Format::Minimal => println!("{}", reg.name),
    }
    Ok(())
}

pub fn leave(repo_root: &Path, name: Option<&str>, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let left = if let Some(name) = name {
        store.leave(name)?;
        name.to_string()
    } else {
        store.leave_current()?
    };
    match format {
        Format::Json => println!("{}", serde_json::json!({"left": left})),
        Format::Pretty => println!("Left mesh: '{}'", left.cyan()),
        Format::Minimal => println!("{left}"),
    }
    Ok(())
}

pub fn list(repo_root: &Path, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let agents = store.list_agents()?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&agents)?),
        Format::Pretty => {
            if agents.is_empty() {
                println!("{}", "No agents in mesh.".dimmed());
            } else {
                for a in &agents {
                    println!(
                        "{} {}",
                        format!("[{}]", a.name).cyan().bold(),
                        format!("session={}", a.session_id).dimmed(),
                    );
                    println!("  {} {}", "cwd:".dimmed(), a.cwd);
                    println!("  {} {}", "status:".dimmed(), a.status);
                    if let Some(pid) = a.pid {
                        println!("  {} {}", "pid:".dimmed(), pid);
                    }
                    if let Some(host) = a.host.as_deref() {
                        println!("  {} {}", "host:".dimmed(), host);
                    }
                    if let Some(last_seen) = a.last_seen_at.as_ref() {
                        println!("  {} {}", "last_seen:".dimmed(), last_seen);
                    }
                }
            }
        }
        Format::Minimal => {
            for a in &agents {
                println!("{}", a.name);
            }
        }
    }
    Ok(())
}

pub fn heartbeat(
    repo_root: &Path,
    name: Option<&str>,
    session_id: Option<&str>,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let heartbeat = store.heartbeat(name, session_id)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&heartbeat)?),
        Format::Pretty => {
            println!(
                "Heartbeat refreshed for '{}'",
                heartbeat.agent.as_str().cyan().bold()
            );
            println!("  {} {}", "at:".dimmed(), heartbeat.refreshed_at);
            println!(
                "  {} {}",
                "reservations refreshed:".dimmed(),
                heartbeat.refreshed_reservations
            );
        }
        Format::Minimal => println!("{}", heartbeat.agent),
    }
    Ok(())
}

pub fn cleanup(
    repo_root: &Path,
    stale: bool,
    dry_run: bool,
    ttl_seconds: Option<u64>,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));

    let report = if stale {
        store.cleanup_stale(ttl_seconds, dry_run)?
    } else {
        store.cleanup_stale(ttl_seconds, true)?
    };

    match format {
        Format::Json => println!("{}", serde_json::to_string(&report)?),
        Format::Pretty => {
            if !stale {
                println!("{}", "No cleanup mode selected (--stale).".yellow());
                return Ok(());
            }

            let mode = if report.dry_run { "dry-run" } else { "applied" };
            println!(
                "Mesh stale cleanup ({mode}) — ttl={}s",
                report.ttl_secs.to_string().bold()
            );

            if report.candidates.is_empty() {
                println!("{}", "No stale agents found.".dimmed());
            } else {
                for candidate in &report.candidates {
                    let paths = if candidate.reservation_paths.is_empty() {
                        "-".into()
                    } else {
                        candidate.reservation_paths.join(", ")
                    };
                    println!(
                        "  {} {} ({})",
                        "-".dimmed(),
                        candidate.agent.cyan(),
                        candidate.reason
                    );
                    println!("    {} {}", "paths:".dimmed(), paths);
                    println!(
                        "    {} {}",
                        "inbox messages:".dimmed(),
                        candidate.inbox_messages
                    );
                }
            }

            if !report.dry_run {
                println!(
                    "{} {}",
                    "Removed agents:".dimmed(),
                    report.removed_agents.len()
                );
                println!(
                    "{} {}",
                    "Removed reservations:".dimmed(),
                    report.removed_reservations
                );
                println!(
                    "{} {}",
                    "Removed inbox messages:".dimmed(),
                    report.removed_inbox_messages
                );
            }
        }
        Format::Minimal => {
            if !stale {
                println!("noop");
            } else {
                println!("{}", report.removed_agents.len());
            }
        }
    }

    Ok(())
}

pub fn blockers(repo_root: &Path, paths: Vec<String>, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let reservations = store.list_reservations()?;
    let normalized_targets = normalize_targets(paths, repo_root)?;
    let blockers = collect_blockers(&reservations, &normalized_targets);

    match format {
        Format::Json => println!("{}", serde_json::to_string(&blockers)?),
        Format::Pretty => {
            if blockers.is_empty() {
                if normalized_targets.is_empty() {
                    println!("{}", "No active reservation blockers.".dimmed());
                } else {
                    println!(
                        "{}",
                        "No reservation blockers for requested paths.".dimmed()
                    );
                }
            } else {
                for blocker in &blockers {
                    println!(
                        "{} {} {}",
                        format!("[{}]", blocker.owner).cyan().bold(),
                        blocker.path.green(),
                        format!("age={}s", blocker.age_secs).dimmed(),
                    );
                    if let Some(reason) = blocker.reason.as_deref() {
                        println!("  {} {}", "reason:".dimmed(), reason);
                    }
                }
            }
        }
        Format::Minimal => {
            for blocker in &blockers {
                println!(
                    "{}\t{}\t{}\t{}",
                    blocker.owner,
                    blocker.path,
                    blocker.age_secs,
                    blocker.reason.as_deref().unwrap_or("")
                );
            }
        }
    }

    Ok(())
}

pub fn send(repo_root: &Path, from: &str, to: &str, text: &str, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let msg = store.send(from, to, text, None)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&msg)?),
        Format::Pretty => println!("Sent to '{}': {}", to.cyan(), text),
        Format::Minimal => println!("{}", msg.id),
    }
    Ok(())
}

pub fn broadcast(repo_root: &Path, from: &str, text: &str, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let msgs = store.broadcast(from, text)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&msgs)?),
        Format::Pretty => println!(
            "Broadcast to {} agents: {}",
            msgs.len().to_string().bold(),
            text
        ),
        Format::Minimal => println!("{}", msgs.len()),
    }
    Ok(())
}

pub fn inbox(repo_root: &Path, name: &str, ack: bool, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let msgs = store.inbox(name, ack)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&msgs)?),
        Format::Pretty => {
            if msgs.is_empty() {
                println!("{}", "No messages.".dimmed());
            } else {
                for m in &msgs {
                    let short_id = m.id.get(..8).unwrap_or(&m.id);
                    println!(
                        "{} {} {}",
                        format!("[{}]", short_id).dimmed(),
                        format!("{}:", m.from).cyan(),
                        m.text,
                    );
                }
            }
        }
        Format::Minimal => {
            for m in &msgs {
                println!("{}: {}", m.from, m.text);
            }
        }
    }
    Ok(())
}

pub fn reserve(
    repo_root: &Path,
    name: &str,
    paths: Vec<String>,
    reason: Option<&str>,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let res = store.reserve(name, paths, reason)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&res)?),
        Format::Pretty => {
            println!("Reserved by '{}':", res.agent.cyan().bold());
            for p in &res.paths {
                println!("  {}", p.green());
            }
            if let Some(ref r) = res.reason {
                println!("  {} {}", "reason:".dimmed(), r);
            }
        }
        Format::Minimal => println!("{}", res.paths.join(",")),
    }
    Ok(())
}

pub fn release(
    repo_root: &Path,
    name: &str,
    paths: Vec<String>,
    all: bool,
    format: Format,
) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let release_paths = if all { vec![] } else { paths };
    store.release(name, release_paths)?;
    match format {
        Format::Json => println!("{}", serde_json::json!({"released": true})),
        Format::Pretty => println!("{}", "Released.".green()),
        Format::Minimal => println!("ok"),
    }
    Ok(())
}

pub fn feed(repo_root: &Path, limit: Option<usize>, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let events = store.read_feed(limit)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&events)?),
        Format::Pretty => {
            if events.is_empty() {
                println!("{}", "No feed events.".dimmed());
            } else {
                for e in &events {
                    let target = e.target.as_deref().unwrap_or("");
                    let preview = e.preview.as_deref().unwrap_or("");
                    println!(
                        "{} {} {} {} {}",
                        e.ts.format("%H:%M:%S").to_string().dimmed(),
                        format!("[{}]", e.agent).cyan(),
                        e.event_type,
                        target,
                        preview.dimmed()
                    );
                }
            }
        }
        Format::Minimal => {
            for e in &events {
                println!("{} {}", e.agent, e.event_type);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn collect_blockers_reports_owner_path_reason_and_age() {
        let reservations = vec![Reservation {
            agent: "agent-a".into(),
            paths: vec!["src/store".into()],
            reason: Some("task-1".into()),
            since: Utc::now() - Duration::seconds(10),
            ttl_secs: Some(60),
            last_heartbeat_at: None,
            expires_at: None,
        }];

        let blockers = collect_blockers(&reservations, &[]);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].owner, "agent-a");
        assert_eq!(blockers[0].path, "src/store");
        assert_eq!(blockers[0].reason.as_deref(), Some("task-1"));
        assert!(blockers[0].age_secs >= 0);
    }

    #[test]
    fn collect_blockers_filters_by_target_path_conflict() {
        let reservations = vec![Reservation {
            agent: "agent-a".into(),
            paths: vec!["src/store".into(), "README.md".into()],
            reason: None,
            since: Utc::now(),
            ttl_secs: Some(60),
            last_heartbeat_at: None,
            expires_at: None,
        }];

        let blockers = collect_blockers(&reservations, &["src/store/mesh.rs".into()]);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].path, "src/store");
    }

    #[test]
    fn normalize_targets_canonicalizes_and_deduplicates() {
        let dir = tempdir().unwrap();

        let normalized = normalize_targets(
            vec!["./src/store/".into(), "src/./store".into()],
            dir.path(),
        )
        .unwrap();

        assert_eq!(normalized, vec!["src/store"]);
    }

    #[test]
    fn normalize_targets_rejects_invalid_paths() {
        let dir = tempdir().unwrap();

        let err = normalize_targets(vec!["../etc/passwd".into()], dir.path()).unwrap_err();
        assert!(matches!(err, TakError::MeshInvalidPath(_)));
    }
}
