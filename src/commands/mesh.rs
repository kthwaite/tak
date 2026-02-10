use std::fs;
use std::path::Path;

use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::coordination_db::{CoordinationDb, DbRegistration, DbReservation};
use crate::store::paths::{normalize_reservation_path, normalized_paths_conflict};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_REGISTRATION_TTL_SECS: u64 = 15 * 60;
const DEFAULT_RESERVATION_TTL_SECS: u64 = 30 * 60;

// ---------------------------------------------------------------------------
// Command-layer helpers (formerly inside MeshStore)
// ---------------------------------------------------------------------------

/// Validate an agent name: non-empty, ASCII alphanumeric + hyphen + underscore.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(TakError::MeshInvalidName);
    }
    Ok(())
}

/// Resolve session ID from: explicit arg -> TAK_SESSION_ID -> CLAUDE_SESSION_ID -> uuid.
fn resolve_session_id(session_id: Option<&str>) -> String {
    session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::var("TAK_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("CLAUDE_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Get local hostname from environment.
fn local_host_name() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Resolve an agent name from the registry when no explicit name is given.
///
/// Resolution order:
/// 1) `$TAK_AGENT` env var (must exist in registry)
/// 2) `$TAK_SESSION_ID`/`$CLAUDE_SESSION_ID` session match (cwd breaks ties)
/// 3) Single agent in current cwd
/// 4) Single agent in registry
fn resolve_current_agent_name(db: &CoordinationDb) -> Result<String> {
    let agents = db.list_agents()?;
    if agents.is_empty() {
        return Err(TakError::MeshAgentNotFound("current-session".into()));
    }

    // TAK_AGENT env
    if let Some(name) = std::env::var("TAK_AGENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        && agents.iter().any(|a| a.name == name)
    {
        return Ok(name);
    }
    // TAK_AGENT might point at assignment metadata rather than a mesh registration.
    // Fall through to session/cwd resolution.

    resolve_from_session_cwd(&agents)
}

/// Resolve an agent name for runtime operations that accept an optional
/// `--name` plus optional `--session-id`.
///
/// If name is given, verify it exists. Otherwise fall through to
/// TAK_AGENT -> session-id match -> cwd match -> single agent fallback.
fn resolve_agent_name_for_runtime(
    db: &CoordinationDb,
    explicit_name: Option<&str>,
    explicit_session_id: Option<&str>,
) -> Result<String> {
    let agents = db.list_agents()?;
    if agents.is_empty() {
        return Err(TakError::MeshAgentNotFound("current-session".into()));
    }

    if let Some(name) = explicit_name {
        if agents.iter().any(|a| a.name == name) {
            return Ok(name.to_string());
        }
        return Err(TakError::MeshAgentNotFound(name.to_string()));
    }

    // TAK_AGENT env
    if let Some(name) = std::env::var("TAK_AGENT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && agents.iter().any(|a| a.name == *s))
    {
        return Ok(name);
    }

    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let session_id = explicit_session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("TAK_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("CLAUDE_SESSION_ID")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    resolve_from_session_cwd_with(&agents, session_id.as_deref(), &cwd)
}

/// Common resolution logic: session match -> cwd match -> single-agent fallback.
fn resolve_from_session_cwd(agents: &[DbRegistration]) -> Result<String> {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let session_id = std::env::var("TAK_SESSION_ID")
        .ok()
        .or_else(|| std::env::var("CLAUDE_SESSION_ID").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    resolve_from_session_cwd_with(agents, session_id.as_deref(), &cwd)
}

fn resolve_from_session_cwd_with(
    agents: &[DbRegistration],
    session_id: Option<&str>,
    cwd: &str,
) -> Result<String> {
    if let Some(sid) = session_id {
        let by_session: Vec<&DbRegistration> =
            agents.iter().filter(|a| a.session_id == sid).collect();
        if by_session.len() == 1 {
            return Ok(by_session[0].name.clone());
        }
        if by_session.len() > 1 {
            let by_session_cwd: Vec<&DbRegistration> = by_session
                .iter()
                .copied()
                .filter(|a| !cwd.is_empty() && a.cwd == cwd)
                .collect();
            if by_session_cwd.len() == 1 {
                return Ok(by_session_cwd[0].name.clone());
            }
            let ambiguous = if by_session_cwd.len() > 1 {
                by_session_cwd
            } else {
                by_session
            };
            let names = ambiguous
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(TakError::MeshAmbiguousAgent(names));
        }
    }

    let by_cwd: Vec<&DbRegistration> = agents
        .iter()
        .filter(|a| !cwd.is_empty() && a.cwd == cwd)
        .collect();
    if by_cwd.len() == 1 {
        return Ok(by_cwd[0].name.clone());
    }
    if by_cwd.len() > 1 {
        let names = by_cwd
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(TakError::MeshAmbiguousAgent(names));
    }

    if agents.len() == 1 {
        return Ok(agents[0].name.clone());
    }

    let names = agents
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(TakError::MeshAmbiguousAgent(names))
}

/// Read lease config from `.tak/config.json`, returning (registration_ttl, reservation_ttl).
pub(crate) fn read_lease_config(repo_root: &Path) -> (u64, u64) {
    let config_path = repo_root.join(".tak").join("config.json");
    let Ok(content) = fs::read_to_string(config_path) else {
        return (DEFAULT_REGISTRATION_TTL_SECS, DEFAULT_RESERVATION_TTL_SECS);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return (DEFAULT_REGISTRATION_TTL_SECS, DEFAULT_RESERVATION_TTL_SECS);
    };
    let Some(mesh) = value.get("mesh").and_then(serde_json::Value::as_object) else {
        return (DEFAULT_REGISTRATION_TTL_SECS, DEFAULT_RESERVATION_TTL_SECS);
    };

    let reg_ttl = mesh
        .get("registration_ttl_secs")
        .or_else(|| mesh.get("registration_ttl_seconds"))
        .and_then(|v| v.as_u64())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_REGISTRATION_TTL_SECS);

    let res_ttl = mesh
        .get("reservation_ttl_secs")
        .or_else(|| mesh.get("reservation_ttl_seconds"))
        .and_then(|v| v.as_u64())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_RESERVATION_TTL_SECS);

    (reg_ttl, res_ttl)
}

// ---------------------------------------------------------------------------
// Blocker support
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BlockerRecord {
    owner: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    age_secs: i64,
}

fn collect_blockers(
    reservations: &[DbReservation],
    normalized_targets: &[String],
) -> Vec<BlockerRecord> {
    let now = Utc::now();

    let mut blockers: Vec<BlockerRecord> = reservations
        .iter()
        .filter_map(|res| {
            let matches_targets = normalized_targets.is_empty()
                || normalized_targets
                    .iter()
                    .any(|target| normalized_paths_conflict(target, &res.path));

            if !matches_targets {
                return None;
            }

            Some(BlockerRecord {
                owner: res.agent.clone(),
                path: res.path.clone(),
                reason: res.reason.clone(),
                age_secs: (now - res.created_at).num_seconds().max(0),
            })
        })
        .collect();

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

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

pub fn join(
    repo_root: &Path,
    name: Option<&str>,
    session_id: Option<&str>,
    format: Format,
) -> Result<()> {
    if let Some(name) = name {
        validate_name(name)?;
    }

    let db = CoordinationDb::from_repo(repo_root)?;

    let resolved_name = if let Some(name) = name {
        name.to_string()
    } else {
        crate::agent::generated_fallback()
    };

    let sid = resolve_session_id(session_id);
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let pid = Some(std::process::id());
    let host = local_host_name();

    let reg = db.join_agent(&resolved_name, &sid, &cwd, pid, host.as_deref())?;

    // Best-effort feed event
    let _ = db.append_event(Some(&reg.name), "mesh.join", None, Some("joined the mesh"));

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
    let db = CoordinationDb::from_repo(repo_root)?;

    let left = if let Some(name) = name {
        db.leave_agent(name)?;
        name.to_string()
    } else {
        // Try TAK_AGENT first, then fall through to session/cwd resolution
        let resolved = if let Some(tak_name) = std::env::var("TAK_AGENT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            match db.leave_agent(&tak_name) {
                Ok(()) => Some(tak_name),
                Err(TakError::MeshAgentNotFound(_)) => None,
                Err(err) => return Err(err),
            }
        } else {
            None
        };

        if let Some(name) = resolved {
            name
        } else {
            let agent_name = resolve_current_agent_name(&db)?;
            db.leave_agent(&agent_name)?;
            agent_name
        }
    };

    // Best-effort feed event
    let _ = db.append_event(Some(&left), "mesh.leave", None, Some("left the mesh"));

    match format {
        Format::Json => println!("{}", serde_json::json!({"left": left})),
        Format::Pretty => println!("Left mesh: '{}'", left.cyan()),
        Format::Minimal => println!("{left}"),
    }
    Ok(())
}

pub fn list(repo_root: &Path, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let agents = db.list_agents()?;
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
                    println!("  {} {}", "updated:".dimmed(), a.updated_at);
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
    if let Some(name) = name {
        validate_name(name)?;
    }

    let db = CoordinationDb::from_repo(repo_root)?;
    let resolved_name = resolve_agent_name_for_runtime(&db, name, session_id)?;

    let pid = Some(std::process::id());
    let host = local_host_name();
    db.heartbeat_agent(&resolved_name, pid, host.as_deref())?;

    let now = Utc::now();

    // Best-effort feed event
    let _ = db.append_event(
        Some(&resolved_name),
        "mesh.heartbeat",
        None,
        Some("heartbeat"),
    );

    #[derive(Serialize)]
    struct HeartbeatOutput {
        agent: String,
        refreshed_at: chrono::DateTime<Utc>,
    }

    let output = HeartbeatOutput {
        agent: resolved_name.clone(),
        refreshed_at: now,
    };

    match format {
        Format::Json => println!("{}", serde_json::to_string(&output)?),
        Format::Pretty => {
            println!(
                "Heartbeat refreshed for '{}'",
                resolved_name.as_str().cyan().bold()
            );
            println!("  {} {}", "at:".dimmed(), now);
        }
        Format::Minimal => println!("{}", resolved_name),
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
    let db = CoordinationDb::from_repo(repo_root)?;
    let (reg_ttl, _) = read_lease_config(repo_root);
    let ttl_secs = ttl_seconds.filter(|v| *v > 0).unwrap_or(reg_ttl);

    #[derive(Serialize)]
    struct CleanupReport {
        dry_run: bool,
        ttl_secs: u64,
        candidates: Vec<CleanupCandidate>,
        removed_agents: Vec<String>,
        removed_reservations: usize,
        removed_inbox_messages: usize,
    }

    #[derive(Serialize)]
    struct CleanupCandidate {
        agent: String,
        reason: String,
        reservation_paths: Vec<String>,
        inbox_messages: usize,
    }

    if !stale {
        let report = CleanupReport {
            dry_run: true,
            ttl_secs,
            candidates: vec![],
            removed_agents: vec![],
            removed_reservations: 0,
            removed_inbox_messages: 0,
        };
        match format {
            Format::Json => println!("{}", serde_json::to_string(&report)?),
            Format::Pretty => {
                println!("{}", "No cleanup mode selected (--stale).".yellow());
            }
            Format::Minimal => println!("noop"),
        }
        return Ok(());
    }

    // Identify stale agents by checking updated_at
    let agents = db.list_agents()?;
    let now = Utc::now();
    let cutoff = now - chrono::Duration::seconds(ttl_secs as i64);

    let stale_agents: Vec<&DbRegistration> =
        agents.iter().filter(|a| a.updated_at < cutoff).collect();

    // Build candidate info
    let reservations = db.list_reservations()?;
    let candidates: Vec<CleanupCandidate> = stale_agents
        .iter()
        .map(|a| {
            let agent_paths: Vec<String> = reservations
                .iter()
                .filter(|r| r.agent == a.name)
                .map(|r| r.path.clone())
                .collect();
            CleanupCandidate {
                agent: a.name.clone(),
                reason: format!("updated_at {} is older than {}s", a.updated_at, ttl_secs),
                reservation_paths: agent_paths,
                inbox_messages: 0, // CoordinationDb doesn't expose per-agent inbox count easily
            }
        })
        .collect();

    let candidate_names: Vec<String> = candidates.iter().map(|c| c.agent.clone()).collect();

    if dry_run {
        let report = CleanupReport {
            dry_run: true,
            ttl_secs,
            candidates,
            removed_agents: vec![],
            removed_reservations: 0,
            removed_inbox_messages: 0,
        };

        match format {
            Format::Json => println!("{}", serde_json::to_string(&report)?),
            Format::Pretty => {
                println!(
                    "Mesh stale cleanup (dry-run) — ttl={}s",
                    ttl_secs.to_string().bold()
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
            }
            Format::Minimal => println!("{}", candidate_names.len()),
        }
    } else {
        // Actually perform cleanup
        let (removed_agents, _msgs_del, _events_del, reservations_del) =
            db.cleanup_all(ttl_secs as i64, ttl_secs as i64, ttl_secs as i64)?;

        let report = CleanupReport {
            dry_run: false,
            ttl_secs,
            candidates,
            removed_agents: removed_agents.clone(),
            removed_reservations: reservations_del,
            removed_inbox_messages: 0,
        };

        match format {
            Format::Json => println!("{}", serde_json::to_string(&report)?),
            Format::Pretty => {
                println!(
                    "Mesh stale cleanup (applied) — ttl={}s",
                    ttl_secs.to_string().bold()
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
                println!("{} {}", "Removed agents:".dimmed(), removed_agents.len());
                println!("{} {}", "Removed reservations:".dimmed(), reservations_del);
                println!("{} {}", "Removed inbox messages:".dimmed(), 0);
            }
            Format::Minimal => println!("{}", removed_agents.len()),
        }
    }

    Ok(())
}

pub fn blockers(repo_root: &Path, paths: Vec<String>, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let reservations = db.list_reservations()?;
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
    let db = CoordinationDb::from_repo(repo_root)?;
    let msg = db.send_message(from, to, text, None)?;

    // Best-effort feed event
    let _ = db.append_event(
        Some(from),
        "mesh.send",
        Some(to),
        Some(&truncate_preview(text, 80)),
    );

    match format {
        Format::Json => println!("{}", serde_json::to_string(&msg)?),
        Format::Pretty => println!("Sent to '{}': {}", to.cyan(), text),
        Format::Minimal => println!("{}", msg.id),
    }
    Ok(())
}

pub fn broadcast(repo_root: &Path, from: &str, text: &str, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let msgs = db.broadcast_message(from, text)?;

    // Best-effort feed event
    let _ = db.append_event(
        Some(from),
        "mesh.broadcast",
        None,
        Some(&truncate_preview(text, 80)),
    );

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

pub fn inbox(
    repo_root: &Path,
    name: &str,
    ack: bool,
    ack_ids: Vec<String>,
    ack_before: Option<&str>,
    format: Format,
) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let msgs = db.read_inbox(name)?;

    // Handle ack operations
    if ack {
        db.ack_all_messages(name)?;
    } else if !ack_ids.is_empty() {
        db.ack_messages(name, &ack_ids)?;
    } else if let Some(before) = ack_before {
        // Ack messages created before the given timestamp
        let before_ids: Vec<String> = msgs
            .iter()
            .filter(|m| m.created_at.to_rfc3339().as_str() <= before)
            .map(|m| m.id.clone())
            .collect();
        if !before_ids.is_empty() {
            db.ack_messages(name, &before_ids)?;
        }
    }

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
                        format!("{}:", m.from_agent).cyan(),
                        m.text,
                    );
                }
            }
        }
        Format::Minimal => {
            for m in &msgs {
                println!("{}: {}", m.from_agent, m.text);
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
    let db = CoordinationDb::from_repo(repo_root)?;
    let (_, reservation_ttl) = read_lease_config(repo_root);

    // Get agent's current generation for the generation fence
    let agent = db.get_agent(name)?;

    // Normalize paths at the command layer
    let normalized = normalize_targets(paths, repo_root)?;

    // Reserve each path (CoordinationDb reserves one path at a time)
    let mut reserved: Vec<DbReservation> = Vec::new();
    for path in &normalized {
        let res = db.reserve(name, agent.generation, path, reason, reservation_ttl as i64)?;
        reserved.push(res);
    }

    // Best-effort feed event
    let _ = db.append_event(
        Some(name),
        "mesh.reserve",
        Some(&normalized.join(", ")),
        reason,
    );

    #[derive(Serialize)]
    struct ReserveOutput {
        agent: String,
        paths: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    }

    let output = ReserveOutput {
        agent: name.to_string(),
        paths: normalized.clone(),
        reason: reason.map(str::to_string),
    };

    match format {
        Format::Json => println!("{}", serde_json::to_string(&output)?),
        Format::Pretty => {
            println!("Reserved by '{}':", name.cyan().bold());
            for p in &normalized {
                println!("  {}", p.green());
            }
            if let Some(r) = reason {
                println!("  {} {}", "reason:".dimmed(), r);
            }
        }
        Format::Minimal => println!("{}", normalized.join(",")),
    }

    let _ = reserved; // suppress unused warning; reserved for future use
    Ok(())
}

pub fn release(
    repo_root: &Path,
    name: &str,
    paths: Vec<String>,
    all: bool,
    format: Format,
) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;

    if all {
        db.release_all(name)?;
    } else {
        let normalized = normalize_targets(paths, repo_root)?;
        for path in &normalized {
            db.release_path(name, path)?;
        }
    }

    // Best-effort feed event
    let _ = db.append_event(
        Some(name),
        "mesh.release",
        None,
        Some(if all {
            "released all"
        } else {
            "released paths"
        }),
    );

    match format {
        Format::Json => println!("{}", serde_json::json!({"released": true})),
        Format::Pretty => println!("{}", "Released.".green()),
        Format::Minimal => println!("ok"),
    }
    Ok(())
}

pub fn feed(repo_root: &Path, limit: Option<usize>, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let events = db.read_events(limit.map(|l| l as u32))?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&events)?),
        Format::Pretty => {
            if events.is_empty() {
                println!("{}", "No feed events.".dimmed());
            } else {
                for e in &events {
                    let agent = e.agent.as_deref().unwrap_or("?");
                    let target = e.target.as_deref().unwrap_or("");
                    let preview = e.preview.as_deref().unwrap_or("");
                    println!(
                        "{} {} {} {} {}",
                        e.created_at.format("%H:%M:%S").to_string().dimmed(),
                        format!("[{}]", agent).cyan(),
                        e.event_type,
                        target,
                        preview.dimmed()
                    );
                }
            }
        }
        Format::Minimal => {
            for e in &events {
                let agent = e.agent.as_deref().unwrap_or("?");
                println!("{} {}", agent, e.event_type);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn truncate_preview(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn collect_blockers_reports_owner_path_reason_and_age() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("agent-a", "s", "/", None, None).unwrap();
        let agent = db.get_agent("agent-a").unwrap();
        db.reserve(
            "agent-a",
            agent.generation,
            "src/store",
            Some("task-1"),
            3600,
        )
        .unwrap();

        let reservations = db.list_reservations().unwrap();
        let blockers = collect_blockers(&reservations, &[]);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].owner, "agent-a");
        assert_eq!(blockers[0].path, "src/store");
        assert_eq!(blockers[0].reason.as_deref(), Some("task-1"));
        assert!(blockers[0].age_secs >= 0);
    }

    #[test]
    fn collect_blockers_filters_by_target_path_conflict() {
        let db = CoordinationDb::open_memory().unwrap();
        db.join_agent("agent-a", "s", "/", None, None).unwrap();
        let agent = db.get_agent("agent-a").unwrap();
        db.reserve("agent-a", agent.generation, "src/store", None, 3600)
            .unwrap();
        db.reserve("agent-a", agent.generation, "README.md", None, 3600)
            .unwrap();

        let reservations = db.list_reservations().unwrap();
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
