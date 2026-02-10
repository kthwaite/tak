use std::path::{Path, PathBuf};

use crate::error::{Result, TakError};
use crate::store::files::FileStore;
use crate::store::index::Index;
use crate::store::learnings::LearningStore;
use crate::store::sidecars::SidecarStore;
use crate::task_id::TaskId;

pub struct Repo {
    pub store: FileStore,
    pub index: Index,
    pub sidecars: SidecarStore,
    pub learnings: LearningStore,
}

impl Repo {
    /// Open an existing .tak repository, auto-rebuilding the index if stale or missing.
    pub fn open(repo_root: &Path) -> Result<Self> {
        let store = FileStore::open(repo_root)?;
        let index_path = store.root().join("index.db");
        let mut needs_rebuild = !index_path.exists();
        let mut index = Index::open(&index_path)?;

        // Auto-migrate legacy INTEGER-id index schema by recreating the derived index DB.
        if !index.uses_text_task_id_schema()? {
            drop(index);
            if index_path.exists() {
                std::fs::remove_file(&index_path)?;
            }
            index = Index::open(&index_path)?;
            needs_rebuild = true;
        }

        let current_fp = store.fingerprint()?;

        if !needs_rebuild {
            let stored_fp = index.get_fingerprint()?;
            needs_rebuild = stored_fp.as_deref() != Some(current_fp.as_str());
        }

        if needs_rebuild {
            let tasks = store.list_all()?;
            index.rebuild(&tasks)?;
        }

        index.set_fingerprint(&current_fp)?;

        let sidecars = SidecarStore::open(store.root());
        let learnings = LearningStore::open(store.root());

        // Rebuild learnings index if stale or missing
        let current_lfp = learnings.fingerprint()?;
        let stored_lfp = index.get_learning_fingerprint()?;
        if stored_lfp.as_deref() != Some(current_lfp.as_str()) {
            let all_learnings = learnings.list_all()?;
            index.rebuild_learnings(&all_learnings)?;
            index.set_learning_fingerprint(&current_lfp)?;
        }

        Ok(Self {
            store,
            index,
            sidecars,
            learnings,
        })
    }

    /// Resolve a user-supplied task ID to a canonical TaskId.
    ///
    /// Resolution strategy:
    /// 1) exact match (canonical hex or legacy decimal),
    /// 2) unique lowercase-hex prefix match,
    /// 3) otherwise return not-found/ambiguous/invalid errors.
    pub fn resolve_task_id(&self, input: &str) -> Result<TaskId> {
        let existing: Vec<TaskId> = self
            .store
            .list_ids()?
            .into_iter()
            .map(TaskId::from)
            .collect();
        resolve_task_id_input(input, &existing)
    }

    /// Convenience wrapper returning legacy numeric ID shape.
    pub fn resolve_task_id_u64(&self, input: &str) -> Result<u64> {
        Ok(self.resolve_task_id(input)?.into())
    }
}

/// Shared exact-or-prefix resolver for task ID inputs.
pub fn resolve_task_id_input(input: &str, existing_ids: &[TaskId]) -> Result<TaskId> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(TakError::InvalidTaskId(
            input.to_string(),
            "task id cannot be empty".into(),
        ));
    }

    let is_digits = raw.bytes().all(|b| b.is_ascii_digit());
    let is_hex = raw.bytes().all(|b| b.is_ascii_hexdigit());
    let is_full_hex = is_hex && raw.len() == TaskId::HEX_LEN;
    let can_try_prefix = is_hex && raw.len() < TaskId::HEX_LEN;

    // Exact first (canonical hex or legacy decimal forms)
    if is_digits || is_full_hex {
        let exact = TaskId::parse_cli(raw)
            .map_err(|e| TakError::InvalidTaskId(raw.to_string(), e.to_string()))?;
        if existing_ids.iter().any(|id| id == &exact) {
            return Ok(exact);
        }

        // If a unique prefix might still be intended (short all-hex), try that next.
        if !can_try_prefix {
            return Err(TakError::TaskIdNotFound(raw.to_string()));
        }
    }

    if !can_try_prefix {
        return Err(TakError::InvalidTaskId(
            raw.to_string(),
            format!(
                "expected legacy decimal id or 1-{} lowercase/uppercase hex characters",
                TaskId::HEX_LEN
            ),
        ));
    }

    let prefix = raw.to_ascii_lowercase();
    let mut matches: Vec<TaskId> = existing_ids
        .iter()
        .filter(|id| id.as_str().starts_with(&prefix))
        .cloned()
        .collect();
    matches.sort();
    matches.dedup();

    match matches.len() {
        0 => Err(TakError::TaskIdNotFound(raw.to_string())),
        1 => Ok(matches.remove(0)),
        _ => Err(TakError::TaskIdAmbiguous(
            raw.to_string(),
            format_task_id_matches(&matches),
        )),
    }
}

