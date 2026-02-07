use thiserror::Error;

#[derive(Debug, Error)]
pub enum TakError {
    #[error("not a tak repository (run `tak init` first)")]
    NotInitialized,

    #[error("tak already initialized in this repository")]
    AlreadyInitialized,

    #[error("task {0} not found")]
    TaskNotFound(u64),

    #[error("task {0} already exists")]
    TaskAlreadyExists(u64),

    #[error("dependency cycle: task {0} would depend on itself (directly or transitively)")]
    CycleDetected(u64),

    #[error("invalid status transition: {0} -> {1}")]
    InvalidTransition(String, String),

    #[error("no available task to claim")]
    NoAvailableTask,

    #[error("locked by another process: {0}")]
    Locked(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, TakError>;
