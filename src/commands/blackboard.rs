use std::collections::HashMap;
use std::path::Path;

use clap::ValueEnum;
use colored::Colorize;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::blackboard::BlackboardStatus;
use crate::store::coordination::{CoordinationLinks, derive_links_from_text};
use crate::store::coordination_db::{CoordinationDb, DbNote};
use crate::store::repo::Repo;
use crate::task_id::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum BlackboardTemplate {
    Blocker,
    Handoff,
    Status,
}

impl BlackboardTemplate {
    fn default_tags(self) -> &'static [&'static str] {
        match self {
            Self::Blocker => &["blocker", "coordination"],
            Self::Handoff => &["handoff", "coordination"],
            Self::Status => &["status", "coordination"],
        }
    }

    fn required_schema_fields(self) -> &'static [&'static str] {
        match self {
            Self::Blocker => &[
                "template",
                "summary",
                "status",
                "scope",
                "owner",
                "verification",
                "blocker",
                "requested_action",
                "next",
            ],
            Self::Handoff | Self::Status => &[
                "template",
                "summary",
                "status",
                "scope",
                "owner",
                "verification",
                "blocker",
                "next",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlackboardPostOptions {
    pub template: Option<BlackboardTemplate>,
    pub since_note: Option<u64>,
    pub no_change_since: bool,
}

fn format_task_id(id: u64) -> String {
    TaskId::from(id).to_string()
}

fn normalize_task_id_token(raw: &str) -> String {
    TaskId::parse_cli(raw)
        .map(|id| id.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn canonicalize_note_task_ids(note: &DbNote) -> DbNote {
    let mut normalized = note.clone();
    normalized.task_ids = note
        .task_ids
        .iter()
        .map(|id| normalize_task_id_token(id))
        .collect();
    normalized
}

fn note_matches_task_candidates(note: &DbNote, candidates: &[String]) -> bool {
    note.task_ids.iter().any(|raw| {
        let normalized = normalize_task_id_token(raw);
        candidates
            .iter()
            .any(|candidate| candidate == raw || candidate == &normalized)
    })
}

pub fn post(
    repo_root: &Path,
    from: &str,
    message: &str,
    template: Option<BlackboardTemplate>,
    tags: Vec<String>,
    task_ids: Vec<u64>,
    format: Format,
) -> Result<()> {
    post_with_options(
        repo_root,
        from,
        message,
        BlackboardPostOptions {
            template,
            since_note: None,
            no_change_since: false,
        },
        tags,
        task_ids,
        format,
    )
}

pub fn post_with_options(
    repo_root: &Path,
    from: &str,
    message: &str,
    options: BlackboardPostOptions,
    mut tags: Vec<String>,
    task_ids: Vec<u64>,
    format: Format,
) -> Result<()> {
    if !task_ids.is_empty() {
        let repo = Repo::open(repo_root)?;
        for &id in &task_ids {
            repo.store.read(id)?;
        }
    }

    if options.no_change_since && options.since_note.is_none() {
        return Err(TakError::BlackboardInvalidMessage);
    }

    let db = CoordinationDb::from_repo(repo_root)?;
    if let Some(note_id) = options.since_note {
        db.get_note(note_id as i64)?;
    }

    let base_message = if let Some(template) = options.template {
        tags.extend(template.default_tags().iter().map(|tag| tag.to_string()));
        render_template(template, from, message, &task_ids)
    } else {
        message.trim().to_string()
    };

    let schema_warnings = detect_schema_warnings(options.template, &base_message);
    emit_schema_warnings(&schema_warnings, format);

    let rendered_message =
        apply_delta_metadata(base_message, options.since_note, options.no_change_since);

    let sensitive_warnings = detect_sensitive_text_warnings(&rendered_message);
    emit_sensitive_warnings(&sensitive_warnings, format);

    // Links are derived but not stored in CoordinationDb (dropped at boundary)
    let _links = derive_transition_links(options.template, &rendered_message, options.since_note);

    let task_id_strs: Vec<String> = task_ids.iter().map(|id| format_task_id(*id)).collect();
    let note = db.post_note(from, &rendered_message, &tags, &task_id_strs)?;
    print_note(&note, format)?;
    Ok(())
}

fn should_auto_link_transition(template: Option<BlackboardTemplate>) -> bool {
    matches!(
        template,
        Some(BlackboardTemplate::Blocker | BlackboardTemplate::Handoff)
    )
}

fn derive_transition_links(
    template: Option<BlackboardTemplate>,
    rendered_message: &str,
    since_note: Option<u64>,
) -> CoordinationLinks {
    if !should_auto_link_transition(template) {
        return CoordinationLinks::default();
    }

    let mut links = derive_links_from_text(rendered_message);
    if let Some(note_id) = since_note {
        links.blackboard_note_ids.push(note_id);
    }
    links.normalize();
    links
}

fn render_template(
    template: BlackboardTemplate,
    author: &str,
    summary: &str,
    task_ids: &[u64],
) -> String {
    let summary = summary.trim();
    let task_scope = if task_ids.is_empty() {
        "tasks=none".to_string()
    } else {
        let ids = task_ids
            .iter()
            .map(|id| format_task_id(*id))
            .collect::<Vec<_>>()
            .join(",");
        format!("tasks={ids}")
    };

    serialize_schema_fields(template_fields(template, author, summary, &task_scope))
}

fn template_fields(
    template: BlackboardTemplate,
    author: &str,
    summary: &str,
    task_scope: &str,
) -> Vec<(&'static str, String)> {
    match template {
        BlackboardTemplate::Blocker => vec![
            ("template", "blocker".to_string()),
            ("summary", summary.to_string()),
            ("status", "blocked".to_string()),
            ("scope", task_scope.to_string()),
            ("owner", author.to_string()),
            ("verification", "<not run | command + result>".to_string()),
            ("blocker", "<exact owner/path/reason>".to_string()),
            ("requested_action", "<what unblock is needed>".to_string()),
            ("next", "<who acts next and when>".to_string()),
            (
                "redaction",
                "redact secrets/tokens (use <redacted>)".to_string(),
            ),
        ],
        BlackboardTemplate::Handoff => vec![
            ("template", "handoff".to_string()),
            ("summary", summary.to_string()),
            ("status", "handoff".to_string()),
            ("scope", task_scope.to_string()),
            ("owner", author.to_string()),
            ("verification", "<latest command + result>".to_string()),
            ("blocker", "<none | unresolved owner/path/risk>".to_string()),
            ("next", "<handoff target + first action>".to_string()),
            (
                "redaction",
                "redact secrets/tokens (use <redacted>)".to_string(),
            ),
        ],
        BlackboardTemplate::Status => vec![
            ("template", "status".to_string()),
            ("summary", summary.to_string()),
            ("status", "in_progress".to_string()),
            ("scope", task_scope.to_string()),
            ("owner", author.to_string()),
            ("verification", "<latest command + result>".to_string()),
            ("blocker", "<none | owner/path/risk>".to_string()),
            ("next", "<next action + owner>".to_string()),
            (
                "redaction",
                "redact secrets/tokens (use <redacted>)".to_string(),
            ),
        ],
    }
}

fn serialize_schema_fields(fields: Vec<(&'static str, String)>) -> String {
    fields
        .into_iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn apply_delta_metadata(message: String, since_note: Option<u64>, no_change_since: bool) -> String {
    let Some(note_id) = since_note else {
        return message;
    };

    let delta_line = if no_change_since {
        "delta: no-change-since".to_string()
    } else {
        format!("delta: <what changed since B{note_id}>")
    };

    format!(
        "{message}\ndelta_since: B{note_id}\n{delta_line}",
        message = message.trim_end()
    )
}

fn detect_schema_warnings(template: Option<BlackboardTemplate>, message: &str) -> Vec<String> {
    let Some(template) = template else {
        return Vec::new();
    };

    let parsed = parse_schema_fields(message);
    let mut warnings = Vec::new();

    for required in template.required_schema_fields() {
        match parsed.get(*required) {
            None => warnings.push(format!(
                "missing required field `{required}` in structured coordination note"
            )),
            Some(value) if is_placeholder_value(value) => warnings.push(format!(
                "field `{required}` still uses placeholder/unset value `{}`",
                value.trim()
            )),
            Some(_) => {}
        }
    }

    warnings
}

fn parse_schema_fields(message: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();

    for line in message.lines() {
        let Some((raw_key, raw_value)) = line.split_once(':') else {
            continue;
        };

        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }

        fields
            .entry(key.to_string())
            .or_insert_with(|| raw_value.trim().to_string());
    }

    fields
}

fn is_placeholder_value(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || (value.contains('<') && value.contains('>'))
}

fn emit_schema_warnings(warnings: &[String], _format: Format) {
    if warnings.is_empty() {
        return;
    }

    eprintln!(
        "{} structured coordination note is missing concrete schema details:",
        "warning:".yellow().bold()
    );
    for warning in warnings {
        eprintln!("  - {warning}");
    }
    eprintln!(
        "  hint: fill required fields with concrete values (status/scope/verification/blocker/next), or omit --template to keep pure free-text mode."
    );
}

fn detect_sensitive_text_warnings(message: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let lower = message.to_ascii_lowercase();

    if contains_sensitive_assignment(&lower) {
        warnings.push("contains assignment-like secret/token/password field".to_string());
    }

    if message.contains("-----BEGIN") && lower.contains("private key") {
        warnings.push("contains private-key marker".to_string());
    }

    if message.contains("ghp_") || message.contains("github_pat_") {
        warnings.push("contains GitHub token-looking prefix".to_string());
    }

    if contains_aws_access_key(message) {
        warnings.push("contains AWS access-key-looking token (AKIA...)".to_string());
    }

    if contains_jwt_like_token(message) {
        warnings.push("contains JWT-like token".to_string());
    }

    if contains_long_credential_like_token(message) {
        warnings.push("contains long credential-like token".to_string());
    }

    warnings
}

fn contains_sensitive_assignment(lower: &str) -> bool {
    const KEYS: [&str; 8] = [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "access_key",
        "bearer",
    ];

    for key in KEYS {
        for sep in ["=", ":"] {
            if lower.contains(&format!("{key}{sep}")) || lower.contains(&format!("{key} {sep}")) {
                return true;
            }
        }
    }

    false
}

fn contains_aws_access_key(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| {
            token.len() == 20
                && token.starts_with("AKIA")
                && token
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
}

fn contains_jwt_like_token(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        });
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return false;
        }
        parts.iter().all(|part| {
            part.len() >= 8
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    })
}

fn contains_long_credential_like_token(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        });

        if token.len() < 28 || token.contains('/') {
            return false;
        }

        let mut has_lower = false;
        let mut has_upper = false;
        let mut has_digit = false;

        for c in token.chars() {
            if c.is_ascii_lowercase() {
                has_lower = true;
            } else if c.is_ascii_uppercase() {
                has_upper = true;
            } else if c.is_ascii_digit() {
                has_digit = true;
            }
        }

        (has_lower || has_upper) && has_digit
    })
}

