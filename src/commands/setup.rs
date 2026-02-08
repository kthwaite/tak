use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::error::{Result, TakError};
use crate::output::Format;

// Embedded plugin assets — compiled into the binary.
const PLUGIN_JSON: &str = include_str!("../../.claude-plugin/plugin.json");
const SKILL_TASK_MGMT: &str = include_str!("../../skills/task-management/SKILL.md");
const SKILL_EPIC_PLAN: &str = include_str!("../../skills/epic-planning/SKILL.md");
const SKILL_TASK_EXEC: &str = include_str!("../../skills/task-execution/SKILL.md");
const HOOKS_JSON: &str = include_str!("../../hooks/hooks.json");

/// The hook entry tak injects into Claude Code settings.
fn tak_hook_entry() -> Value {
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": "tak reindex 2>/dev/null || true",
            "timeout": 10
        }]
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
        (".claude/plugins/tak/hooks/hooks.json", HOOKS_JSON),
    ]
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

/// Check if the tak hook entry already exists in a SessionStart array.
fn has_tak_hook(session_start: &[Value]) -> bool {
    let target = tak_hook_entry();
    session_start.iter().any(|entry| entry == &target)
}

/// Install hook into settings.
fn install_hook(settings: &mut Map<String, Value>) -> bool {
    let hook_entry = tak_hook_entry();

    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));

    let hooks_obj = match hooks.as_object_mut() {
        Some(obj) => obj,
        None => {
            *hooks = json!({});
            hooks.as_object_mut().unwrap()
        }
    };

    let session_start = hooks_obj.entry("SessionStart").or_insert_with(|| json!([]));

    let arr = match session_start.as_array_mut() {
        Some(a) => a,
        None => {
            *session_start = json!([]);
            session_start.as_array_mut().unwrap()
        }
    };

    if has_tak_hook(arr) {
        return false; // already installed
    }

    arr.push(hook_entry);
    true
}

/// Remove tak hook entries from settings. Returns true if anything was removed.
fn remove_hook(settings: &mut Map<String, Value>) -> bool {
    let target = tak_hook_entry();

    let Some(hooks) = settings.get_mut("hooks") else {
        return false;
    };
    let Some(hooks_obj) = hooks.as_object_mut() else {
        return false;
    };
    let Some(session_start) = hooks_obj.get_mut("SessionStart") else {
        return false;
    };
    let Some(arr) = session_start.as_array_mut() else {
        return false;
    };

    let before = arr.len();
    arr.retain(|entry| entry != &target);
    let removed = arr.len() < before;

    // Clean up empty arrays/objects
    if arr.is_empty() {
        hooks_obj.remove("SessionStart");
    }
    if hooks_obj.is_empty() {
        settings.remove("hooks");
    }

    removed
}

/// Check whether hooks are installed. Returns Some("project") or Some("global") or None.
pub fn check_hooks_installed() -> Option<&'static str> {
    if settings_has_tak_hook(false) {
        Some("project")
    } else if settings_has_tak_hook(true) {
        Some("global")
    } else {
        None
    }
}

/// Check if the settings file (project or global) contains the tak hook.
fn settings_has_tak_hook(global: bool) -> bool {
    let Ok(path) = settings_path(global) else {
        return false;
    };
    let Ok(settings) = read_settings(&path) else {
        return false;
    };
    settings
        .get("hooks")
        .and_then(|h| h.get("SessionStart"))
        .and_then(|ss| ss.as_array())
        .is_some_and(|arr| has_tak_hook(arr))
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

pub fn run(global: bool, check: bool, remove: bool, plugin: bool, format: Format) -> Result<()> {
    // Project-scoped setup writes into `.claude/` and must run from a git repo root.
    if !global || plugin {
        ensure_git_repo_root()?;
    }

    if check {
        return run_check(plugin, format);
    }
    if remove {
        return run_remove(global, plugin, format);
    }
    run_install(global, plugin, format)
}

fn run_check(plugin: bool, format: Format) -> Result<()> {
    let hooks_status = check_hooks_installed();
    let plugin_status = if plugin {
        Some(check_plugin_installed())
    } else {
        None
    };
    let tak_in_path = which_tak();

    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "hooks".into(),
                match hooks_status {
                    Some(scope) => json!({"installed": true, "scope": scope}),
                    None => json!({"installed": false}),
                },
            );
            if let Some(ps) = plugin_status {
                obj.insert("plugin".into(), json!(ps));
            }
            obj.insert("tak_binary".into(), json!(tak_in_path));
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            match hooks_status {
                Some(scope) => eprintln!("hooks: installed ({scope})"),
                None => eprintln!("hooks: not installed"),
            }
            if let Some(ps) = plugin_status {
                eprintln!("plugin: {ps}");
            }
            if tak_in_path {
                eprintln!("tak binary: found in PATH");
            } else {
                eprintln!("tak binary: not found in PATH");
            }
        }
    }

    if hooks_status.is_some() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn run_install(global: bool, plugin: bool, format: Format) -> Result<()> {
    let path = settings_path(global)?;
    let mut settings = read_settings(&path)?;

    let installed = install_hook(&mut settings);
    write_settings(&path, &settings)?;

    let scope = if global { "global" } else { "project" };
    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert("action".into(), json!("install"));
            obj.insert("scope".into(), json!(scope));
            obj.insert("changed".into(), json!(installed));
            obj.insert("path".into(), json!(path.display().to_string()));
            if plugin {
                obj.insert("plugin".into(), json!(true));
            }
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            if installed {
                eprintln!("Installed tak hooks ({scope}): {}", path.display());
            } else {
                eprintln!("Hooks already installed ({scope}): {}", path.display());
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

fn run_remove(global: bool, plugin: bool, format: Format) -> Result<()> {
    let path = settings_path(global)?;
    let mut settings = read_settings(&path)?;

    let removed = remove_hook(&mut settings);
    write_settings(&path, &settings)?;

    let scope = if global { "global" } else { "project" };
    match format {
        Format::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert("action".into(), json!("remove"));
            obj.insert("scope".into(), json!(scope));
            obj.insert("changed".into(), json!(removed));
            if plugin {
                obj.insert("plugin".into(), json!(true));
            }
            println!("{}", serde_json::to_string(&Value::Object(obj))?);
        }
        _ => {
            if removed {
                eprintln!("Removed tak hooks ({scope}): {}", path.display());
            } else {
                eprintln!("No tak hooks found ({scope}): {}", path.display());
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
    fn git_repo_root_detection_checks_for_dot_git() {
        let dir = tempdir().unwrap();
        assert!(!is_git_repo_root(dir.path()));

        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_git_repo_root(dir.path()));
    }
}
