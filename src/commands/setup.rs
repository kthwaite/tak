use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::error::{Result, TakError};
use crate::output::Format;

// Embedded Claude Code plugin assets — compiled into the binary.
const PLUGIN_JSON: &str = include_str!("../../claude-plugin/.claude-plugin/plugin.json");
const SKILL_TASK_MGMT: &str = include_str!("../../claude-plugin/skills/task-management/SKILL.md");
const SKILL_EPIC_PLAN: &str = include_str!("../../claude-plugin/skills/epic-planning/SKILL.md");
const SKILL_TASK_EXEC: &str = include_str!("../../claude-plugin/skills/task-execution/SKILL.md");

// Embedded pi integration assets — compiled into the binary.
const PI_EXTENSION_TAK: &str = include_str!("../../pi-plugin/extensions/tak.ts");
const PI_SKILL_COORDINATION: &str =
    include_str!("../../pi-plugin/skills/tak-coordination/SKILL.md");

const PI_SYSTEM_APPEND_START: &str = "<!-- tak:pi-system:start -->";
const PI_SYSTEM_APPEND_END: &str = "<!-- tak:pi-system:end -->";
const PI_SYSTEM_APPEND_BODY: &str = "Use `tak` actively as the source of truth for work planning and execution.\n\nCoordination rules:\n- Prefer `/tak` and `tak_cli` for task selection and updates.\n- Prioritise urgent tasks first (critical/high), then oldest tasks first within the same priority.\n- Keep task state accurate with lifecycle commands (`claim/start/handoff/finish/cancel/reopen`).\n- If mesh peers are active, avoid stepping on their toes:\n  - check mesh state/inbox,\n  - reserve files before major edits,\n  - coordinate via mesh and blackboard before overlapping work.\n- Use blackboard notes for blockers, handoffs, and cross-agent context.";

const REINDEX_HOOK_COMMAND: &str = "tak reindex 2>/dev/null || true";
const MESH_CLEANUP_HOOK_COMMAND: &str =
    "tak mesh cleanup --stale --format minimal >/dev/null 2>/dev/null || true";
const MESH_JOIN_HOOK_COMMAND: &str =
    "tak mesh join --format minimal >/dev/null 2>/dev/null || true";
const MESH_LEAVE_HOOK_COMMAND: &str =
    "tak mesh leave --format minimal >/dev/null 2>/dev/null || true";

fn session_start_hook_entry() -> Value {
    json!({
        "matcher": "",
        "hooks": [
            {
                "type": "command",
                "command": REINDEX_HOOK_COMMAND,
                "timeout": 10
            },
            {
                "type": "command",
                "command": MESH_CLEANUP_HOOK_COMMAND,
                "timeout": 10
            },
            {
                "type": "command",
                "command": MESH_JOIN_HOOK_COMMAND,
                "timeout": 10
            }
        ]
    })
}

fn stop_hook_entry() -> Value {
    json!({
        "matcher": "",
        "hooks": [
            {
                "type": "command",
                "command": MESH_LEAVE_HOOK_COMMAND,
                "timeout": 10
            }
        ]
    })
}

/// Plugin files to write inside the project-local `.claude/` directory.
fn plugin_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            ".claude/plugins/tak/.claude-plugin/plugin.json",
            PLUGIN_JSON,
        ),
        (
            ".claude/plugins/tak/skills/task-management/SKILL.md",
            SKILL_TASK_MGMT,
        ),
        (
            ".claude/plugins/tak/skills/epic-planning/SKILL.md",
            SKILL_EPIC_PLAN,
        ),
        (
            ".claude/plugins/tak/skills/task-execution/SKILL.md",
            SKILL_TASK_EXEC,
        ),
    ]
}

fn claude_skills_base_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = std::env::var("HOME").map_err(|_| {
            TakError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME not set",
            ))
        })?;
        Ok(PathBuf::from(home).join(".claude").join("skills"))
    } else {
        Ok(PathBuf::from(".claude").join("skills"))
    }
}

fn claude_skill_files(base: &Path) -> Vec<(PathBuf, &'static str)> {
    vec![
        (
            base.join("task-management").join("SKILL.md"),
            SKILL_TASK_MGMT,
        ),
        (base.join("epic-planning").join("SKILL.md"), SKILL_EPIC_PLAN),
        (
            base.join("task-execution").join("SKILL.md"),
            SKILL_TASK_EXEC,
        ),
    ]
}

fn pi_base_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = std::env::var("HOME").map_err(|_| {
            TakError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME not set",
            ))
        })?;
        Ok(PathBuf::from(home).join(".pi").join("agent"))
    } else {
        Ok(PathBuf::from(".pi"))
    }
}

fn pi_files(base: &Path) -> Vec<(PathBuf, &'static str)> {
    vec![
        (base.join("extensions").join("tak.ts"), PI_EXTENSION_TAK),
        (
            base.join("skills")
                .join("tak-coordination")
                .join("SKILL.md"),
            PI_SKILL_COORDINATION,
        ),
    ]
}

fn pi_append_system_path(base: &Path) -> PathBuf {
    base.join("APPEND_SYSTEM.md")
}

#[cfg(test)]
fn pi_system_block() -> String {
    format!(
        "{}\n{}\n{}",
        PI_SYSTEM_APPEND_START, PI_SYSTEM_APPEND_BODY, PI_SYSTEM_APPEND_END
    )
}

