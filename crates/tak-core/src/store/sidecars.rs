use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::coordination::CoordinationLinks;
use crate::task_id::TaskId;

/// A single event in a task's history log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub detail: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "CoordinationLinks::is_empty")]
    pub links: CoordinationLinks,
}

impl HistoryEvent {
    fn ensure_normalized_ids(&mut self) {
        self.id = self
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| Some(uuid::Uuid::new_v4().to_string()));
        self.links.normalize();
    }
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SidecarMigrationReport {
    pub context_files_renamed: usize,
    pub history_files_renamed: usize,
    pub verification_files_renamed: usize,
    pub artifact_dirs_renamed: usize,
}

impl SidecarMigrationReport {
    pub fn total_renamed(&self) -> usize {
        self.context_files_renamed
            + self.history_files_renamed
            + self.verification_files_renamed
            + self.artifact_dirs_renamed
    }
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
    pub fn context_path(&self, id: &TaskId) -> PathBuf {
        self.context_dir().join(format!("{id}.md"))
    }

    /// Path to the history file for a given task ID.
    pub fn history_path(&self, id: &TaskId) -> PathBuf {
        self.history_dir().join(format!("{id}.jsonl"))
    }

    /// Path to the verification result file for a given task ID.
    pub fn verification_path(&self, id: &TaskId) -> PathBuf {
        self.verification_dir().join(format!("{id}.json"))
    }

    /// Path to the artifacts directory for a given task ID.
    pub fn artifacts_dir(&self, id: &TaskId) -> PathBuf {
        self.root.join("artifacts").join(id.as_str())
    }

    fn legacy_context_path(&self, id: u64) -> PathBuf {
        self.context_dir().join(format!("{id}.md"))
    }

    fn legacy_history_path(&self, id: u64) -> PathBuf {
        self.history_dir().join(format!("{id}.jsonl"))
    }

    fn legacy_verification_path(&self, id: u64) -> PathBuf {
        self.verification_dir().join(format!("{id}.json"))
    }

    fn legacy_artifacts_dir(&self, id: u64) -> PathBuf {
        self.root.join("artifacts").join(id.to_string())
    }

    pub fn migrate_task_paths(
        &self,
        id_map: &HashMap<u64, u64>,
        dry_run: bool,
    ) -> Result<SidecarMigrationReport> {
        let mut report = SidecarMigrationReport::default();
        if id_map.is_empty() {
            return Ok(report);
        }

        for (&old_id, &new_id) in id_map {
            let old_task_id = TaskId::from(old_id);
            let new_task_id = TaskId::from(new_id);

            if Self::migrate_path(
                &[
                    self.legacy_context_path(old_id),
                    self.context_path(&old_task_id),
                ],
                &self.context_path(&new_task_id),
                dry_run,
            )? {
                report.context_files_renamed += 1;
            }

            if Self::migrate_path(
                &[
                    self.legacy_history_path(old_id),
                    self.history_path(&old_task_id),
                ],
                &self.history_path(&new_task_id),
                dry_run,
            )? {
                report.history_files_renamed += 1;
            }

            if Self::migrate_path(
                &[
                    self.legacy_verification_path(old_id),
                    self.verification_path(&old_task_id),
                ],
                &self.verification_path(&new_task_id),
                dry_run,
            )? {
                report.verification_files_renamed += 1;
            }

            if Self::migrate_path(
                &[
                    self.legacy_artifacts_dir(old_id),
                    self.artifacts_dir(&old_task_id),
                ],
                &self.artifacts_dir(&new_task_id),
                dry_run,
            )? {
                report.artifact_dirs_renamed += 1;
            }
        }

        Ok(report)
    }

