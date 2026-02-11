use std::path::{Path, PathBuf};

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::repo::{Repo, find_repo_root};
use crate::store::work::{WorkCoordinationVerbosity, WorkStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContext {
    repo_root: PathBuf,
}

impl RepoContext {
    pub fn discover() -> Result<Self> {
        let repo_root = find_repo_root()?;
        Ok(Self { repo_root })
    }

    pub fn from_root(repo_root: impl Into<PathBuf>) -> Result<Self> {
        let repo_root = repo_root.into();
        Repo::open(&repo_root)?;
        Ok(Self { repo_root })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn resolve_task_id(&self, input: impl AsRef<str>) -> Result<u64> {
        resolve_task_id_arg(&self.repo_root, input)
    }

    pub fn resolve_optional_task_id(&self, input: Option<String>) -> Result<Option<u64>> {
        resolve_optional_task_id_arg(&self.repo_root, input)
    }

    pub fn resolve_task_ids(&self, inputs: Vec<String>) -> Result<Vec<u64>> {
        resolve_task_id_args(&self.repo_root, inputs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessOutput {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub exit_code: i32,
}

impl ProcessOutput {
    pub fn success(stdout: Option<String>) -> Self {
        Self {
            stdout,
            stderr: None,
            exit_code: 0,
        }
    }

    pub fn error(error: &TakError, format: Format) -> Self {
        Self {
            stdout: None,
            stderr: Some(render_error_message(error, format)),
            exit_code: 1,
        }
    }
}

pub fn execute_with_context<F>(context: &RepoContext, run: F) -> Result<ProcessOutput>
where
    F: FnOnce(&Path) -> Result<()>,
{
    run(context.repo_root())?;
    Ok(ProcessOutput::success(None))
}

pub fn execute_with_discovered_repo<F>(run: F) -> Result<ProcessOutput>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let context = RepoContext::discover()?;
    execute_with_context(&context, run)
}

pub fn resolve_task_id_arg(repo_root: &Path, input: impl AsRef<str>) -> Result<u64> {
    let repo = Repo::open(repo_root)?;
    repo.resolve_task_id_u64(input.as_ref())
}

pub fn resolve_optional_task_id_arg(
    repo_root: &Path,
    input: Option<String>,
) -> Result<Option<u64>> {
    input
        .map(|id| resolve_task_id_arg(repo_root, id))
        .transpose()
}

pub fn resolve_task_id_args(repo_root: &Path, inputs: Vec<String>) -> Result<Vec<u64>> {
    inputs
        .into_iter()
        .map(|id| resolve_task_id_arg(repo_root, id))
        .collect()
}

pub fn resolve_effective_coordination_verbosity(
    repo_root: &Path,
    agent: Option<&str>,
    override_level: Option<WorkCoordinationVerbosity>,
) -> WorkCoordinationVerbosity {
    if let Some(level) = override_level {
        return level;
    }

    let Some(agent) = agent else {
        return WorkCoordinationVerbosity::default();
    };

    let store = WorkStore::open(&repo_root.join(".tak"));
    store
        .status(agent)
        .map(|state| state.coordination_verbosity)
        .unwrap_or_default()
}

pub fn apply_coordination_verbosity_label(
    message: &str,
    level: WorkCoordinationVerbosity,
    explicit_override: bool,
) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if !explicit_override && level == WorkCoordinationVerbosity::Medium {
        return trimmed.to_string();
    }

    format!("[verbosity={level}] {trimmed}")
}

pub fn maybe_add_verbosity_tag(
    tags: &mut Vec<String>,
    level: WorkCoordinationVerbosity,
    explicit_override: bool,
) {
    if !explicit_override && level == WorkCoordinationVerbosity::Medium {
        return;
    }

    tags.push(format!("verbosity-{level}"));
}

pub fn task_assignee_for_verbosity(repo_root: &Path, task_id: u64) -> Result<Option<String>> {
    let repo = Repo::open(repo_root)?;
    let task = repo.store.read(task_id)?;
    Ok(task.assignee)
}

pub fn render_error_message(error: &TakError, format: Format) -> String {
    match format {
        Format::Json => serde_json::json!({
            "error": error.code(),
            "message": error.to_string(),
        })
        .to_string(),
        _ => format!("error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tempfile::tempdir;

    use super::*;
    use crate::store::files::FileStore;

    #[test]
    fn apply_coordination_verbosity_label_skips_default_medium_without_override() {
        let rendered = apply_coordination_verbosity_label(
            "status update",
            WorkCoordinationVerbosity::Medium,
            false,
        );
        assert_eq!(rendered, "status update");
    }

    #[test]
    fn apply_coordination_verbosity_label_adds_marker_when_needed() {
        let rendered = apply_coordination_verbosity_label(
            "status update",
            WorkCoordinationVerbosity::High,
            false,
        );
        assert_eq!(rendered, "[verbosity=high] status update");
    }

    #[test]
    fn maybe_add_verbosity_tag_skips_default_medium_without_override() {
        let mut tags = vec!["coordination".to_string()];
        maybe_add_verbosity_tag(&mut tags, WorkCoordinationVerbosity::Medium, false);
        assert_eq!(tags, vec!["coordination"]);

        maybe_add_verbosity_tag(&mut tags, WorkCoordinationVerbosity::High, false);
        assert_eq!(tags, vec!["coordination", "verbosity-high"]);
    }

    #[test]
    fn render_error_message_json_includes_code_and_message() {
        let rendered = render_error_message(&TakError::NoAvailableTask, Format::Json);
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["error"], "no_available_task");
        assert_eq!(value["message"], "no available task to claim");
    }

    #[test]
    fn process_output_error_uses_exit_code_1() {
        let output = ProcessOutput::error(&TakError::NoAvailableTask, Format::Minimal);
        assert_eq!(output.exit_code, 1);
        assert!(output.stdout.is_none());
        assert_eq!(
            output.stderr.as_deref(),
            Some("error: no available task to claim")
        );
    }

    #[test]
    fn repo_context_from_root_validates_repository() {
        let dir = tempdir().unwrap();
        FileStore::init(dir.path()).unwrap();

        let context = RepoContext::from_root(dir.path()).unwrap();
        assert_eq!(context.repo_root(), dir.path());
    }

    #[test]
    fn execute_with_context_runs_closure() {
        let dir = tempdir().unwrap();
        FileStore::init(dir.path()).unwrap();
        let context = RepoContext::from_root(dir.path()).unwrap();

        let ran = Cell::new(false);
        let output = execute_with_context(&context, |_| {
            ran.set(true);
            Ok(())
        })
        .unwrap();

        assert!(ran.get());
        assert_eq!(output.exit_code, 0);
        assert!(output.stderr.is_none());
    }
}