fn upsert_marked_block(existing: &str, start: &str, end: &str, body: &str) -> (String, bool) {
    let block = format!("{}\n{}\n{}", start, body, end);

    if let Some(start_idx) = existing.find(start)
        && let Some(end_rel) = existing[start_idx..].find(end)
    {
        let end_idx = start_idx + end_rel + end.len();
        let mut updated = String::new();
        updated.push_str(&existing[..start_idx]);
        updated.push_str(&block);
        updated.push_str(&existing[end_idx..]);
        return (updated.clone(), updated != existing);
    }

    let mut updated = existing.to_string();
    if updated.trim().is_empty() {
        updated = format!("{block}\n");
    } else {
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
        updated.push_str(&block);
        updated.push('\n');
    }

    (updated.clone(), updated != existing)
}

fn remove_marked_block(existing: &str, start: &str, end: &str) -> (String, bool) {
    if let Some(start_idx) = existing.find(start)
        && let Some(end_rel) = existing[start_idx..].find(end)
    {
        let end_idx = start_idx + end_rel + end.len();
        let mut updated = String::new();
        updated.push_str(&existing[..start_idx]);
        updated.push_str(&existing[end_idx..]);
        return (updated, true);
    }
    (existing.to_string(), false)
}

fn upsert_pi_append_system(path: &Path) -> Result<bool> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let (updated, changed) = upsert_marked_block(
        &existing,
        PI_SYSTEM_APPEND_START,
        PI_SYSTEM_APPEND_END,
        PI_SYSTEM_APPEND_BODY,
    );

    if !changed {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn remove_pi_append_system(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let existing = fs::read_to_string(path)?;
    let (updated, changed) =
        remove_marked_block(&existing, PI_SYSTEM_APPEND_START, PI_SYSTEM_APPEND_END);

    if !changed {
        return Ok(false);
    }

    if updated.trim().is_empty() {
        fs::remove_file(path)?;
    } else {
        fs::write(path, updated)?;
    }
    Ok(true)
}

fn check_pi_installed_at(base: &Path) -> &'static str {
    let files = pi_files(base);
    let mut any_exists = false;
    let mut any_mismatch = false;

    for (path, expected) in &files {
        if path.exists() {
            any_exists = true;
            if let Ok(actual) = fs::read_to_string(path) {
                if actual.trim() != expected.trim() {
                    any_mismatch = true;
                }
            } else {
                any_mismatch = true;
            }
        } else if any_exists {
            any_mismatch = true;
        }
    }

    let append_path = pi_append_system_path(base);
    let has_prompt_block = fs::read_to_string(&append_path).ok().is_some_and(|text| {
        text.contains(PI_SYSTEM_APPEND_START) && text.contains(PI_SYSTEM_APPEND_END)
    });

    if !any_exists && !has_prompt_block {
        "not installed"
    } else if any_mismatch || !has_prompt_block {
        "outdated"
    } else {
        "installed"
    }
}

pub fn check_pi_installed(global: bool) -> &'static str {
    let Ok(base) = pi_base_path(global) else {
        return "not installed";
    };
    check_pi_installed_at(&base)
}

/// Check project-local pi integration status at an explicit repo root.
pub fn check_project_pi_installed(repo_root: &Path) -> &'static str {
    check_pi_installed_at(&repo_root.join(".pi"))
}

/// Check whether this binary's embedded pi assets match the repository source files.
///
/// Returns `Ok(false)` when source files exist but content differs from embedded assets.
pub fn embedded_pi_assets_match_repo_source(repo_root: &Path) -> Result<bool> {
    let source_extension = fs::read_to_string(repo_root.join("pi-plugin/extensions/tak.ts"))?;
    let source_skill =
        fs::read_to_string(repo_root.join("pi-plugin/skills/tak-coordination/SKILL.md"))?;

    Ok(source_extension.trim() == PI_EXTENSION_TAK.trim()
        && source_skill.trim() == PI_SKILL_COORDINATION.trim())
}

fn write_pi_files(global: bool, format: Format) -> Result<bool> {
    let base = pi_base_path(global)?;
    let mut changed = false;

    for (path, content) in pi_files(&base) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let existing = fs::read_to_string(&path)?;
            if existing.trim() == content.trim() {
                if format == Format::Pretty {
                    eprintln!("  skip  {} (unchanged)", path.display());
                }
                continue;
            }
            if format == Format::Pretty {
                eprintln!("  write {} (updated)", path.display());
            }
        } else if format == Format::Pretty {
            eprintln!("  write {}", path.display());
        }

        fs::write(&path, content)?;
        changed = true;
    }

    let append_path = pi_append_system_path(&base);
    if upsert_pi_append_system(&append_path)? {
        changed = true;
        if format == Format::Pretty {
            eprintln!("  upsert {}", append_path.display());
        }
    }

    Ok(changed)
}

