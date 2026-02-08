use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Pending,
    InProgress,
    Done,
    Cancelled,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Kind {
    Epic,
    #[default]
    Task,
    Bug,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DepType {
    #[default]
    Hard,
    Soft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dep_type: Option<DepType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Dependency {
    /// Create a simple dependency with no type or reason.
    pub fn simple(id: u64) -> Self {
        Self {
            id,
            dep_type: None,
            reason: None,
        }
    }
}

impl std::fmt::Display for DepType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hard => write!(f, "hard"),
            Self::Soft => write!(f, "soft"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Contract {
    /// One-sentence outcome definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,

    /// Checklist of acceptance criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,

    /// Commands to verify the task is done.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<String>,

    /// Constraints the implementer must respect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
}

impl Contract {
    pub fn is_empty(&self) -> bool {
        self.objective.is_none()
            && self.acceptance_criteria.is_empty()
            && self.verification.is_empty()
            && self.constraints.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

impl Priority {
    /// Numeric rank for SQL ordering. Lower = higher priority.
    pub fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Estimate {
    Xs,
    S,
    M,
    L,
    Xl,
}

impl std::fmt::Display for Estimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xs => write!(f, "xs"),
            Self::S => write!(f, "s"),
            Self::M => write!(f, "m"),
            Self::L => write!(f, "l"),
            Self::Xl => write!(f, "xl"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for Risk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Planning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skills: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<Risk>,
}

impl Planning {
    pub fn is_empty(&self) -> bool {
        self.priority.is_none()
            && self.estimate.is_none()
            && self.required_skills.is_empty()
            && self.risk.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GitInfo {
    /// Branch name when the task was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Commit SHA at `tak start` time (the baseline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_commit: Option<String>,

    /// Commit SHA at `tak finish` time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_commit: Option<String>,

    /// Commits between start_commit and end_commit (populated by `tak finish`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,

    /// Pull request URL (set via `tak edit --pr`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
}

impl GitInfo {
    pub fn is_empty(&self) -> bool {
        self.branch.is_none()
            && self.start_commit.is_none()
            && self.end_commit.is_none()
            && self.commits.is_empty()
            && self.pr.is_none()
    }
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Execution {
    /// How many times this task has been started (attempt_count).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt_count: u32,

    /// Error or reason from the last cancellation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    /// Summary left by a previous assignee when handing off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_summary: Option<String>,

    /// Why this task is blocked (human-supplied context, not derived).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

impl Execution {
    pub fn is_empty(&self) -> bool {
        self.attempt_count == 0
            && self.last_error.is_none()
            && self.handoff_summary.is_none()
            && self.blocked_reason.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: Status,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<Dependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Contract::is_empty")]
    pub contract: Contract,
    #[serde(default, skip_serializing_if = "Planning::is_empty")]
    pub planning: Planning,
    #[serde(default, skip_serializing_if = "GitInfo::is_empty")]
    pub git: GitInfo,
    #[serde(default, skip_serializing_if = "Execution::is_empty")]
    pub execution: Execution,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Preserve unknown fields for forward compatibility.
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Done => write!(f, "done"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Epic => write!(f, "epic"),
            Self::Task => write!(f, "task"),
            Self::Bug => write!(f, "bug"),
        }
    }
}

impl Task {
    /// Trim whitespace, drop empty tags, then deduplicate and sort for deterministic storage.
    pub fn normalize(&mut self) {
        for tag in &mut self.tags {
            let trimmed = tag.trim();
            if trimmed.len() != tag.len() {
                *tag = trimmed.to_string();
            }
        }
        self.tags.retain(|t| !t.is_empty());
        self.depends_on.sort_by_key(|d| d.id);
        self.depends_on.dedup_by_key(|d| d.id);
        self.tags.sort();
        self.tags.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn task_round_trips_json() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Test task".into(),
            description: Some("A description".into()),
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![Dependency::simple(2), Dependency::simple(3)],
            assignee: Some("agent-1".into()),
            tags: vec!["backend".into()],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };

        let json = serde_json::to_string_pretty(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task, parsed);
    }

    #[test]
    fn status_serializes_snake_case() {
        let json = serde_json::to_string(&Status::InProgress).unwrap();
        assert_eq!(json, r#""in_progress""#);
    }

    #[test]
    fn minimal_task_omits_optional_fields() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Minimal".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };

        let json = serde_json::to_string(&task).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("parent"));
        assert!(!json.contains("depends_on"));
        assert!(!json.contains("assignee"));
        assert!(!json.contains("tags"));
    }

    #[test]
    fn normalize_trims_and_drops_empty_tags() {
        let now = Utc::now();
        let mut task = Task {
            id: 1,
            title: "Test".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![
                "".into(),
                " ".into(),
                "  valid  ".into(),
                "keep".into(),
                "keep".into(),
            ],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        task.normalize();
        assert_eq!(task.tags, vec!["keep", "valid"]);
    }

    #[test]
    fn dependency_serializes_minimal() {
        let dep = Dependency {
            id: 1,
            dep_type: None,
            reason: None,
        };
        let json = serde_json::to_string(&dep).unwrap();
        assert_eq!(json, r#"{"id":1}"#);
    }

    #[test]
    fn dependency_serializes_full() {
        let dep = Dependency {
            id: 1,
            dep_type: Some(DepType::Soft),
            reason: Some("nice to have".into()),
        };
        let json = serde_json::to_string(&dep).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["dep_type"], "soft");
        assert_eq!(parsed["reason"], "nice to have");
    }

    #[test]
    fn dependency_round_trips() {
        let dep = Dependency {
            id: 42,
            dep_type: Some(DepType::Hard),
            reason: None,
        };
        let json = serde_json::to_string(&dep).unwrap();
        let parsed: Dependency = serde_json::from_str(&json).unwrap();
        assert_eq!(dep, parsed);
    }

    #[test]
    fn task_preserves_unknown_fields() {
        let json = r#"{
            "id": 1,
            "title": "Test",
            "status": "pending",
            "kind": "task",
            "depends_on": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "custom_field": "preserved",
            "nested": {"key": "value"}
        }"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.extensions.get("custom_field").unwrap(), "preserved");

        // Round-trip: unknown fields survive
        let serialized = serde_json::to_string(&task).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reparsed["custom_field"], "preserved");
        assert_eq!(reparsed["nested"]["key"], "value");
    }

    #[test]
    fn contract_round_trips() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Contracted".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: Contract {
                objective: Some("Ship the widget".into()),
                acceptance_criteria: vec!["Tests pass".into(), "No warnings".into()],
                verification: vec!["cargo test".into(), "cargo clippy".into()],
                constraints: vec!["No unsafe".into()],
            },
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        let json = serde_json::to_string_pretty(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task, parsed);
        assert_eq!(
            parsed.contract.objective.as_deref(),
            Some("Ship the widget")
        );
        assert_eq!(parsed.contract.verification.len(), 2);
    }

    #[test]
    fn empty_contract_omitted_from_json() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Plain task".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&task).unwrap();
        assert!(
            !json.contains("contract"),
            "empty contract should not appear in JSON"
        );
        assert!(!json.contains("objective"));
        assert!(!json.contains("acceptance_criteria"));
        assert!(!json.contains("verification"));
        assert!(!json.contains("constraints"));
    }

    #[test]
    fn planning_round_trips() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Planned".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning {
                priority: Some(Priority::High),
                estimate: Some(Estimate::M),
                required_skills: vec!["rust".into(), "sql".into()],
                risk: Some(Risk::Low),
            },
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        let json = serde_json::to_string_pretty(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task, parsed);
        assert_eq!(parsed.planning.priority, Some(Priority::High));
        assert_eq!(parsed.planning.estimate, Some(Estimate::M));
        assert_eq!(parsed.planning.risk, Some(Risk::Low));
        assert_eq!(parsed.planning.required_skills, vec!["rust", "sql"]);
    }

    #[test]
    fn empty_planning_omitted_from_json() {
        let now = Utc::now();
        let task = Task {
            id: 1,
            title: "Bare task".into(),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent: None,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&task).unwrap();
        assert!(
            !json.contains("planning"),
            "empty planning should not appear in JSON"
        );
        assert!(!json.contains("\"priority\""));
        assert!(!json.contains("\"estimate\""));
        assert!(!json.contains("required_skills"));
        assert!(!json.contains("\"risk\""));
    }

    #[test]
    fn priority_rank_ordering() {
        assert!(Priority::Critical.rank() < Priority::High.rank());
        assert!(Priority::High.rank() < Priority::Medium.rank());
        assert!(Priority::Medium.rank() < Priority::Low.rank());
    }
}
