use serde::{Deserialize, Serialize};

/// Cross-channel linkage metadata used to tie together related mesh messages,
/// blackboard notes, and task history events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordinationLinks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mesh_message_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blackboard_note_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_event_ids: Vec<String>,
}

impl CoordinationLinks {
    pub fn is_empty(&self) -> bool {
        self.mesh_message_ids.is_empty()
            && self.blackboard_note_ids.is_empty()
            && self.history_event_ids.is_empty()
    }

    pub fn normalize(&mut self) {
        normalize_string_ids(&mut self.mesh_message_ids);
        normalize_string_ids(&mut self.history_event_ids);
        self.blackboard_note_ids.sort_unstable();
        self.blackboard_note_ids.dedup();
    }

    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

/// Build coordination links from free-form text by extracting:
/// - blackboard note references like `B123`
/// - UUID-like mesh message IDs
pub fn derive_links_from_text(text: &str) -> CoordinationLinks {
    CoordinationLinks {
        mesh_message_ids: extract_uuid_like_ids(text),
        blackboard_note_ids: extract_blackboard_note_ids(text),
        history_event_ids: vec![],
    }
}

/// Extract blackboard note references (`B123`) from free-form text.
pub fn extract_blackboard_note_ids(text: &str) -> Vec<u64> {
    let bytes = text.as_bytes();
    let mut ids = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch == 'B' || ch == 'b' {
            let prev_is_boundary = i == 0 || !(bytes[i - 1] as char).is_ascii_alphanumeric();
            if prev_is_boundary {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                    j += 1;
                }

                if j > i + 1 {
                    if let Ok(id_str) = std::str::from_utf8(&bytes[i + 1..j])
                        && let Ok(id) = id_str.parse::<u64>()
                    {
                        ids.push(id);
                    }
                    i = j;
                    continue;
                }
            }
        }

        i += 1;
    }

    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Extract UUID-like tokens (canonicalized to lowercase) from text.
pub fn extract_uuid_like_ids(text: &str) -> Vec<String> {
    let mut ids = text
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .filter_map(normalize_uuid_like)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn normalize_uuid_like(token: &str) -> Option<String> {
    if token.len() != 36 {
        return None;
    }

    for dash_index in [8, 13, 18, 23] {
        if token.as_bytes().get(dash_index).copied() != Some(b'-') {
            return None;
        }
    }

    for (idx, ch) in token.chars().enumerate() {
        let is_dash_slot = matches!(idx, 8 | 13 | 18 | 23);
        if is_dash_slot {
            if ch != '-' {
                return None;
            }
        } else if !ch.is_ascii_hexdigit() {
            return None;
        }
    }

    Some(token.to_ascii_lowercase())
}

fn normalize_string_ids(ids: &mut Vec<String>) {
    let mut normalized: Vec<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    *ids = normalized;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_sorts_and_deduplicates() {
        let mut links = CoordinationLinks {
            mesh_message_ids: vec![" msg-b ".into(), "msg-a".into(), "msg-a".into()],
            blackboard_note_ids: vec![7, 3, 7],
            history_event_ids: vec![" h2 ".into(), "h1".into(), "h2".into()],
        };

        links.normalize();

        assert_eq!(links.mesh_message_ids, vec!["msg-a", "msg-b"]);
        assert_eq!(links.blackboard_note_ids, vec![3, 7]);
        assert_eq!(links.history_event_ids, vec!["h1", "h2"]);
    }

    #[test]
    fn serde_omits_empty_fields() {
        let links = CoordinationLinks::default();
        let json = serde_json::to_string(&links).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn extract_blackboard_note_ids_finds_b_prefix_tokens() {
        let ids = extract_blackboard_note_ids("see B12, B7 and b42; ignore AB8 and Bx");
        assert_eq!(ids, vec![7, 12, 42]);
    }

    #[test]
    fn extract_uuid_like_ids_finds_and_normalizes_uuid_tokens() {
        let ids = extract_uuid_like_ids(
            "mesh=550E8400-E29B-41D4-A716-446655440000 bad=not-a-uuid history=550e8400-e29b-41d4-a716-446655440000",
        );
        assert_eq!(ids, vec!["550e8400-e29b-41d4-a716-446655440000"]);
    }

    #[test]
    fn derive_links_from_text_combines_extractors() {
        let links =
            derive_links_from_text("handoff refs: B8 mesh=550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(links.blackboard_note_ids, vec![8]);
        assert_eq!(
            links.mesh_message_ids,
            vec!["550e8400-e29b-41d4-a716-446655440000"]
        );
        assert!(links.history_event_ids.is_empty());
    }
}