fn remove_dir_if_empty(path: &Path) -> Result<()> {
    match fs::read_dir(path) {
        Ok(mut iter) => {
            if iter.next().is_none() {
                fs::remove_dir(path)?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn remove_pi_files(global: bool, format: Format) -> Result<bool> {
    let base = pi_base_path(global)?;
    let mut changed = false;

    for (path, _) in pi_files(&base) {
        if path.exists() {
            fs::remove_file(&path)?;
            changed = true;
            if format == Format::Pretty {
                eprintln!("  remove {}", path.display());
            }
        }
    }

    let append_path = pi_append_system_path(&base);
    if remove_pi_append_system(&append_path)? {
        changed = true;
        if format == Format::Pretty {
            eprintln!("  remove tak block from {}", append_path.display());
        }
    }

    // Best-effort cleanup of empty integration directories.
    let skill_dir = base.join("skills").join("tak-coordination");
    let skills_root = base.join("skills");
    let extensions_root = base.join("extensions");

    remove_dir_if_empty(&skill_dir)?;
    remove_dir_if_empty(&skills_root)?;
    remove_dir_if_empty(&extensions_root)?;
    remove_dir_if_empty(&base)?;

    Ok(changed)
}

/// Resolve the settings file path.
fn settings_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = std::env::var("HOME").map_err(|_| {
            TakError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME not set",
            ))
        })?;
        Ok(PathBuf::from(home).join(".claude").join("settings.json"))
    } else {
        Ok(PathBuf::from(".claude").join("settings.local.json"))
    }
}

fn is_git_repo_root(path: &Path) -> bool {
    path.join(".git").exists()
}

fn ensure_git_repo_root() -> Result<()> {
    let cwd = std::env::current_dir()?;
    if is_git_repo_root(&cwd) {
        Ok(())
    } else {
        Err(TakError::NotGitRepository)
    }
}

/// Read a settings file, returning an empty object if absent.
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    if path.exists() {
        let data = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&data)?;
        match val {
            Value::Object(map) => Ok(map),
            _ => Ok(Map::new()),
        }
    } else {
        Ok(Map::new())
    }
}

/// Write settings with pretty formatting.
fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&Value::Object(settings.clone()))?;
    fs::write(path, json + "\n")?;
    Ok(())
}

fn is_tak_entry_with_command(entry: &Value, command: &str) -> bool {
    let Some(obj) = entry.as_object() else {
        return false;
    };

    if obj.get("matcher").and_then(Value::as_str) != Some("") {
        return false;
    }

    let Some(hooks) = obj.get("hooks").and_then(Value::as_array) else {
        return false;
    };

    hooks.iter().any(|hook| {
        hook.get("type").and_then(Value::as_str) == Some("command")
            && hook.get("command").and_then(Value::as_str) == Some(command)
    })
}

fn is_tak_session_start_entry(entry: &Value) -> bool {
    is_tak_entry_with_command(entry, REINDEX_HOOK_COMMAND)
        || is_tak_entry_with_command(entry, MESH_CLEANUP_HOOK_COMMAND)
        || is_tak_entry_with_command(entry, MESH_JOIN_HOOK_COMMAND)
}

fn is_tak_stop_entry(entry: &Value) -> bool {
    is_tak_entry_with_command(entry, MESH_LEAVE_HOOK_COMMAND)
}

fn has_session_start_hook(session_start: &[Value]) -> bool {
    session_start
        .iter()
        .any(|entry| is_tak_entry_with_command(entry, REINDEX_HOOK_COMMAND))
        && session_start
            .iter()
            .any(|entry| is_tak_entry_with_command(entry, MESH_CLEANUP_HOOK_COMMAND))
        && session_start
            .iter()
            .any(|entry| is_tak_entry_with_command(entry, MESH_JOIN_HOOK_COMMAND))
}

fn has_stop_hook(stop: &[Value]) -> bool {
    stop.iter()
        .any(|entry| is_tak_entry_with_command(entry, MESH_LEAVE_HOOK_COMMAND))
}

fn command_hook(command: &str) -> Value {
    json!({
        "type": "command",
        "command": command,
        "timeout": 10
    })
}

fn upsert_commands_into_entry(entry: &mut Value, commands: &[&str]) -> bool {
    let Some(obj) = entry.as_object_mut() else {
        return false;
    };
    if obj.get("matcher").and_then(Value::as_str) != Some("") {
        return false;
    }

    let hooks_val = obj.entry("hooks").or_insert_with(|| json!([]));
    let hooks = match hooks_val.as_array_mut() {
        Some(a) => a,
        None => {
            *hooks_val = json!([]);
            hooks_val.as_array_mut().unwrap()
        }
    };

    let mut changed = false;
    for command in commands {
        let exists = hooks.iter().any(|hook| {
            hook.get("type").and_then(Value::as_str) == Some("command")
                && hook.get("command").and_then(Value::as_str) == Some(*command)
        });
        if !exists {
            hooks.push(command_hook(command));
            changed = true;
        }
    }

    changed
}

fn remove_commands_from_entry(entry: &mut Value, commands: &[&str]) -> bool {
    let Some(obj) = entry.as_object_mut() else {
        return false;
    };
    if obj.get("matcher").and_then(Value::as_str) != Some("") {
        return false;
    }

    let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_array_mut) else {
        return false;
    };

    let before = hooks.len();
    hooks.retain(|hook| {
        !(hook.get("type").and_then(Value::as_str) == Some("command")
            && hook
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|cmd| commands.contains(&cmd)))
    });
    hooks.len() != before
}

fn is_empty_matcher_entry_with_no_hooks(entry: &Value) -> bool {
    let Some(obj) = entry.as_object() else {
        return false;
    };
    if obj.get("matcher").and_then(Value::as_str) != Some("") {
        return false;
    }
    obj.get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| hooks.is_empty())
}

