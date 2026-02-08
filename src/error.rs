use thiserror::Error;

#[derive(Debug, Error)]
pub enum TakError {
    #[error("not a tak repository (run `tak init` first)")]
    NotInitialized,

    #[error("tak already initialized in this repository")]
    AlreadyInitialized,

    #[error("current directory is not a git repository (run from the repository root)")]
    NotGitRepository,

    #[error("task {0} not found")]
    TaskNotFound(u64),

    #[error("learning {0} not found")]
    LearningNotFound(u64),

    #[error("dependency cycle: task {0} would depend on itself (directly or transitively)")]
    CycleDetected(u64),

    #[error("invalid status transition: {0} -> {1}")]
    InvalidTransition(String, String),

    #[error("no available task to claim")]
    NoAvailableTask,

    #[error("task {0} is blocked by unfinished dependencies")]
    TaskBlocked(u64),

    #[error(
        "task {0} is referenced by other tasks; use --force to cascade or resolve references first"
    )]
    TaskInUse(u64),

    #[error("locked by another process: {0}")]
    Locked(String),

    #[error("mesh: agent '{0}' not found in registry")]
    MeshAgentNotFound(String),

    #[error("mesh: agent name '{0}' is already registered")]
    MeshNameConflict(String),

    #[error("mesh: agent name must be non-empty ASCII alphanumeric/hyphen/underscore")]
    MeshInvalidName,

    #[error("mesh: reservation conflict — path '{0}' is held by agent '{1}'")]
    MeshReservationConflict(String, String),

    #[error("mesh: corrupt file '{0}': {1}")]
    MeshCorruptFile(String, String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

impl TakError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::AlreadyInitialized => "already_initialized",
            Self::NotGitRepository => "not_git_repository",
            Self::TaskNotFound(_) => "task_not_found",
            Self::LearningNotFound(_) => "learning_not_found",
            Self::CycleDetected(_) => "cycle_detected",
            Self::InvalidTransition(_, _) => "invalid_transition",
            Self::NoAvailableTask => "no_available_task",
            Self::TaskBlocked(_) => "task_blocked",
            Self::TaskInUse(_) => "task_in_use",
            Self::Locked(_) => "locked",
            Self::MeshAgentNotFound(_) => "mesh_agent_not_found",
            Self::MeshNameConflict(_) => "mesh_name_conflict",
            Self::MeshInvalidName => "mesh_invalid_name",
            Self::MeshReservationConflict(_, _) => "mesh_reservation_conflict",
            Self::MeshCorruptFile(_, _) => "mesh_corrupt_file",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
            Self::Db(_) => "db_error",
        }
    }
}

pub type Result<T> = std::result::Result<T, TakError>;