fn format_task_id_matches(ids: &[TaskId]) -> String {
    ids.iter()
        .map(|id| id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Walk up from current directory to find the .tak root.
pub fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().map_err(TakError::Io)?;
    loop {
        if dir.join(".tak").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(TakError::NotInitialized);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<TaskId> {
        values
            .iter()
            .map(|v| v.parse::<TaskId>().unwrap())
            .collect()
    }

    #[test]
    fn resolves_exact_hex_match() {
        let existing = ids(&["deadbeefcafefeed", "aaaaaaaaaaaaaaa1"]);
        let resolved = resolve_task_id_input("deadbeefcafefeed", &existing).unwrap();
        assert_eq!(resolved.as_str(), "deadbeefcafefeed");
    }

    #[test]
    fn resolves_exact_legacy_decimal_match() {
        let existing = vec![TaskId::from(42_u64)];
        let resolved = resolve_task_id_input("42", &existing).unwrap();
        assert_eq!(resolved, TaskId::from(42_u64));
    }

    #[test]
    fn resolves_unique_hex_prefix_match() {
        let existing = ids(&["deadbeef00000001", "cafebabe00000002"]);
        let resolved = resolve_task_id_input("dead", &existing).unwrap();
        assert_eq!(resolved.as_str(), "deadbeef00000001");
    }

    #[test]
    fn resolves_unique_hex_prefix_case_insensitively() {
        let existing = ids(&["deadbeef00000001", "cafebabe00000002"]);
        let resolved = resolve_task_id_input("DEAD", &existing).unwrap();
        assert_eq!(resolved.as_str(), "deadbeef00000001");
    }

    #[test]
    fn resolves_digits_as_prefix_when_exact_legacy_missing() {
        let existing = ids(&["1234abcd00000001"]);
        let resolved = resolve_task_id_input("1234", &existing).unwrap();
        assert_eq!(resolved.as_str(), "1234abcd00000001");
    }

    #[test]
    fn exact_match_wins_over_prefix_path() {
        let mut existing = ids(&["1000000000000000"]);
        existing.push(TaskId::from(1_u64));

        let resolved = resolve_task_id_input("1", &existing).unwrap();
        assert_eq!(resolved, TaskId::from(1_u64));
    }

    #[test]
    fn reports_ambiguous_prefix() {
        let existing = ids(&["abc0000000000001", "abcf000000000002"]);
        let err = resolve_task_id_input("abc", &existing).unwrap_err();

        assert!(matches!(err, TakError::TaskIdAmbiguous(_, _)));
        if let TakError::TaskIdAmbiguous(prefix, matches) = err {
            assert_eq!(prefix, "abc");
            assert!(matches.contains("abc0000000000001"));
            assert!(matches.contains("abcf000000000002"));
        }
    }

    #[test]
    fn ambiguous_prefix_lists_unique_sorted_matches() {
        let existing = vec![
            "abcf000000000002".parse::<TaskId>().unwrap(),
            "abc0000000000001".parse::<TaskId>().unwrap(),
            "abc0000000000001".parse::<TaskId>().unwrap(),
        ];

        let err = resolve_task_id_input("abc", &existing).unwrap_err();
        match err {
            TakError::TaskIdAmbiguous(prefix, matches) => {
                assert_eq!(prefix, "abc");
                assert_eq!(matches, "abc0000000000001, abcf000000000002");
            }
            other => panic!("expected TaskIdAmbiguous error, got {other:?}"),
        }
    }

    #[test]
    fn reports_not_found_for_missing_id_or_prefix() {
        let existing = ids(&["deadbeef00000001"]);
        let err = resolve_task_id_input("beef", &existing).unwrap_err();

        assert!(matches!(err, TakError::TaskIdNotFound(_)));
    }

    #[test]
    fn rejects_invalid_non_hex_prefix_input() {
        let existing = ids(&["deadbeef00000001"]);
        let err = resolve_task_id_input("bad-prefix", &existing).unwrap_err();

        assert!(matches!(err, TakError::InvalidTaskId(_, _)));
    }

    #[test]
    fn rejects_overlength_hex_input() {
        let existing = ids(&["deadbeef00000001"]);
        let err = resolve_task_id_input("deadbeef000000011", &existing).unwrap_err();

        assert!(matches!(err, TakError::InvalidTaskId(_, _)));
    }
}
