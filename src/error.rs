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

    #[error("invalid task id '{0}': {1}")]
    InvalidTaskId(String, String),

    #[error("task id '{0}' not found")]
    TaskIdNotFound(String),

    #[error("task id '{0}' is ambiguous; matches: {1}")]
    TaskIdAmbiguous(String, String),

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

    #[error("mesh: multiple matching agents found ({0}); specify --name")]
    MeshAmbiguousAgent(String),

    #[error("mesh: agent name '{0}' is already registered")]
    MeshNameConflict(String),

    #[error("mesh: agent name must be non-empty ASCII alphanumeric/hyphen/underscore")]
    MeshInvalidName,

    #[error("mesh: invalid reservation path '{0}'")]
    MeshInvalidPath(String),

    #[error(
        "mesh: reservation conflict — requested '{requested_path}' overlaps held '{held_path}' by agent '{owner}' (reason: {reason}, age: {age_secs}s)"
    )]
    MeshReservationConflict {
        requested_path: String,
        held_path: String,
        owner: String,
        reason: String,
        age_secs: i64,
    },

    #[error("mesh: stale generation token (expected {expected}, got {got}) for agent '{agent}'")]
    MeshStaleGeneration {
        agent: String,
        expected: i64,
        got: i64,
    },

    #[error("blackboard: note {0} not found")]
    BlackboardNoteNotFound(u64),

    #[error("blackboard: agent name must be non-empty ASCII alphanumeric/hyphen/underscore")]
    BlackboardInvalidName,

    #[error("blackboard: message cannot be empty")]
    BlackboardInvalidMessage,

    #[error("therapist: session '{0}' not found")]
    TherapistSessionNotFound(String),

    #[error("therapist: session selector '{selector}' is ambiguous; matches: {matches}")]
    TherapistSessionAmbiguous { selector: String, matches: String },

    #[error("therapist rpc timeout: {0}")]
    TherapistRpcTimeout(String),

    #[error("therapist rpc protocol error: {0}")]
    TherapistRpcProtocol(String),

    #[error("wait: specify exactly one of --path or --on-task")]
    WaitInvalidTarget,

    #[error("wait timed out: {0}")]
    WaitTimeout(String),

    #[error("metrics: invalid query options: {0}")]
    MetricsInvalidQuery(String),

    #[error("work: invalid agent name '{0}' (expected ASCII alphanumeric/hyphen/underscore)")]
    WorkInvalidAgentName(String),

    #[error("work: corrupt file '{0}': {1}")]
    WorkCorruptFile(String, String),

    #[error("epic finish blocked by tak hygiene checks: {0}")]
    EpicFinishHygiene(String),

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
            Self::InvalidTaskId(_, _) => "invalid_task_id",
            Self::TaskIdNotFound(_) => "task_id_not_found",
            Self::TaskIdAmbiguous(_, _) => "task_id_ambiguous",
            Self::LearningNotFound(_) => "learning_not_found",
            Self::CycleDetected(_) => "cycle_detected",
            Self::InvalidTransition(_, _) => "invalid_transition",
            Self::NoAvailableTask => "no_available_task",
            Self::TaskBlocked(_) => "task_blocked",
            Self::TaskInUse(_) => "task_in_use",
            Self::Locked(_) => "locked",
            Self::MeshAgentNotFound(_) => "mesh_agent_not_found",
            Self::MeshAmbiguousAgent(_) => "mesh_ambiguous_agent",
            Self::MeshNameConflict(_) => "mesh_name_conflict",
            Self::MeshInvalidName => "mesh_invalid_name",
            Self::MeshInvalidPath(_) => "mesh_invalid_path",
            Self::MeshReservationConflict { .. } => "mesh_reservation_conflict",
            Self::MeshStaleGeneration { .. } => "mesh_stale_generation",
            Self::BlackboardNoteNotFound(_) => "blackboard_note_not_found",
            Self::BlackboardInvalidName => "blackboard_invalid_name",
            Self::BlackboardInvalidMessage => "blackboard_invalid_message",
            Self::TherapistSessionNotFound(_) => "therapist_session_not_found",
            Self::TherapistSessionAmbiguous { .. } => "therapist_session_ambiguous",
            Self::TherapistRpcTimeout(_) => "therapist_rpc_timeout",
            Self::TherapistRpcProtocol(_) => "therapist_rpc_protocol",
            Self::WaitInvalidTarget => "wait_invalid_target",
            Self::WaitTimeout(_) => "wait_timeout",
            Self::MetricsInvalidQuery(_) => "metrics_invalid_query",
            Self::WorkInvalidAgentName(_) => "work_invalid_agent_name",
            Self::WorkCorruptFile(_, _) => "work_corrupt_file",
            Self::EpicFinishHygiene(_) => "epic_finish_hygiene",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
            Self::Db(_) => "db_error",
        }
    }
}

pub type Result<T> = std::result::Result<T, TakError>;