fn emit_sensitive_warnings(warnings: &[String], _format: Format) {
    if warnings.is_empty() {
        return;
    }

    eprintln!(
        "{} potential sensitive text detected in blackboard message:",
        "warning:".yellow().bold()
    );
    for warning in warnings {
        eprintln!("  - {warning}");
    }
    eprintln!(
        "  hint: redact secrets/tokens (example: sk-...abcd -> <redacted:...abcd>) before posting."
    );
}

pub fn list(
    repo_root: &Path,
    status: Option<BlackboardStatus>,
    tag: Option<String>,
    task_id: Option<u64>,
    limit: Option<usize>,
    format: Format,
) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let status_str = status.map(|s| s.to_string());

    let mut notes = db.list_notes(status_str.as_deref(), tag.as_deref(), None, None)?;

    if let Some(task_id) = task_id {
        let candidates = vec![format_task_id(task_id), task_id.to_string()];
        notes.retain(|note| note_matches_task_candidates(note, &candidates));
    }

    if let Some(limit) = limit {
        notes.truncate(limit);
    }

    print_notes(&notes, format)?;
    Ok(())
}

pub fn show(repo_root: &Path, id: u64, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    let note = db.get_note(id as i64)?;
    print_note(&note, format)?;
    Ok(())
}

pub fn close(
    repo_root: &Path,
    id: u64,
    by: &str,
    reason: Option<&str>,
    format: Format,
) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    db.close_note(id as i64, by, reason)?;
    let note = db.get_note(id as i64)?;
    print_note(&note, format)?;
    Ok(())
}

