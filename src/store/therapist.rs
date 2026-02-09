use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::lock;

/// Therapist operating mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum TherapistMode {
    Offline,
    Online,
}

impl std::fmt::Display for TherapistMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => write!(f, "offline"),
            Self::Online => write!(f, "online"),
        }
    }
}

/// A single append-only therapist observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TherapistObservation {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub mode: TherapistMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interview: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metrics: serde_json::Map<String, serde_json::Value>,
}

/// Append-only storage for therapist observations under `.tak/therapist/`.
pub struct TherapistStore {
    root: PathBuf,
}

impl TherapistStore {
    pub fn open(tak_root: &Path) -> Self {
        Self {
            root: tak_root.join("therapist"),
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let log_path = self.log_path();
        if !log_path.exists() {
            fs::write(log_path, "")?;
        }
        Ok(())
    }

    pub fn append(&self, observation: &TherapistObservation) -> Result<()> {
        self.ensure_dirs()?;
        let lock = lock::acquire_lock(&self.lock_path())?;

        let mut line = serde_json::to_string(observation)?;
        line.push('\n');

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        file.write_all(line.as_bytes())?;

        lock::release_lock(lock)?;
        Ok(())
    }

    /// List observations, newest-first.
    pub fn list(&self, limit: Option<usize>) -> Result<Vec<TherapistObservation>> {
        let path = self.log_path();
        if !path.exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(path)?;
        let mut rows = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let row: TherapistObservation = serde_json::from_str(line)?;
            rows.push(row);
        }

        if let Some(limit) = limit {
            let len = rows.len();
            if len > limit {
                rows = rows.split_off(len - limit);
            }
        }

        rows.reverse();
        Ok(rows)
    }

    pub fn log_path_for_display(&self) -> PathBuf {
        self.log_path()
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("observations.jsonl")
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join("therapist.lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, TherapistStore) {
        let dir = tempdir().unwrap();
        let tak_root = dir.path().join(".tak");
        fs::create_dir_all(&tak_root).unwrap();
        let store = TherapistStore::open(&tak_root);
        (dir, store)
    }

    fn obs(id: &str, mode: TherapistMode) -> TherapistObservation {
        TherapistObservation {
            id: id.to_string(),
            timestamp: Utc::now(),
            mode,
            session: None,
            requested_by: Some("agent-1".into()),
            summary: format!("summary-{id}"),
            findings: vec!["finding".into()],
            recommendations: vec!["recommendation".into()],
            interview: None,
            metrics: serde_json::Map::new(),
        }
    }

    #[test]
    fn append_and_list_newest_first() {
        let (_dir, store) = setup();

        store.append(&obs("a", TherapistMode::Offline)).unwrap();
        store.append(&obs("b", TherapistMode::Online)).unwrap();

        let rows = store.list(None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "b");
        assert_eq!(rows[1].id, "a");
    }

    #[test]
    fn list_with_limit_returns_latest_entries() {
        let (_dir, store) = setup();

        store.append(&obs("1", TherapistMode::Offline)).unwrap();
        store.append(&obs("2", TherapistMode::Offline)).unwrap();
        store.append(&obs("3", TherapistMode::Online)).unwrap();

        let rows = store.list(Some(2)).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "3");
        assert_eq!(rows[1].id, "2");
    }

    #[test]
    fn ensure_dirs_creates_log_file() {
        let (_dir, store) = setup();
        store.ensure_dirs().unwrap();
        assert!(store.log_path().exists());
    }
}
