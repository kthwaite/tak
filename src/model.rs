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
}
