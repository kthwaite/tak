use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use colored::Colorize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::{Result, TakError};
use crate::output::Format;
use crate::store::blackboard::{BlackboardNote, BlackboardStore};
use crate::store::mesh::{FeedEvent, MeshStore};
use crate::store::therapist::{TherapistMode, TherapistObservation, TherapistStore};

pub fn offline(
    repo_root: &Path,
    by: Option<String>,
    limit: Option<usize>,
    format: Format,
) -> Result<()> {
    let tak_root = repo_root.join(".tak");
    let mesh = MeshStore::open(&tak_root);
    let board = BlackboardStore::open(&tak_root);

    let scan_limit = limit.unwrap_or(200);
    let feed = mesh.read_feed(Some(scan_limit))?;
    let notes = board.list(None, None, None, Some(scan_limit))?;

    let diagnosis = diagnose_offline(&feed, &notes);

    let mut metrics = diagnosis.metrics;
    metrics.insert("scan_limit".into(), json!(scan_limit));

    let observation = TherapistObservation {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        mode: TherapistMode::Offline,
        session: None,
        requested_by: by,
        summary: diagnosis.summary,
        findings: diagnosis.findings,
        recommendations: diagnosis.recommendations,
        interview: None,
        metrics,
    };

    let store = TherapistStore::open(&tak_root);
    store.append(&observation)?;
    print_single(&observation, store.log_path_for_display().as_path(), format)
}

pub fn online(
    repo_root: &Path,
    session: Option<String>,
    session_dir: Option<String>,
    by: Option<String>,
    format: Format,
) -> Result<()> {
    let tak_root = repo_root.join(".tak");
    let session_root = resolve_session_root(session_dir.as_deref())?;

    let (session_arg, resolved_path) = resolve_session_target(session.as_deref(), &session_root)?;
    let session_stats = resolved_path
        .as_deref()
        .and_then(|path| inspect_session(path).ok());

    let prompt = build_online_prompt(&session_arg, session_stats.as_ref());
    let interview = run_rpc_interview(&session_arg, &prompt)?;

    let recommendations = extract_recommendations(&interview);
    let mut findings = vec![format!(
        "Interviewed resumed pi session `{session_arg}` via RPC mode."
    )];
    if let Some(stats) = &session_stats {
        if stats.work_loop_mentions == 0 {
            findings.push(
                "Session inspection found no explicit `/tak work` markers; recommendations may be lower-confidence."
                    .into(),
            );
        } else {
            findings.push(format!(
                "Session includes {} `/tak work` marker(s), {} total `/tak` mentions.",
                stats.work_loop_mentions, stats.tak_mentions
            ));
        }
    } else {
        findings
            .push("Session file not locally resolvable; relied on pi --session resolution.".into());
    }

    let summary = first_non_empty_line(&interview)
        .map(|line| truncate(line, 180))
        .unwrap_or_else(|| "Online therapist interview completed.".into());

    let mut metrics = serde_json::Map::new();
    metrics.insert("interview_chars".into(), json!(interview.chars().count()));
    if let Some(stats) = &session_stats {
        metrics.insert("work_loop_mentions".into(), json!(stats.work_loop_mentions));
        metrics.insert("tak_mentions".into(), json!(stats.tak_mentions));
    }

    let observation = TherapistObservation {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        mode: TherapistMode::Online,
        session: Some(session_arg),
        requested_by: by,
        summary,
        findings,
        recommendations,
        interview: Some(interview),
        metrics,
    };

    let store = TherapistStore::open(&tak_root);
    store.append(&observation)?;
    print_single(&observation, store.log_path_for_display().as_path(), format)
}

pub fn log(repo_root: &Path, limit: Option<usize>, format: Format) -> Result<()> {
    let tak_root = repo_root.join(".tak");
    let store = TherapistStore::open(&tak_root);
    let rows = store.list(limit)?;

    match format {
        Format::Json => println!("{}", serde_json::to_string(&rows)?),
        Format::Pretty => {
            if rows.is_empty() {
                println!("{}", "No therapist observations yet.".dimmed());
            } else {
                for row in &rows {
                    print_pretty_observation(row, Some(store.log_path_for_display().as_path()));
                    println!();
                }
            }
        }
        Format::Minimal => {
            for row in &rows {
                println!("{} {}", row.id, row.mode);
            }
        }
    }

    Ok(())
}