fn upsert_hook_entry(
    hooks_obj: &mut Map<String, Value>,
    event: &str,
    hook_entry: Value,
    matcher: fn(&Value) -> bool,
    commands: &[&str],
) -> bool {
    let event_val = hooks_obj.entry(event).or_insert_with(|| json!([]));

    let arr = match event_val.as_array_mut() {
        Some(a) => a,
        None => {
            *event_val = json!([]);
            event_val.as_array_mut().unwrap()
        }
    };

    if let Some(existing) = arr.iter_mut().find(|entry| matcher(entry)) {
        // Migrate/extend legacy tak hook entries in place without dropping
        // unrelated hook commands users may have colocated.
        return upsert_commands_into_entry(existing, commands);
    }

    if arr.iter().any(|entry| entry == &hook_entry) {
        return false;
    }

    arr.push(hook_entry);
    true
}

/// Install hook into settings.
fn install_hook(settings: &mut Map<String, Value>) -> bool {
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));

    let hooks_obj = match hooks.as_object_mut() {
        Some(obj) => obj,
        None => {
            *hooks = json!({});
            hooks.as_object_mut().unwrap()
        }
    };

    let mut changed = false;
    changed |= upsert_hook_entry(
        hooks_obj,
        "SessionStart",
        session_start_hook_entry(),
        is_tak_session_start_entry,
        [
            REINDEX_HOOK_COMMAND,
            MESH_CLEANUP_HOOK_COMMAND,
            MESH_JOIN_HOOK_COMMAND,
        ]
        .as_ref(),
    );
    changed |= upsert_hook_entry(
        hooks_obj,
        "Stop",
        stop_hook_entry(),
        is_tak_stop_entry,
        &[MESH_LEAVE_HOOK_COMMAND],
    );
    changed
}

/// Remove tak hook commands from settings. Returns true if anything was removed.
fn remove_hook(settings: &mut Map<String, Value>) -> bool {
    let Some(hooks) = settings.get_mut("hooks") else {
        return false;
    };
    let Some(hooks_obj) = hooks.as_object_mut() else {
        return false;
    };

    let mut removed = false;

    if let Some(session_start) = hooks_obj.get_mut("SessionStart")
        && let Some(arr) = session_start.as_array_mut()
    {
        for entry in arr.iter_mut() {
            removed |= remove_commands_from_entry(
                entry,
                &[
                    REINDEX_HOOK_COMMAND,
                    MESH_CLEANUP_HOOK_COMMAND,
                    MESH_JOIN_HOOK_COMMAND,
                ],
            );
        }
        let before = arr.len();
        arr.retain(|entry| !is_empty_matcher_entry_with_no_hooks(entry));
        removed |= arr.len() < before;
        if arr.is_empty() {
            hooks_obj.remove("SessionStart");
        }
    }

    if let Some(stop) = hooks_obj.get_mut("Stop")
        && let Some(arr) = stop.as_array_mut()
    {
        for entry in arr.iter_mut() {
            removed |= remove_commands_from_entry(entry, &[MESH_LEAVE_HOOK_COMMAND]);
        }
        let before = arr.len();
        arr.retain(|entry| !is_empty_matcher_entry_with_no_hooks(entry));
        removed |= arr.len() < before;
        if arr.is_empty() {
            hooks_obj.remove("Stop");
        }
    }

    if hooks_obj.is_empty() {
        settings.remove("hooks");
    }

    removed
}

/// Check whether hooks are installed. Returns Some("project") or Some("global") or None.
pub fn check_hooks_installed() -> Option<&'static str> {
    if settings_has_tak_hooks(false) {
        Some("project")
    } else if settings_has_tak_hooks(true) {
        Some("global")
    } else {
        None
    }
}

/// Check if the settings file (project or global) contains both tak hooks.
fn settings_has_tak_hooks(global: bool) -> bool {
    let Ok(path) = settings_path(global) else {
        return false;
    };
    let Ok(settings) = read_settings(&path) else {
        return false;
    };

    let Some(hooks) = settings.get("hooks") else {
        return false;
    };
    let Some(session_start) = hooks.get("SessionStart").and_then(Value::as_array) else {
        return false;
    };
    let Some(stop) = hooks.get("Stop").and_then(Value::as_array) else {
        return false;
    };

    has_session_start_hook(session_start) && has_stop_hook(stop)
}

/// Check whether plugin files exist and match embedded content.
/// Returns "installed", "outdated", or "not installed".
pub fn check_plugin_installed() -> &'static str {
    let files = plugin_files();
    let mut any_exists = false;
    let mut any_mismatch = false;

    for (rel_path, expected) in &files {
        let path = Path::new(rel_path);
        if path.exists() {
            any_exists = true;
            if let Ok(actual) = fs::read_to_string(path) {
                if actual.trim() != expected.trim() {
                    any_mismatch = true;
                }
            } else {
                any_mismatch = true;
            }
        } else {
            // If some files exist but not all, it's outdated
            if any_exists {
                any_mismatch = true;
            }
        }
    }

    if !any_exists {
        "not installed"
    } else if any_mismatch {
        "outdated"
    } else {
        "installed"
    }
}

fn check_claude_skills_installed_at(base: &Path) -> &'static str {
    let files = claude_skill_files(base);
    let mut any_exists = false;
    let mut any_mismatch = false;

    for (path, expected) in &files {
        if path.exists() {
            any_exists = true;
            if let Ok(actual) = fs::read_to_string(path) {
                if actual.trim() != expected.trim() {
                    any_mismatch = true;
                }
            } else {
                any_mismatch = true;
            }
        } else if any_exists {
            any_mismatch = true;
        }
    }

    if !any_exists {
        "not installed"
    } else if any_mismatch {
        "outdated"
    } else {
        "installed"
    }
}