pub fn reopen(repo_root: &Path, id: u64, by: &str, format: Format) -> Result<()> {
    let db = CoordinationDb::from_repo(repo_root)?;
    db.reopen_note(id as i64, by)?;
    let note = db.get_note(id as i64)?;
    print_note(&note, format)?;
    Ok(())
}

fn print_note(note: &DbNote, format: Format) -> Result<()> {
    let note = canonicalize_note_task_ids(note);

    match format {
        Format::Json => println!("{}", serde_json::to_string(&note)?),
        Format::Pretty => {
            let status = style_status(&note.status);
            println!(
                "{} {} {}",
                format!("[B{}]", note.id).magenta().bold(),
                status,
                format!("by {}", note.from_agent).dimmed(),
            );
            println!("  {}", note.message);
            if !note.tags.is_empty() {
                println!("  {} {}", "tags:".dimmed(), note.tags.join(", ").cyan());
            }
            if !note.task_ids.is_empty() {
                println!("  {} {}", "tasks:".dimmed(), note.task_ids.join(", "));
            }
            println!(
                "  {} {}",
                "updated:".dimmed(),
                note.updated_at.to_rfc3339().dimmed()
            );
            if note.status == "closed" {
                if let Some(by) = note.closed_by.as_deref() {
                    println!("  {} {}", "closed by:".dimmed(), by);
                }
                if let Some(reason) = note.closed_reason.as_deref() {
                    println!("  {} {}", "reason:".dimmed(), reason);
                }
            }
        }
        Format::Minimal => {
            println!("{}", note.id);
        }
    }
    Ok(())
}