struct OfflineDiagnosis {
    summary: String,
    findings: Vec<String>,
    recommendations: Vec<String>,
    metrics: serde_json::Map<String, serde_json::Value>,
}

fn diagnose_offline(feed: &[FeedEvent], notes: &[BlackboardNote]) -> OfflineDiagnosis {
    let send_events = feed.iter().filter(|e| e.event_type == "mesh.send").count();
    let reserve_events = feed
        .iter()
        .filter(|e| e.event_type == "mesh.reserve")
        .count();
    let release_events = feed
        .iter()
        .filter(|e| e.event_type == "mesh.release")
        .count();

    let reservation_friction = feed
        .iter()
        .filter(|e| {
            e.event_type == "mesh.send"
                && preview_contains_any(
                    e.preview.as_deref().unwrap_or_default(),
                    &[
                        "blocked",
                        "reservation",
                        "conflict",
                        "release",
                        "unblock",
                        "wait",
                    ],
                )
        })
        .count();

    let blocker_notes = notes
        .iter()
        .filter(|n| {
            n.tags.iter().any(|t| t == "blocker")
                || preview_contains_any(&n.message, &["blocked", "conflict", "waiting", "stuck"])
        })
        .count();

    let handoff_mentions = notes
        .iter()
        .filter(|n| preview_contains_any(&n.message, &["handoff", "handed off"]))
        .count();

    let coordination_notes = notes
        .iter()
        .filter(|n| n.tags.iter().any(|t| t == "coordination"))
        .count();

    let churn_ratio = if reserve_events == 0 {
        0.0
    } else {
        release_events as f64 / reserve_events as f64
    };

    let friction_score = reservation_friction + blocker_notes + handoff_mentions;
    let friction_label = match friction_score {
        0..=2 => "low",
        3..=6 => "moderate",
        _ => "high",
    };

    let mut findings = Vec::new();
    if reservation_friction > 0 {
        findings.push(format!(
            "Detected {reservation_friction} mesh message(s) indicating reservation/blocker contention."
        ));
    }
    if blocker_notes > 0 {
        findings.push(format!(
            "Observed {blocker_notes} blocker-oriented blackboard note(s), suggesting repeated coordination stalls."
        ));
    }
    if handoff_mentions > 0 {
        findings.push(format!(
            "Found {handoff_mentions} handoff mention(s); handoff quality likely affects cycle time."
        ));
    }
    if release_events > reserve_events.saturating_mul(2) && reserve_events > 0 {
        findings.push(format!(
            "Release/reserve ratio is {:.2}, indicating potentially high reservation churn.",
            churn_ratio
        ));
    }
    if coordination_notes == 0 {
        findings.push(
            "No recent notes tagged `coordination`; implicit coordination may be causing avoidable confusion.".into(),
        );
    }

    if findings.is_empty() {
        findings.push(
            "No obvious conflict hotspots found in the sampled feed and blackboard window.".into(),
        );
    }

    let mut recommendations = Vec::new();
    if reservation_friction > 0 {
        recommendations.push(
            "Add a short-lived reservation queue/window primitive (or `/tak reserve-window`) to reduce ping/release churn."
                .into(),
        );
    }
    if blocker_notes > 0 {
        recommendations.push(
            "Have `/tak work` auto-post structured blocker notes (path, owner, unblock request, timeout) to the blackboard."
                .into(),
        );
    }
    if handoff_mentions > 0 {
        recommendations.push(
            "Add a handoff template in CLI/extension with required fields: done, next-step, exact blocked path, verification state."
                .into(),
        );
    }
    recommendations.push(
        "Run `tak therapist offline` at least daily and track friction metrics trend as a leading indicator for time-to-done."
            .into(),
    );

    let summary = format!(
        "Offline diagnosis: {friction_label} coordination friction across {} mesh event(s) and {} blackboard note(s).",
        feed.len(),
        notes.len()
    );

    let mut metrics = serde_json::Map::new();
    metrics.insert("feed_events".into(), json!(feed.len()));
    metrics.insert("send_events".into(), json!(send_events));
    metrics.insert("reserve_events".into(), json!(reserve_events));
    metrics.insert("release_events".into(), json!(release_events));
    metrics.insert(
        "reservation_friction_signals".into(),
        json!(reservation_friction),
    );
    metrics.insert("blocker_notes".into(), json!(blocker_notes));
    metrics.insert("handoff_mentions".into(), json!(handoff_mentions));
    metrics.insert("coordination_notes".into(), json!(coordination_notes));
    metrics.insert("release_to_reserve_ratio".into(), json!(churn_ratio));

    OfflineDiagnosis {
        summary,
        findings,
        recommendations,
        metrics,
    }
}