pub fn check_claude_skills_installed(global: bool) -> &'static str {
    let Ok(base) = claude_skills_base_path(global) else {
        return "not installed";
    };
    check_claude_skills_installed_at(&base)
}

fn write_claude_skill_files(global: bool, format: Format) -> Result<bool> {
    let base = claude_skills_base_path(global)?;
    let mut changed = false;

    for (path, content) in claude_skill_files(&base) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let existing = fs::read_to_string(&path)?;
            if existing.trim() == content.trim() {
                if format == Format::Pretty {
                    eprintln!("  skip  {} (unchanged)", path.display());
                }
                continue;
            }
            if format == Format::Pretty {
                eprintln!("  write {} (updated)", path.display());
            }
        } else if format == Format::Pretty {
            eprintln!("  write {}", path.display());
        }

        fs::write(&path, content)?;
        changed = true;
    }

    Ok(changed)
}

fn remove_claude_skill_files(global: bool, format: Format) -> Result<bool> {
    let base = claude_skills_base_path(global)?;
    let mut changed = false;

    for (path, _) in claude_skill_files(&base) {
        if path.exists() {
            fs::remove_file(&path)?;
            changed = true;
            if format == Format::Pretty {
                eprintln!("  remove {}", path.display());
            }
        }
    }

    for skill_dir in ["task-management", "epic-planning", "task-execution"] {
        remove_dir_if_empty(&base.join(skill_dir))?;
    }
    remove_dir_if_empty(&base)?;

    Ok(changed)
}

/// Write plugin files to `.claude/plugins/tak`.
fn write_plugin_files(format: Format) -> Result<()> {
    for (rel_path, content) in plugin_files() {
        let path = Path::new(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let existing = fs::read_to_string(path)?;
            if existing.trim() == content.trim() {
                if format == Format::Pretty {
                    eprintln!("  skip  {rel_path} (unchanged)");
                }
                continue;
            }
            if format == Format::Pretty {
                eprintln!("  write {rel_path} (updated)");
            }
        } else if format == Format::Pretty {
            eprintln!("  write {rel_path}");
        }

        fs::write(path, content)?;
    }
    Ok(())
}

/// Remove plugin files from `.claude/plugins/tak`.
fn remove_plugin_files(format: Format) -> Result<()> {
    for (rel_path, _) in plugin_files() {
        let path = Path::new(rel_path);
        if path.exists() {
            fs::remove_file(path)?;
            if format == Format::Pretty {
                eprintln!("  remove {rel_path}");
            }
        }
    }
    Ok(())
}

fn hooks_requested(plugin: bool, pi: bool, skills: bool) -> bool {
    // `tak setup --skills` is an explicit skills-only mode.
    !(skills && !plugin && !pi)
}

pub fn run(
    global: bool,
    check: bool,
    remove: bool,
    plugin: bool,
    skills: bool,
    pi: bool,
    format: Format,
) -> Result<()> {
    // Project-scoped setup writes into repo-local config and must run from a git repo root.
    if !global || plugin {
        ensure_git_repo_root()?;
    }

    let manage_hooks = hooks_requested(plugin, pi, skills);

    if check {
        return run_check(global, plugin, skills, pi, manage_hooks, format);
    }
    if remove {
        return run_remove(global, plugin, skills, pi, manage_hooks, format);
    }
    run_install(global, plugin, skills, pi, manage_hooks, format)
}