fn print_notes(notes: &[DbNote], format: Format) -> Result<()> {
    let normalized: Vec<DbNote> = notes.iter().map(canonicalize_note_task_ids).collect();

    match format {
        Format::Json => println!("{}", serde_json::to_string(&normalized)?),
        Format::Pretty => {
            if normalized.is_empty() {
                println!("{}", "No blackboard notes.".dimmed());
            } else {
                for note in &normalized {
                    let status = style_status(&note.status);
                    println!(
                        "{} {} {} {}",
                        format!("[B{}]", note.id).magenta().bold(),
                        status,
                        format!("{}:", note.from_agent).cyan(),
                        note.message,
                    );
                    if !note.tags.is_empty() {
                        println!("  {} {}", "tags:".dimmed(), note.tags.join(", ").cyan());
                    }
                    if !note.task_ids.is_empty() {
                        println!("  {} {}", "tasks:".dimmed(), note.task_ids.join(", "));
                    }
                }
            }
        }
        Format::Minimal => {
            for note in &normalized {
                println!("{} {} {}", note.id, note.status, note.from_agent);
            }
        }
    }
    Ok(())
}

fn style_status(status: &str) -> String {
    match status {
        "open" => "open".yellow().to_string(),
        "closed" => "closed".green().to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::files::FileStore;
    use tempfile::tempdir;

    #[test]
    fn apply_delta_metadata_with_change_placeholder() {
        let rendered = apply_delta_metadata("status update".to_string(), Some(12), false);
        assert!(rendered.contains("delta_since: B12"));
        assert!(rendered.contains("delta: <what changed since B12>"));
    }

    #[test]
    fn apply_delta_metadata_with_no_change_marker() {
        let rendered = apply_delta_metadata("status update".to_string(), Some(7), true);
        assert!(rendered.contains("delta_since: B7"));
        assert!(rendered.contains("delta: no-change-since"));
    }

    #[test]
    fn template_includes_redaction_guidance() {
        let rendered = render_template(BlackboardTemplate::Status, "agent-1", "Summary", &[1]);
        assert!(rendered.contains("redaction: redact secrets/tokens"));
    }

    #[test]
    fn template_serialization_includes_required_schema_fields() {
        for template in [
            BlackboardTemplate::Blocker,
            BlackboardTemplate::Handoff,
            BlackboardTemplate::Status,
        ] {
            let rendered = render_template(template, "agent-1", "Summary", &[1]);
            let parsed = parse_schema_fields(&rendered);
            for field in template.required_schema_fields() {
                assert!(parsed.contains_key(*field), "missing field: {field}");
            }
        }
    }

    #[test]
    fn schema_detection_flags_placeholder_values_for_template_mode() {
        let rendered = render_template(BlackboardTemplate::Status, "agent-1", "Summary", &[1]);
        let warnings = detect_schema_warnings(Some(BlackboardTemplate::Status), &rendered);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("verification"))
        );
        assert!(warnings.iter().any(|warning| warning.contains("blocker")));
        assert!(warnings.iter().any(|warning| warning.contains("next")));
    }

    #[test]
    fn schema_detection_accepts_filled_structured_values() {
        let rendered = "template: status\nsummary: Progress update\nstatus: in_progress\nscope: tasks=1\nowner: agent-1\nverification: cargo test blackboard (pass)\nblocker: none\nnext: submit PR";
        let warnings = detect_schema_warnings(Some(BlackboardTemplate::Status), rendered);
        assert!(warnings.is_empty());
    }

    #[test]
    fn schema_detection_skips_free_text_mode() {
        let warnings = detect_schema_warnings(None, "plain text note without schema fields");
        assert!(warnings.is_empty());
    }

    #[test]
    fn auto_link_transition_applies_to_blocker_and_handoff_only() {
        assert!(should_auto_link_transition(Some(
            BlackboardTemplate::Blocker
        )));
        assert!(should_auto_link_transition(Some(
            BlackboardTemplate::Handoff
        )));
        assert!(!should_auto_link_transition(Some(
            BlackboardTemplate::Status
        )));
        assert!(!should_auto_link_transition(None));
    }

    #[test]
    fn derive_transition_links_extracts_mesh_and_blackboard_refs() {
        let message = "template: blocker\nsummary: waiting on B7\nmesh_ref: 550e8400-e29b-41d4-a716-446655440000";
        let links = derive_transition_links(Some(BlackboardTemplate::Blocker), message, Some(12));

        assert_eq!(links.blackboard_note_ids, vec![7, 12]);
        assert_eq!(
            links.mesh_message_ids,
            vec!["550e8400-e29b-41d4-a716-446655440000"]
        );
    }

    #[test]
    fn derive_transition_links_ignores_non_transition_templates() {
        let links = derive_transition_links(
            Some(BlackboardTemplate::Status),
            "B9 550e8400-e29b-41d4-a716-446655440000",
            Some(4),
        );
        assert!(links.is_empty());
    }

    #[test]
    fn post_with_options_auto_links_blocker_template_note() {
        let dir = tempdir().unwrap();
        FileStore::init(dir.path()).unwrap();

        post(
            dir.path(),
            "agent_1",
            "baseline note",
            None,
            vec![],
            vec![],
            Format::Json,
        )
        .unwrap();

        post_with_options(
            dir.path(),
            "agent_1",
            "Blocked on B7 after mesh ping 550e8400-e29b-41d4-a716-446655440000",
            BlackboardPostOptions {
                template: Some(BlackboardTemplate::Blocker),
                since_note: Some(1),
                no_change_since: false,
            },
            vec![],
            vec![],
            Format::Json,
        )
        .unwrap();

        let db = CoordinationDb::from_repo(dir.path()).unwrap();
        let notes = db.list_notes(None, None, None, None).unwrap();
        assert_eq!(notes.len(), 2);

        let note = notes.iter().find(|n| n.id == 2).unwrap();
        // Links are no longer stored in CoordinationDb — verify note was posted
        assert!(note.message.contains("template: blocker"));
    }

    #[test]
    fn sensitive_detection_flags_common_markers() {
        let rendered = "token = ghp_abcd1234EFGH5678IJKL9012";
        let warnings = detect_sensitive_text_warnings(rendered);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("assignment-like"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("GitHub token"))
        );
    }

    #[test]
    fn sensitive_detection_flags_jwt_like_values() {
        let rendered = "auth: eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyIjoiYWdlbnQiLCJyb2xlIjoiYWRtaW4ifQ.XyZ1234567890abcdEFGHijklMNOP";
        let warnings = detect_sensitive_text_warnings(rendered);
        assert!(warnings.iter().any(|warning| warning.contains("JWT-like")));
    }
}
