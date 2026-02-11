//! Canonical task-ID rendering helpers for JSON and text output.
//!
//! Consolidates task-ID formatting and JSON value rewriting so that every
//! command module emits canonical 16-hex IDs without duplicating logic.

use crate::error::Result;
use crate::model::{Learning, TRACE_ORIGIN_IDEA_ID_KEY, TRACE_REFINEMENT_TASK_IDS_KEY, Task};
use crate::task_id::TaskId;

/// Format a raw u64 task ID as a canonical 16-hex string.
pub(crate) fn format_task_id(id: u64) -> String {
    TaskId::from(id).to_string()
}

/// Try to parse/normalize a raw string token into canonical hex form.
/// Returns the original string unchanged if it doesn't parse as a valid task ID.
pub(crate) fn normalize_task_id_token(raw: &str) -> String {
    TaskId::parse_cli(raw)
        .map(|id| id.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// Extract a canonical task-ID string from a JSON value (number or string).
pub(crate) fn task_id_string_from_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(num) => num.as_u64().map(format_task_id),
        serde_json::Value::String(raw) => TaskId::parse_cli(raw).ok().map(|id| id.to_string()),
        _ => None,
    }
}

/// Rewrite a single JSON value in-place to its canonical hex form.
pub(crate) fn rewrite_task_id_value(value: &mut serde_json::Value) {
    if let Some(id) = task_id_string_from_json(value) {
        *value = serde_json::Value::String(id);
    }
}

/// Rewrite every element of a JSON array to canonical hex form.
pub(crate) fn rewrite_task_id_array(values: &mut [serde_json::Value]) {
    for value in values {
        rewrite_task_id_value(value);
    }
}

/// Rewrite all task-ID fields inside a serialized Task JSON value.
pub(crate) fn rewrite_task_json_value(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    if let Some(id) = obj.get_mut("id") {
        rewrite_task_id_value(id);
    }

    if let Some(parent) = obj.get_mut("parent") {
        rewrite_task_id_value(parent);
    }

    if let Some(depends_on) = obj.get_mut("depends_on").and_then(|v| v.as_array_mut()) {
        for dep in depends_on {
            if let Some(dep_obj) = dep.as_object_mut()
                && let Some(dep_id) = dep_obj.get_mut("id")
            {
                rewrite_task_id_value(dep_id);
            }
        }
    }

    if let Some(origin_idea_id) = obj.get_mut(TRACE_ORIGIN_IDEA_ID_KEY) {
        rewrite_task_id_value(origin_idea_id);
    }

    if let Some(refinement_task_ids) = obj
        .get_mut(TRACE_REFINEMENT_TASK_IDS_KEY)
        .and_then(|v| v.as_array_mut())
    {
        rewrite_task_id_array(refinement_task_ids);
    }
}

/// Serialize a Task to JSON with all task-ID fields in canonical hex form.
pub(crate) fn task_to_json_value(task: &Task) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(task)?;
    rewrite_task_json_value(&mut value);
    Ok(value)
}

/// Serialize a Learning to JSON with task_ids in canonical hex form.
pub(crate) fn learning_to_json_value(learning: &Learning) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(learning)?;

    if let Some(task_ids) = value
        .as_object_mut()
        .and_then(|obj| obj.get_mut("task_ids"))
        .and_then(|v| v.as_array_mut())
    {
        rewrite_task_id_array(task_ids);
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dependency, Kind, Status};
    use chrono::Utc;

    fn task(id: u64, parent: Option<u64>) -> Task {
        let now = Utc::now();
        Task {
            id,
            title: format!("task-{id}"),
            description: None,
            status: Status::Pending,
            kind: Kind::Task,
            parent,
            depends_on: vec![],
            assignee: None,
            tags: vec![],
            contract: crate::model::Contract::default(),
            planning: crate::model::Planning::default(),
            git: crate::model::GitInfo::default(),
            execution: crate::model::Execution::default(),
            learnings: vec![],
            created_at: now,
            updated_at: now,
            extensions: serde_json::Map::new(),
        }
    }

    #[test]
    fn format_task_id_uses_fixed_width_lower_hex() {
        assert_eq!(format_task_id(42), "000000000000002a");
    }

    #[test]
    fn rewrite_task_id_value_normalizes_legacy_decimal_strings() {
        let mut value = serde_json::json!("42");
        rewrite_task_id_value(&mut value);
        assert_eq!(value, serde_json::json!("000000000000002a"));
    }

    #[test]
    fn task_to_json_value_serializes_task_ids_as_canonical_hex() {
        let mut task = task(42, Some(1));
        task.depends_on = vec![Dependency::simple(255)];
        task.set_origin_idea_id(Some(7));
        task.set_refinement_task_ids(vec![10, 9]);

        let value = task_to_json_value(&task).unwrap();

        assert_eq!(value["id"], "000000000000002a");
        assert_eq!(value["parent"], "0000000000000001");
        assert_eq!(value["depends_on"][0]["id"], "00000000000000ff");
        assert_eq!(value["origin_idea_id"], "0000000000000007");
        assert_eq!(
            value["refinement_task_ids"],
            serde_json::json!(["0000000000000009", "000000000000000a"])
        );
    }

    #[test]
    fn normalize_task_id_token_round_trips_valid_ids() {
        assert_eq!(normalize_task_id_token("42"), "000000000000002a");
        assert_eq!(
            normalize_task_id_token("000000000000002a"),
            "000000000000002a"
        );
    }

    #[test]
    fn normalize_task_id_token_preserves_unparseable_strings() {
        assert_eq!(normalize_task_id_token("not-an-id"), "not-an-id");
    }

    #[test]
    fn learning_to_json_value_normalizes_task_ids() {
        let learning = crate::model::Learning {
            id: 1,
            title: "test".into(),
            description: None,
            category: crate::model::LearningCategory::Insight,
            tags: vec![],
            task_ids: vec![42, 255],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            extensions: serde_json::Map::new(),
        };

        let value = learning_to_json_value(&learning).unwrap();

        let task_ids = value["task_ids"].as_array().unwrap();
        assert_eq!(task_ids[0], "000000000000002a");
        assert_eq!(task_ids[1], "00000000000000ff");
    }
}