fn preview_contains_any(haystack: &str, needles: &[&str]) -> bool {
    let lower = haystack.to_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn resolve_session_root(session_dir: Option<&str>) -> Result<PathBuf> {
    if let Some(dir) = session_dir {
        return Ok(PathBuf::from(dir));
    }

    let home = std::env::var("HOME")
        .map_err(|_| therapist_error("HOME not set; provide --session-dir"))?;
    Ok(PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("sessions"))
}

fn resolve_session_target(
    session: Option<&str>,
    session_root: &Path,
) -> Result<(String, Option<PathBuf>)> {
    if let Some(session) = session {
        let direct = PathBuf::from(session);
        if direct.exists() {
            return Ok((direct.display().to_string(), Some(direct)));
        }

        if let Some(path) = find_session_by_prefix(session_root, session)? {
            return Ok((path.display().to_string(), Some(path)));
        }

        return Ok((session.to_string(), None));
    }

    let candidate = find_latest_work_loop_session(session_root)?.ok_or_else(|| {
        therapist_error(
            "No recent session with `/tak work` markers found. Pass --session <id|path> explicitly.",
        )
    })?;

    Ok((candidate.display().to_string(), Some(candidate)))
}

fn find_session_by_prefix(root: &Path, prefix: &str) -> Result<Option<PathBuf>> {
    let mut files = Vec::new();
    collect_session_files(root, &mut files)?;

    let mut matches = files
        .into_iter()
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let stem = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.contains(prefix) || stem.starts_with(prefix)
        })
        .collect::<Vec<_>>();

    matches.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    Ok(matches.pop())
}

fn find_latest_work_loop_session(root: &Path) -> Result<Option<PathBuf>> {
    let mut files = Vec::new();
    collect_session_files(root, &mut files)?;

    let mut candidates = Vec::new();
    for path in files {
        let mentions = count_work_loop_markers(&path)?;
        if mentions > 0 {
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            candidates.push((modified, path));
        }
    }

    candidates.sort_by_key(|(modified, _)| *modified);
    Ok(candidates.pop().map(|(_, path)| path))
}

fn collect_session_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            collect_session_files(&path, out)?;
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct SessionStats {
    work_loop_mentions: usize,
    tak_mentions: usize,
}

fn inspect_session(path: &Path) -> Result<SessionStats> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    let mut work_loop_mentions = 0usize;
    let mut tak_mentions = 0usize;

    for line in reader.lines() {
        let line = line?;
        if line.contains("/tak work") || line.contains("Work loop") {
            work_loop_mentions += 1;
        }
        if line.contains("/tak") {
            tak_mentions += 1;
        }
    }

    Ok(SessionStats {
        work_loop_mentions,
        tak_mentions,
    })
}

fn count_work_loop_markers(path: &Path) -> Result<usize> {
    Ok(inspect_session(path)?.work_loop_mentions)
}

