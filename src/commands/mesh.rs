use std::path::Path;

use crate::error::Result;
use crate::output::Format;
use crate::store::mesh::MeshStore;

pub fn join(repo_root: &Path, name: &str, session_id: Option<&str>, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let reg = store.join(name, session_id)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&reg)?),
        Format::Pretty => {
            println!("Joined mesh as '{}'", reg.name);
            println!("  session: {}", reg.session_id);
            println!("  pid: {}", reg.pid);
        }
        Format::Minimal => println!("{}", reg.name),
    }
    Ok(())
}

pub fn leave(repo_root: &Path, name: &str, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    store.leave(name)?;
    match format {
        Format::Json => println!("{}", serde_json::json!({"left": name})),
        Format::Pretty => println!("Left mesh: '{name}'"),
        Format::Minimal => println!("{name}"),
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
                println!("No agents in mesh.");
            } else {
                for a in &agents {
                    println!("[{}] pid={} session={}", a.name, a.pid, a.session_id);
                    println!("  cwd: {}", a.cwd);
                    println!("  status: {}", a.status);
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

pub fn send(repo_root: &Path, from: &str, to: &str, text: &str, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let msg = store.send(from, to, text, None)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&msg)?),
        Format::Pretty => println!("Sent to '{}': {}", to, text),
        Format::Minimal => println!("{}", msg.id),
    }
    Ok(())
}

pub fn broadcast(repo_root: &Path, from: &str, text: &str, format: Format) -> Result<()> {
    let store = MeshStore::open(&repo_root.join(".tak"));
    let msgs = store.broadcast(from, text)?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&msgs)?),
        Format::Pretty => println!("Broadcast to {} agents: {}", msgs.len(), text),
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
                println!("No messages.");
            } else {
                for m in &msgs {
                    let short_id = &m.id[..8];
                    println!("[{}] from={}: {}", short_id, m.from, m.text);
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
            println!("Reserved by '{}':", res.agent);
            for p in &res.paths {
                println!("  {p}");
            }
            if let Some(ref r) = res.reason {
                println!("  reason: {r}");
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
        Format::Pretty => println!("Released."),
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
                println!("No feed events.");
            } else {
                for e in &events {
                    let target = e.target.as_deref().unwrap_or("");
                    let preview = e.preview.as_deref().unwrap_or("");
                    println!(
                        "{} [{}] {} {} {}",
                        e.ts.format("%H:%M:%S"),
                        e.agent,
                        e.event_type,
                        target,
                        preview
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
