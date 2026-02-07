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
    pub depends_on: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
        self.depends_on.sort();
        self.depends_on.dedup();
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
            depends_on: vec![2, 3],
            assignee: Some("agent-1".into()),
            tags: vec!["backend".into()],
            created_at: now,
            updated_at: now,
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
        };
        task.normalize();
        assert_eq!(task.tags, vec!["keep", "valid"]);
    }
}