fn build_online_prompt(session_label: &str, stats: Option<&SessionStats>) -> String {
    let stats_line = if let Some(stats) = stats {
        format!(
            "Context hints: {} `/tak work` markers and {} `/tak` mentions were detected in this session file.",
            stats.work_loop_mentions, stats.tak_mentions
        )
    } else {
        "Context hints: session file stats were unavailable; infer from resumed conversation context.".into()
    };

    format!(
        "You are a tak workflow therapist. Resume session `{session_label}` and reflect on the /tak work-loop experience as if you were that agent.\n\
{stats_line}\n\
Answer with four concise sections:\n\
1) Friction/conflict hotspots in the workflow\n\
2) How the agent felt/thought during those moments\n\
3) Top interface/workflow improvements ranked by impact\n\
4) Two experiments with measurable success criteria\n\
Use bullets and keep it actionable."
    )
}

fn therapist_pi_binary() -> String {
    std::env::var("TAK_THERAPIST_PI_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "pi".to_string())
}

fn run_rpc_interview(session: &str, prompt: &str) -> Result<String> {
    let pi_bin = therapist_pi_binary();
    let mut child = Command::new(&pi_bin)
        .arg("--mode")
        .arg("rpc")
        .arg("--session")
        .arg(session)
        .arg("--no-tools")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| therapist_error(format!("failed to spawn `{pi_bin}` in rpc mode: {e}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| therapist_error("failed to open stdin for pi rpc process"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| therapist_error("failed to open stdout for pi rpc process"))?;
    let mut reader = BufReader::new(stdout);

    writeln!(
        stdin,
        "{}",
        json!({"id": "therapist-prompt", "type": "prompt", "message": prompt})
    )?;
    stdin.flush()?;

    let mut line = String::new();
    let mut saw_agent_end = false;

    while reader.read_line(&mut line)? > 0 {
        let raw = line.trim().to_string();
        line.clear();
        if raw.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };

        if event.get("type").and_then(Value::as_str) == Some("response")
            && event.get("id").and_then(Value::as_str) == Some("therapist-prompt")
            && !event
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            let message = event
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("prompt failed");
            return Err(therapist_error(format!("pi rpc prompt failed: {message}")));
        }

        if event.get("type").and_then(Value::as_str) == Some("agent_end") {
            saw_agent_end = true;
            break;
        }
    }

    if !saw_agent_end {
        return Err(therapist_error(
            "pi rpc stream ended before the therapist interview completed",
        ));
    }

    writeln!(
        stdin,
        "{}",
        json!({"id": "therapist-last", "type": "get_last_assistant_text"})
    )?;
    stdin.flush()?;

    let mut interview_text: Option<String> = None;
    line.clear();
    while reader.read_line(&mut line)? > 0 {
        let raw = line.trim().to_string();
        line.clear();
        if raw.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };

        if event.get("type").and_then(Value::as_str) != Some("response") {
            continue;
        }
        if event.get("id").and_then(Value::as_str) != Some("therapist-last") {
            continue;
        }

        if !event
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let message = event
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("get_last_assistant_text failed");
            return Err(therapist_error(format!(
                "pi rpc failed to fetch interview text: {message}"
            )));
        }

        interview_text = event
            .get("data")
            .and_then(|d| d.get("text"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        break;
    }

    drop(stdin);

    for _ in 0..20 {
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    if child.try_wait()?.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }

    interview_text.ok_or_else(|| therapist_error("pi rpc interview returned empty assistant text"))
}

fn extract_recommendations(interview: &str) -> Vec<String> {
    let mut out = Vec::new();

    for line in interview.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let cleaned = trimmed
            .trim_start_matches(|c: char| {
                c == '-'
                    || c == '*'
                    || c == '•'
                    || c == ' '
                    || c.is_ascii_digit()
                    || c == '.'
                    || c == ')'
            })
            .trim();

        if cleaned.len() < 12 {
            continue;
        }

        let lower = cleaned.to_lowercase();
        if lower.contains("improv")
            || lower.contains("recommend")
            || lower.contains("should")
            || lower.contains("could")
            || lower.contains("experiment")
            || lower.contains("try")
        {
            out.push(cleaned.to_string());
        }

        if out.len() >= 5 {
            break;
        }
    }

    if out.is_empty() {
        out.push(
            "Review the interview transcript and extract top 2-3 workflow experiments.".into(),
        );
    }

    out
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn truncate(text: &str, max_len: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_len {
        return text.to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_len.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn therapist_error(message: impl Into<String>) -> TakError {
    TakError::Io(std::io::Error::other(message.into()))
}

fn print_single(observation: &TherapistObservation, log_path: &Path, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(observation)?),
        Format::Pretty => print_pretty_observation(observation, Some(log_path)),
        Format::Minimal => println!("{}", observation.id),
    }
    Ok(())
}

fn print_pretty_observation(observation: &TherapistObservation, log_path: Option<&Path>) {
    println!(
        "{} {} {}",
        "[therapist]".magenta().bold(),
        observation.mode.to_string().cyan().bold(),
        observation.timestamp.to_rfc3339().dimmed(),
    );

    if let Some(session) = observation.session.as_deref() {
        println!("  {} {}", "session:".dimmed(), session);
    }
    if let Some(by) = observation.requested_by.as_deref() {
        println!("  {} {}", "requested_by:".dimmed(), by);
    }

    println!("  {} {}", "summary:".dimmed(), observation.summary);

    if !observation.findings.is_empty() {
        println!("  {}", "findings:".dimmed());
        for finding in &observation.findings {
            println!("    - {}", finding);
        }
    }

    if !observation.recommendations.is_empty() {
        println!("  {}", "recommendations:".dimmed());
        for recommendation in &observation.recommendations {
            println!("    - {}", recommendation.green());
        }
    }

    if let Some(interview) = observation.interview.as_deref() {
        println!("  {}", "interview:".dimmed());
        for line in interview.lines().take(12) {
            println!("    {}", line);
        }
        if interview.lines().count() > 12 {
            println!("    {}", "...".dimmed());
        }
    }

    if let Some(log_path) = log_path {
        println!("  {} {}", "log:".dimmed(), log_path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::blackboard::BlackboardStatus;
    use crate::store::coordination::CoordinationLinks;

    fn feed_event(event_type: &str, preview: &str) -> FeedEvent {
        FeedEvent {
            ts: Utc::now(),
            agent: "agent-a".into(),
            event_type: event_type.into(),
            target: None,
            preview: Some(preview.into()),
        }
    }

    fn note(tags: &[&str], message: &str) -> BlackboardNote {
        BlackboardNote {
            id: 1,
            author: "agent-a".into(),
            message: message.into(),
            status: BlackboardStatus::Open,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            task_ids: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_by: None,
            closed_reason: None,
            closed_at: None,
            links: CoordinationLinks::default(),
        }
    }

    #[test]
    fn diagnose_offline_flags_contention() {
        let feed = vec![
            feed_event("mesh.send", "blocked on reservation conflict"),
            feed_event("mesh.reserve", "reserved src/store/files.rs"),
            feed_event("mesh.release", "released all"),
            feed_event("mesh.release", "released all"),
        ];
        let notes = vec![
            note(&["blocker"], "Task blocked waiting on release"),
            note(&["coordination"], "handoff complete"),
        ];

        let diagnosis = diagnose_offline(&feed, &notes);

        assert!(diagnosis.summary.contains("coordination friction"));
        assert!(
            diagnosis
                .recommendations
                .iter()
                .any(|r| r.contains("reservation"))
        );
        assert!(
            diagnosis
                .metrics
                .contains_key("reservation_friction_signals")
        );
    }

    #[test]
    fn extract_recommendations_falls_back_when_unstructured() {
        let recs = extract_recommendations("plain narrative without explicit action verbs");
        assert_eq!(recs.len(), 1);
        assert!(recs[0].contains("extract top 2-3 workflow experiments"));
    }

    #[test]
    fn latest_work_loop_session_prefers_newest_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old = root.join("old.jsonl");
        let new = root.join("new.jsonl");

        fs::write(&old, "user: /tak work\n").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&new, "user: /tak work status\n").unwrap();

        let selected = find_latest_work_loop_session(root).unwrap().unwrap();
        assert_eq!(selected.file_name().unwrap(), "new.jsonl");
    }
}