fn run_check(
    global: bool,
    plugin: bool,
    skills: bool,
    pi: bool,
    manage_hooks: bool,
    format: Format,
) -> Result<()> {
    let hooks_status = if manage_hooks {
        check_hooks_installed()
    } else {
        None
    };
    let plugin_status = if plugin {
        Some(check_plugin_installed())
    } else {
        None
    };
    let skills_status = if skills {
        Some(check_claude_skills_installed(global))
    } else {
        None
    };
    let pi_status = if pi {
        Some(check_pi_installed(global))
    } else {
        None
    };
    let tak_in_path = which_tak();

    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            if manage_hooks {
                obj.insert(
                    "hooks".into(),
                    match hooks_status {
                        Some(scope) => json!({"installed": true, "scope": scope}),
                        None => json!({"installed": false}),
                    },
                );
            }
            if let Some(ps) = plugin_status {
                obj.insert("plugin".into(), json!(ps));
            }
            if let Some(ps) = skills_status {
                obj.insert(
                    "skills".into(),
                    json!({
                        "status": ps,
                        "scope": if global { "global" } else { "project" }
                    }),
                );
            }
            if let Some(ps) = pi_status {
                obj.insert(
                    "pi".into(),
                    json!({
                        "status": ps,
                        "scope": if global { "global" } else { "project" }
                    }),
                );
            }
            obj.insert("tak_binary".into(), json!(tak_in_path));
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            if manage_hooks {
                match hooks_status {
                    Some(scope) => eprintln!("hooks: installed ({scope})"),
                    None => eprintln!("hooks: not installed"),
                }
            }
            if let Some(ps) = plugin_status {
                eprintln!("plugin: {ps}");
            }
            if let Some(ps) = skills_status {
                let scope = if global { "global" } else { "project" };
                eprintln!("skills: {ps} ({scope})");
            }
            if let Some(ps) = pi_status {
                let scope = if global { "global" } else { "project" };
                eprintln!("pi: {ps} ({scope})");
            }
            if tak_in_path {
                eprintln!("tak binary: found in PATH");
            } else {
                eprintln!("tak binary: not found in PATH");
            }
        }
    }

    let hooks_ok = !manage_hooks || hooks_status.is_some();
    let plugin_ok = plugin_status.is_none_or(|s| s == "installed");
    let skills_ok = skills_status.is_none_or(|s| s == "installed");
    let pi_ok = pi_status.is_none_or(|s| s == "installed");

    if hooks_ok && plugin_ok && skills_ok && pi_ok {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn run_install(
    global: bool,
    plugin: bool,
    skills: bool,
    pi: bool,
    manage_hooks: bool,
    format: Format,
) -> Result<()> {
    let scope = if global { "global" } else { "project" };

    let mut hooks_changed = false;
    let mut hooks_path: Option<PathBuf> = None;
    if manage_hooks {
        let path = settings_path(global)?;
        let mut settings = read_settings(&path)?;

        hooks_changed = install_hook(&mut settings);
        write_settings(&path, &settings)?;
        hooks_path = Some(path);
    }

    let mut skills_changed = false;
    if skills {
        skills_changed = write_claude_skill_files(global, format)?;
    }

    let mut pi_changed = false;
    if pi {
        pi_changed = write_pi_files(global, format)?;
    }

    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert("action".into(), json!("install"));
            obj.insert("scope".into(), json!(scope));
            if manage_hooks {
                obj.insert("changed".into(), json!(hooks_changed));
                if let Some(path) = &hooks_path {
                    obj.insert("path".into(), json!(path.display().to_string()));
                }
            }
            if plugin {
                obj.insert("plugin".into(), json!(true));
            }
            if skills {
                obj.insert(
                    "skills".into(),
                    json!({"enabled": true, "changed": skills_changed}),
                );
            }
            if pi {
                obj.insert("pi".into(), json!({"enabled": true, "changed": pi_changed}));
            }
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            if manage_hooks {
                let path = hooks_path.expect("hooks path present when hooks are managed");
                if hooks_changed {
                    eprintln!("Installed tak hooks ({scope}): {}", path.display());
                } else {
                    eprintln!("Hooks already installed ({scope}): {}", path.display());
                }
            }
            if skills {
                if skills_changed {
                    eprintln!("Installed Claude skills ({scope})");
                } else {
                    eprintln!("Claude skills already installed ({scope})");
                }
            }
            if pi {
                if pi_changed {
                    eprintln!("Installed pi integration ({scope})");
                } else {
                    eprintln!("Pi integration already installed ({scope})");
                }
            }
        }
    }

    if plugin {
        write_plugin_files(format)?;
        if !matches!(format, Format::Json) {
            eprintln!("Plugin files written to .claude/plugins/tak");
        }
    }

    Ok(())
}

fn run_remove(
    global: bool,
    plugin: bool,
    skills: bool,
    pi: bool,
    manage_hooks: bool,
    format: Format,
) -> Result<()> {
    let scope = if global { "global" } else { "project" };

    let mut hooks_removed = false;
    let mut hooks_path: Option<PathBuf> = None;
    if manage_hooks {
        let path = settings_path(global)?;
        let mut settings = read_settings(&path)?;

        hooks_removed = remove_hook(&mut settings);
        write_settings(&path, &settings)?;
        hooks_path = Some(path);
    }

    let mut skills_removed = false;
    if skills {
        skills_removed = remove_claude_skill_files(global, format)?;
    }

    let mut pi_changed = false;
    if pi {
        pi_changed = remove_pi_files(global, format)?;
    }

    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert("action".into(), json!("remove"));
            obj.insert("scope".into(), json!(scope));
            if manage_hooks {
                obj.insert("changed".into(), json!(hooks_removed));
            }
            if plugin {
                obj.insert("plugin".into(), json!(true));
            }
            if skills {
                obj.insert("skills".into(), json!({"removed": skills_removed}));
            }
            if pi {
                obj.insert("pi".into(), json!({"removed": pi_changed}));
            }
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            if manage_hooks {
                let path = hooks_path.expect("hooks path present when hooks are managed");
                if hooks_removed {
                    eprintln!("Removed tak hooks ({scope}): {}", path.display());
                } else {
                    eprintln!("No tak hooks found ({scope}): {}", path.display());
                }
            }
            if skills {
                if skills_removed {
                    eprintln!("Removed Claude skills ({scope})");
                } else {
                    eprintln!("No Claude skills found ({scope})");
                }
            }
            if pi {
                if pi_changed {
                    eprintln!("Removed pi integration ({scope})");
                } else {
                    eprintln!("No pi integration found ({scope})");
                }
            }
        }
    }

    if plugin {
        remove_plugin_files(format)?;
        if !matches!(format, Format::Json) {
            eprintln!("Plugin files removed from .claude/plugins/tak");
        }
    }

    Ok(())
}

