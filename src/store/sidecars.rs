use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A single event in a task's history log.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub timestamp: DateTime<Utc>,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub detail: serde_json::Map<String, serde_json::Value>,
}

/// Result of running all verification commands for a task.
#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResult {
    pub timestamp: DateTime<Utc>,
    pub results: Vec<CommandResult>,
    pub passed: bool,
}

/// Result of running a single verification command.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub passed: bool,
}

/// Manages sidecar files: per-task context notes, history logs,
/// verification results, and artifacts.
///
/// On-disk layout under `.tak/`:
///   - `context/{id}.md`                — free-form context notes (git-committed)
///   - `history/{id}.jsonl`             — append-only structured event log (git-committed)
///   - `verification_results/{id}.json` — latest verification run (gitignored)
///   - `artifacts/{id}/`                — per-task artifact directory (gitignored)
pub struct SidecarStore {
    root: PathBuf,
}

impl SidecarStore {
    /// Open an existing sidecar store rooted at `.tak/`.
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.to_path_buf(),
        }
    }

    fn context_dir(&self) -> PathBuf {
        self.root.join("context")
    }

    fn history_dir(&self) -> PathBuf {
        self.root.join("history")
    }

    fn verification_dir(&self) -> PathBuf {
        self.root.join("verification_results")
    }

    /// Path to the context file for a given task ID.
    pub fn context_path(&self, id: u64) -> PathBuf {
        self.context_dir().join(format!("{id}.md"))
    }

    /// Path to the history file for a given task ID.
    pub fn history_path(&self, id: u64) -> PathBuf {
        self.history_dir().join(format!("{id}.jsonl"))
    }

    /// Path to the verification result file for a given task ID.
    pub fn verification_path(&self, id: u64) -> PathBuf {
        self.verification_dir().join(format!("{id}.json"))
    }

    /// Path to the artifacts directory for a given task ID.
    pub fn artifacts_dir(&self, id: u64) -> PathBuf {
        self.root.join("artifacts").join(id.to_string())
    }

    /// Read the context notes for a task. Returns None if no context file exists.
    pub fn read_context(&self, id: u64) -> Result<Option<String>> {
        let path = self.context_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        Ok(Some(content))
    }

    /// Write (overwrite) the context notes for a task.
    pub fn write_context(&self, id: u64, text: &str) -> Result<()> {
        let dir = self.context_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        fs::write(self.context_path(id), text)?;
        Ok(())
    }

    /// Delete the context file for a task, if it exists.
    pub fn delete_context(&self, id: u64) -> Result<()> {
        let path = self.context_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Append a structured history event to the JSONL log for a task.
    pub fn append_history(&self, id: u64, event: &HistoryEvent) -> Result<()> {
        let dir = self.history_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        let path = self.history_path(id);

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Read the full history log for a task. Returns empty vec if no history file exists.
    pub fn read_history(&self, id: u64) -> Result<Vec<HistoryEvent>> {
        let path = self.history_path(id);
        if !path.exists() {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(path)?;
        let mut events = Vec::new();
        for line in content.lines() {
            if !line.trim().is_empty() {
                let event: HistoryEvent = serde_json::from_str(line)?;
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Delete the history file for a task, if it exists.
    pub fn delete_history(&self, id: u64) -> Result<()> {
        let path = self.history_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Write a verification result for a task.
    pub fn write_verification(&self, id: u64, result: &VerificationResult) -> Result<()> {
        let dir = self.verification_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let json = serde_json::to_string_pretty(result)?;
        fs::write(self.verification_path(id), json)?;
        Ok(())
    }

    /// Read the latest verification result for a task.
    pub fn read_verification(&self, id: u64) -> Result<Option<VerificationResult>> {
        let path = self.verification_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        let result: VerificationResult = serde_json::from_str(&content)?;
        Ok(Some(result))
    }

    /// Delete the verification result file for a task, if it exists.
    fn delete_verification(&self, id: u64) -> Result<()> {
        let path = self.verification_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Delete the artifacts directory for a task, if it exists.
    fn delete_artifacts(&self, id: u64) -> Result<()> {
        let dir = self.artifacts_dir(id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Delete all sidecar files (context + history + verification + artifacts) for a task.
    pub fn delete(&self, id: u64) -> Result<()> {
        self.delete_context(id)?;
        self.delete_history(id)?;
        self.delete_verification(id)?;
        self.delete_artifacts(id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, SidecarStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(tak_root.join("context")).unwrap();
        fs::create_dir_all(tak_root.join("history")).unwrap();
        fs::create_dir_all(tak_root.join("verification_results")).unwrap();
        fs::create_dir_all(tak_root.join("artifacts")).unwrap();
        let store = SidecarStore::open(&tak_root);
        (dir, store)
    }

    #[test]
    fn context_write_and_read() {
        let (_dir, store) = setup();
        assert!(store.read_context(1).unwrap().is_none());
        store.write_context(1, "some notes").unwrap();
        assert_eq!(
            store.read_context(1).unwrap().as_deref(),
            Some("some notes")
        );
        // Overwrite
        store.write_context(1, "updated").unwrap();
        assert_eq!(store.read_context(1).unwrap().as_deref(), Some("updated"));
    }

    #[test]
    fn context_read_missing_returns_none() {
        let (_dir, store) = setup();
        assert!(store.read_context(999).unwrap().is_none());
    }

    #[test]
    fn history_append_and_read() {
        let (_dir, store) = setup();
        assert!(store.read_history(1).unwrap().is_empty());

        let evt1 = HistoryEvent {
            timestamp: Utc::now(),
            event: "started".into(),
            agent: Some("agent-1".into()),
            detail: serde_json::Map::new(),
        };
        store.append_history(1, &evt1).unwrap();

        let mut detail = serde_json::Map::new();
        detail.insert("reason".into(), serde_json::Value::String("done".into()));
        let evt2 = HistoryEvent {
            timestamp: Utc::now(),
            event: "finished".into(),
            agent: Some("agent-1".into()),
            detail,
        };
        store.append_history(1, &evt2).unwrap();

        let events = store.read_history(1).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "started");
        assert_eq!(events[0].agent.as_deref(), Some("agent-1"));
        assert!(events[0].detail.is_empty());
        assert_eq!(events[1].event, "finished");
        assert!(!events[1].detail.is_empty());
    }

    #[test]
    fn history_read_missing_returns_empty() {
        let (_dir, store) = setup();
        let events = store.read_history(999).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn verification_write_and_read() {
        let (_dir, store) = setup();
        assert!(store.read_verification(1).unwrap().is_none());

        let result = VerificationResult {
            timestamp: Utc::now(),
            results: vec![
                CommandResult {
                    command: "cargo test".into(),
                    exit_code: 0,
                    stdout: "ok".into(),
                    stderr: String::new(),
                    passed: true,
                },
                CommandResult {
                    command: "cargo clippy".into(),
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "warnings".into(),
                    passed: false,
                },
            ],
            passed: false,
        };
        store.write_verification(1, &result).unwrap();

        let read = store.read_verification(1).unwrap().unwrap();
        assert!(!read.passed);
        assert_eq!(read.results.len(), 2);
        assert!(read.results[0].passed);
        assert!(!read.results[1].passed);
        assert_eq!(read.results[0].command, "cargo test");
    }

    #[test]
    fn delete_all_cleans_up() {
        let (_dir, store) = setup();

        // Create all types of sidecar data
        store.write_context(1, "ctx").unwrap();
        let evt = HistoryEvent {
            timestamp: Utc::now(),
            event: "test".into(),
            agent: None,
            detail: serde_json::Map::new(),
        };
        store.append_history(1, &evt).unwrap();
        let vr = VerificationResult {
            timestamp: Utc::now(),
            results: vec![],
            passed: true,
        };
        store.write_verification(1, &vr).unwrap();

        // Create an artifacts dir with a file in it
        let artifacts = store.artifacts_dir(1);
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("output.txt"), "data").unwrap();

        assert!(store.context_path(1).exists());
        assert!(store.history_path(1).exists());
        assert!(store.verification_path(1).exists());
        assert!(store.artifacts_dir(1).exists());

        store.delete(1).unwrap();

        assert!(!store.context_path(1).exists());
        assert!(!store.history_path(1).exists());
        assert!(!store.verification_path(1).exists());
        assert!(!store.artifacts_dir(1).exists());

        // Deleting nonexistent is a no-op
        store.delete(999).unwrap();
    }
}