    fn migrate_path(candidates: &[PathBuf], destination: &Path, dry_run: bool) -> Result<bool> {
        let mut existing_sources: Vec<&PathBuf> = candidates
            .iter()
            .filter(|path| path.exists() && path.as_path() != destination)
            .collect();

        if existing_sources.len() > 1 {
            let paths = existing_sources
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(std::io::Error::other(format!(
                "multiple sidecar sources found for {}: {}",
                destination.display(),
                paths
            ))
            .into());
        }

        let Some(source) = existing_sources.pop() else {
            return Ok(false);
        };

        if destination.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "cannot migrate sidecar path '{}' -> '{}': destination exists",
                    source.display(),
                    destination.display()
                ),
            )
            .into());
        }

        if !dry_run {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(source, destination)?;
        }

        Ok(true)
    }

    /// Read the context notes for a task. Returns None if no context file exists.
    pub fn read_context(&self, id: u64) -> Result<Option<String>> {
        let task_id = TaskId::from(id);
        let path = self.context_path(&task_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        Ok(Some(content))
    }

    /// Write (overwrite) the context notes for a task.
    pub fn write_context(&self, id: u64, text: &str) -> Result<()> {
        let task_id = TaskId::from(id);
        let dir = self.context_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        fs::write(self.context_path(&task_id), text)?;
        Ok(())
    }

    /// Delete the context file for a task, if it exists.
    pub fn delete_context(&self, id: u64) -> Result<()> {
        let task_id = TaskId::from(id);
        let path = self.context_path(&task_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Append a structured history event to the JSONL log for a task.
    pub fn append_history(&self, id: u64, event: &HistoryEvent) -> Result<()> {
        let task_id = TaskId::from(id);
        let dir = self.history_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        let mut stored_event = event.clone();
        stored_event.ensure_normalized_ids();

        let mut line = serde_json::to_string(&stored_event)?;
        line.push('\n');
        let path = self.history_path(&task_id);

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
        let task_id = TaskId::from(id);
        let path = self.history_path(&task_id);
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
        let task_id = TaskId::from(id);
        let path = self.history_path(&task_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Write a verification result for a task.
    pub fn write_verification(&self, id: u64, result: &VerificationResult) -> Result<()> {
        let task_id = TaskId::from(id);
        let dir = self.verification_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let json = serde_json::to_string_pretty(result)?;
        fs::write(self.verification_path(&task_id), json)?;
        Ok(())
    }

    /// Read the latest verification result for a task.
    pub fn read_verification(&self, id: u64) -> Result<Option<VerificationResult>> {
        let task_id = TaskId::from(id);
        let path = self.verification_path(&task_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)?;
        let result: VerificationResult = serde_json::from_str(&content)?;
        Ok(Some(result))
    }

    /// Delete the verification result file for a task, if it exists.
    fn delete_verification(&self, id: u64) -> Result<()> {
        let task_id = TaskId::from(id);
        let path = self.verification_path(&task_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Delete the artifacts directory for a task, if it exists.
    fn delete_artifacts(&self, id: u64) -> Result<()> {
        let task_id = TaskId::from(id);
        let dir = self.artifacts_dir(&task_id);
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
    fn path_helpers_use_task_id_filenames() {
        let (_dir, store) = setup();
        let id = TaskId::from(42);

        assert_eq!(
            store
                .context_path(&id)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "000000000000002a.md"
        );
        assert_eq!(
            store
                .history_path(&id)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "000000000000002a.jsonl"
        );
        assert_eq!(
            store
                .verification_path(&id)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "000000000000002a.json"
        );
        assert_eq!(
            store
                .artifacts_dir(&id)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "000000000000002a"
        );
    }

    #[test]
    fn migrate_task_paths_apply_renames_all_sidecar_variants() {
        let (_dir, store) = setup();
        let old_id = 7_u64;
        let new_id = 42_u64;

        fs::write(store.legacy_context_path(old_id), "ctx").unwrap();
        fs::write(store.legacy_history_path(old_id), "{}\n").unwrap();
        fs::write(store.legacy_verification_path(old_id), "{}").unwrap();

        let legacy_artifacts = store.legacy_artifacts_dir(old_id);
        fs::create_dir_all(&legacy_artifacts).unwrap();
        fs::write(legacy_artifacts.join("artifact.txt"), "data").unwrap();

        let mut mapping = HashMap::new();
        mapping.insert(old_id, new_id);

        let report = store.migrate_task_paths(&mapping, false).unwrap();
        assert_eq!(report.context_files_renamed, 1);
        assert_eq!(report.history_files_renamed, 1);
        assert_eq!(report.verification_files_renamed, 1);
        assert_eq!(report.artifact_dirs_renamed, 1);
        assert_eq!(report.total_renamed(), 4);

        let old_task = TaskId::from(old_id);
        let new_task = TaskId::from(new_id);
        assert!(!store.legacy_context_path(old_id).exists());
        assert!(!store.legacy_history_path(old_id).exists());
        assert!(!store.legacy_verification_path(old_id).exists());
        assert!(!store.legacy_artifacts_dir(old_id).exists());

        assert!(store.context_path(&new_task).exists());
        assert!(store.history_path(&new_task).exists());
        assert!(store.verification_path(&new_task).exists());
        assert!(store.artifacts_dir(&new_task).exists());
        assert!(!store.context_path(&old_task).exists());
    }

    #[test]
    fn migrate_task_paths_dry_run_reports_without_renaming() {
        let (_dir, store) = setup();
        let old_id = 9_u64;
        let new_id = 10_u64;

        fs::write(store.legacy_context_path(old_id), "ctx").unwrap();

        let mut mapping = HashMap::new();
        mapping.insert(old_id, new_id);

        let report = store.migrate_task_paths(&mapping, true).unwrap();
        assert_eq!(report.context_files_renamed, 1);
        assert_eq!(report.total_renamed(), 1);

        assert!(store.legacy_context_path(old_id).exists());
        assert!(!store.context_path(&TaskId::from(new_id)).exists());
    }

    #[test]
    fn migrate_task_paths_fails_when_destination_exists() {
        let (_dir, store) = setup();
        let old_id = 11_u64;
        let new_id = 12_u64;

        fs::write(store.legacy_context_path(old_id), "old").unwrap();
        fs::write(store.context_path(&TaskId::from(new_id)), "new").unwrap();

        let mut mapping = HashMap::new();
        mapping.insert(old_id, new_id);

        let err = store.migrate_task_paths(&mapping, false).unwrap_err();
        assert!(
            err.to_string().contains("destination exists"),
            "unexpected error: {err}"
        );
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
            id: None,
            timestamp: Utc::now(),
            event: "started".into(),
            agent: Some("agent-1".into()),
            detail: serde_json::Map::new(),
            links: CoordinationLinks::default(),
        };
        store.append_history(1, &evt1).unwrap();

        let mut detail = serde_json::Map::new();
        detail.insert("reason".into(), serde_json::Value::String("done".into()));
        let evt2 = HistoryEvent {
            id: None,
            timestamp: Utc::now(),
            event: "finished".into(),
            agent: Some("agent-1".into()),
            detail,
            links: CoordinationLinks::default(),
        };
        store.append_history(1, &evt2).unwrap();

        let events = store.read_history(1).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "started");
        assert_eq!(events[0].agent.as_deref(), Some("agent-1"));
        assert!(events[0].id.is_some());
        assert!(events[0].detail.is_empty());
        assert_eq!(events[1].event, "finished");
        assert!(events[1].id.is_some());
        assert!(!events[1].detail.is_empty());
    }

    #[test]
    fn history_append_normalizes_cross_links() {
        let (_dir, store) = setup();

        let event = HistoryEvent {
            id: Some("  ".into()),
            timestamp: Utc::now(),
            event: "handoff".into(),
            agent: Some("agent-1".into()),
            detail: serde_json::Map::new(),
            links: CoordinationLinks {
                mesh_message_ids: vec![" m2 ".into(), "m1".into(), "m1".into()],
                blackboard_note_ids: vec![8, 3, 8],
                history_event_ids: vec![" h2 ".into(), "h1".into()],
            },
        };

        store.append_history(1, &event).unwrap();
        let events = store.read_history(1).unwrap();

        assert_eq!(events.len(), 1);
        assert!(events[0].id.as_deref().is_some_and(|id| !id.is_empty()));
        assert_eq!(events[0].links.mesh_message_ids, vec!["m1", "m2"]);
        assert_eq!(events[0].links.blackboard_note_ids, vec![3, 8]);
        assert_eq!(events[0].links.history_event_ids, vec!["h1", "h2"]);
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
            id: None,
            timestamp: Utc::now(),
            event: "test".into(),
            agent: None,
            detail: serde_json::Map::new(),
            links: CoordinationLinks::default(),
        };
        store.append_history(1, &evt).unwrap();
        let vr = VerificationResult {
            timestamp: Utc::now(),
            results: vec![],
            passed: true,
        };
        store.write_verification(1, &vr).unwrap();

        // Create an artifacts dir with a file in it
        let task_id = TaskId::from(1);
        let artifacts = store.artifacts_dir(&task_id);
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("output.txt"), "data").unwrap();

        assert!(store.context_path(&task_id).exists());
        assert!(store.history_path(&task_id).exists());
        assert!(store.verification_path(&task_id).exists());
        assert!(store.artifacts_dir(&task_id).exists());

        store.delete(1).unwrap();

        assert!(!store.context_path(&task_id).exists());
        assert!(!store.history_path(&task_id).exists());
        assert!(!store.verification_path(&task_id).exists());
        assert!(!store.artifacts_dir(&task_id).exists());

        // Deleting nonexistent is a no-op
        store.delete(999).unwrap();
    }
}