fn which_tak() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("tak").exists()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn install_hook_idempotent() {
        let mut settings = Map::new();
        assert!(install_hook(&mut settings));
        assert!(!install_hook(&mut settings)); // second call is no-op
    }

    #[test]
    fn install_hook_migrates_legacy_entry() {
        let mut settings = Map::new();
        settings.insert(
            "hooks".into(),
            json!({
                "SessionStart": [{
                    "matcher": "",
                    "hooks": [{
                        "type": "command",
                        "command": REINDEX_HOOK_COMMAND,
                        "timeout": 10
                    }]
                }]
            }),
        );

        assert!(install_hook(&mut settings));

        let arr = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "legacy hook should be replaced in-place");

        let hooks = arr[0]["hooks"].as_array().unwrap();
        assert!(
            hooks
                .iter()
                .any(|h| h["command"] == json!(REINDEX_HOOK_COMMAND))
        );
        assert!(
            hooks
                .iter()
                .any(|h| h["command"] == json!(MESH_JOIN_HOOK_COMMAND))
        );

        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "stop hook should be added");
        let stop_hooks = stop[0]["hooks"].as_array().unwrap();
        assert!(
            stop_hooks
                .iter()
                .any(|h| h["command"] == json!(MESH_LEAVE_HOOK_COMMAND))
        );
    }

    #[test]
    fn install_hook_preserves_colocated_non_tak_commands() {
        let mut settings = Map::new();
        settings.insert(
            "hooks".into(),
            json!({
                "SessionStart": [{
                    "matcher": "",
                    "hooks": [
                        {"type": "command", "command": "echo custom start", "timeout": 5},
                        {"type": "command", "command": REINDEX_HOOK_COMMAND, "timeout": 10}
                    ]
                }],
                "Stop": [{
                    "matcher": "",
                    "hooks": [
                        {"type": "command", "command": "echo custom stop", "timeout": 5},
                        {"type": "command", "command": MESH_LEAVE_HOOK_COMMAND, "timeout": 10}
                    ]
                }]
            }),
        );

        assert!(install_hook(&mut settings));

        let start_hooks = settings["hooks"]["SessionStart"][0]["hooks"]
            .as_array()
            .unwrap();
        assert!(
            start_hooks
                .iter()
                .any(|h| h["command"] == json!("echo custom start"))
        );
        assert!(
            start_hooks
                .iter()
                .any(|h| h["command"] == json!(REINDEX_HOOK_COMMAND))
        );
        assert!(
            start_hooks
                .iter()
                .any(|h| h["command"] == json!(MESH_JOIN_HOOK_COMMAND))
        );

        let stop_hooks = settings["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert!(
            stop_hooks
                .iter()
                .any(|h| h["command"] == json!("echo custom stop"))
        );
        assert!(
            stop_hooks
                .iter()
                .any(|h| h["command"] == json!(MESH_LEAVE_HOOK_COMMAND))
        );
    }

    #[test]
    fn remove_hook_preserves_colocated_non_tak_commands() {
        let mut settings = Map::new();
        settings.insert(
            "hooks".into(),
            json!({
                "SessionStart": [{
                    "matcher": "",
                    "hooks": [
                        {"type": "command", "command": "echo custom start", "timeout": 5},
                        {"type": "command", "command": REINDEX_HOOK_COMMAND, "timeout": 10},
                        {"type": "command", "command": MESH_JOIN_HOOK_COMMAND, "timeout": 10}
                    ]
                }],
                "Stop": [{
                    "matcher": "",
                    "hooks": [
                        {"type": "command", "command": "echo custom stop", "timeout": 5},
                        {"type": "command", "command": MESH_LEAVE_HOOK_COMMAND, "timeout": 10}
                    ]
                }]
            }),
        );

        assert!(remove_hook(&mut settings));

        let start_hooks = settings["hooks"]["SessionStart"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(start_hooks.len(), 1);
        assert_eq!(start_hooks[0]["command"], json!("echo custom start"));

        let stop_hooks = settings["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert_eq!(stop_hooks.len(), 1);
        assert_eq!(stop_hooks[0]["command"], json!("echo custom stop"));
    }

    #[test]
    fn remove_hook_cleans_up() {
        let mut settings = Map::new();
        install_hook(&mut settings);
        assert!(remove_hook(&mut settings));
        assert!(!remove_hook(&mut settings)); // already removed
        // hooks key should be gone
        assert!(!settings.contains_key("hooks"));
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut settings = Map::new();
        install_hook(&mut settings);
        write_settings(&path, &settings).unwrap();

        let loaded = read_settings(&path).unwrap();
        assert_eq!(Value::Object(settings), Value::Object(loaded),);
    }

    #[test]
    fn read_missing_settings_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let settings = read_settings(&path).unwrap();
        assert!(settings.is_empty());
    }

    #[test]
    fn install_preserves_existing_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        // Write settings with existing content
        let mut settings = Map::new();
        settings.insert("model".into(), json!("sonnet"));
        write_settings(&path, &settings).unwrap();

        // Install hook
        let mut settings = read_settings(&path).unwrap();
        install_hook(&mut settings);
        write_settings(&path, &settings).unwrap();

        // Verify existing key preserved
        let loaded = read_settings(&path).unwrap();
        assert_eq!(loaded.get("model"), Some(&json!("sonnet")));
        assert!(loaded.contains_key("hooks"));
    }

    #[test]
    fn install_preserves_existing_hooks() {
        let mut settings = Map::new();
        settings.insert(
            "hooks".into(),
            json!({
                "SessionStart": [{
                    "matcher": "",
                    "hooks": [{"type": "command", "command": "echo hello", "timeout": 5}]
                }]
            }),
        );

        install_hook(&mut settings);

        let arr = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "should have both the existing and new hook");

        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "stop hook should be added");
    }

    #[test]
    fn plugin_files_are_under_claude_plugins_tak() {
        for (path, _) in plugin_files() {
            assert!(
                path.starts_with(".claude/plugins/tak/"),
                "plugin path should live under .claude/plugins/tak: {path}"
            );
        }
    }

    #[test]
    fn claude_skill_files_are_under_claude_skills_paths() {
        let dir = tempdir().unwrap();
        let base = dir.path().join(".claude").join("skills");
        let files = claude_skill_files(&base);

        assert_eq!(files.len(), 3);
        assert!(
            files[0]
                .0
                .to_string_lossy()
                .ends_with("skills/task-management/SKILL.md"),
            "unexpected claude skill path: {}",
            files[0].0.display()
        );
        assert!(
            files[1]
                .0
                .to_string_lossy()
                .ends_with("skills/epic-planning/SKILL.md"),
            "unexpected claude skill path: {}",
            files[1].0.display()
        );
        assert!(
            files[2]
                .0
                .to_string_lossy()
                .ends_with("skills/task-execution/SKILL.md"),
            "unexpected claude skill path: {}",
            files[2].0.display()
        );
    }

    #[test]
    fn check_claude_skills_installed_at_reports_states() {
        let dir = tempdir().unwrap();
        let base = dir.path().join(".claude").join("skills");
        fs::create_dir_all(&base).unwrap();

        assert_eq!(check_claude_skills_installed_at(&base), "not installed");

        let files = claude_skill_files(&base);
        fs::create_dir_all(files[0].0.parent().unwrap()).unwrap();
        fs::write(&files[0].0, files[0].1).unwrap();
        assert_eq!(check_claude_skills_installed_at(&base), "outdated");

        fs::create_dir_all(files[1].0.parent().unwrap()).unwrap();
        fs::write(&files[1].0, files[1].1).unwrap();
        fs::create_dir_all(files[2].0.parent().unwrap()).unwrap();
        fs::write(&files[2].0, files[2].1).unwrap();

        assert_eq!(check_claude_skills_installed_at(&base), "installed");
    }

    #[test]
    fn hooks_requested_is_false_for_skills_only_mode() {
        assert!(!hooks_requested(false, false, true));
        assert!(hooks_requested(false, true, true));
        assert!(hooks_requested(true, false, true));
        assert!(hooks_requested(false, false, false));
    }

    #[test]
    fn pi_files_are_under_pi_extension_and_skill_paths() {
        let dir = tempdir().unwrap();
        let files = pi_files(dir.path());

        assert_eq!(files.len(), 2);
        assert!(
            files[0].0.to_string_lossy().ends_with("extensions/tak.ts"),
            "unexpected extension path: {}",
            files[0].0.display()
        );
        assert!(
            files[1]
                .0
                .to_string_lossy()
                .ends_with("skills/tak-coordination/SKILL.md"),
            "unexpected skill path: {}",
            files[1].0.display()
        );
    }

    #[test]
    fn marked_block_upsert_and_remove_are_idempotent() {
        let existing = "user line\n";

        let (with_block, changed) = upsert_marked_block(
            existing,
            PI_SYSTEM_APPEND_START,
            PI_SYSTEM_APPEND_END,
            PI_SYSTEM_APPEND_BODY,
        );
        assert!(changed);
        assert!(with_block.contains(PI_SYSTEM_APPEND_START));
        assert!(with_block.contains(PI_SYSTEM_APPEND_END));

        let (with_block_again, changed_again) = upsert_marked_block(
            &with_block,
            PI_SYSTEM_APPEND_START,
            PI_SYSTEM_APPEND_END,
            PI_SYSTEM_APPEND_BODY,
        );
        assert!(!changed_again);
        assert_eq!(with_block_again, with_block);

        let (removed, removed_changed) =
            remove_marked_block(&with_block, PI_SYSTEM_APPEND_START, PI_SYSTEM_APPEND_END);
        assert!(removed_changed);
        assert!(!removed.contains(PI_SYSTEM_APPEND_START));

        let (removed_again, removed_again_changed) =
            remove_marked_block(&removed, PI_SYSTEM_APPEND_START, PI_SYSTEM_APPEND_END);
        assert!(!removed_again_changed);
        assert_eq!(removed_again, removed);
    }

    #[test]
    fn check_pi_installed_at_reports_states() {
        let dir = tempdir().unwrap();
        let base = dir.path().join(".pi");
        fs::create_dir_all(&base).unwrap();

        assert_eq!(check_pi_installed_at(&base), "not installed");

        let files = pi_files(&base);
        fs::create_dir_all(files[0].0.parent().unwrap()).unwrap();
        fs::write(&files[0].0, files[0].1).unwrap();
        assert_eq!(check_pi_installed_at(&base), "outdated");

        fs::create_dir_all(files[1].0.parent().unwrap()).unwrap();
        fs::write(&files[1].0, files[1].1).unwrap();

        let append_path = pi_append_system_path(&base);
        fs::write(&append_path, pi_system_block()).unwrap();

        assert_eq!(check_pi_installed_at(&base), "installed");
    }

    #[test]
    fn git_repo_root_detection_checks_for_dot_git() {
        let dir = tempdir().unwrap();
        assert!(!is_git_repo_root(dir.path()));

        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_git_repo_root(dir.path()));
    }
}
