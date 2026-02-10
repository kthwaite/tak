use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};

use crate::error::Result;
use crate::model::{Learning, Priority, Status, Task};
use crate::output::Format;
use crate::store::coordination_db::{
    CoordinationDb, DbEvent, DbMessage, DbNote, DbRegistration, DbReservation,
};
use crate::store::repo::Repo;
use crate::store::sidecars::{HistoryEvent, VerificationResult};
use crate::task_id::TaskId;

const SECTION_COUNT: usize = 5;
const DETAIL_SCROLL_STEP: usize = 8;
const MAX_NOTES: u32 = 200;
const MAX_EVENTS: u32 = 200;
const MAX_ARTIFACT_PATHS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum TuiSection {
    Tasks,
    Learnings,
    Blackboard,
    Mesh,
    Feed,
}

impl TuiSection {
    const ALL: [Self; SECTION_COUNT] = [
        Self::Tasks,
        Self::Learnings,
        Self::Blackboard,
        Self::Mesh,
        Self::Feed,
    ];

    fn index(self) -> usize {
        match self {
            Self::Tasks => 0,
            Self::Learnings => 1,
            Self::Blackboard => 2,
            Self::Mesh => 3,
            Self::Feed => 4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::Learnings => "Learnings",
            Self::Blackboard => "Blackboard",
            Self::Mesh => "Mesh",
            Self::Feed => "Feed",
        }
    }

    fn next(self) -> Self {
        let next = (self.index() + 1) % SECTION_COUNT;
        Self::ALL[next]
    }

    fn prev(self) -> Self {
        let next = if self.index() == 0 {
            SECTION_COUNT - 1
        } else {
            self.index() - 1
        };
        Self::ALL[next]
    }
}

impl Default for TuiSection {
    fn default() -> Self {
        Self::Tasks
    }
}

impl std::fmt::Display for TuiSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[derive(Debug, Clone)]
struct TakTuiConfig {
    focus: TuiSection,
    query: String,
    tick_rate: StdDuration,
    auto_refresh: StdDuration,
}

impl Default for TakTuiConfig {
    fn default() -> Self {
        Self {
            focus: TuiSection::Tasks,
            query: String::new(),
            tick_rate: StdDuration::from_millis(200),
            auto_refresh: StdDuration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct VerificationCommandSummary {
    command: String,
    exit_code: i32,
    passed: bool,
    stdout_preview: String,
    stderr_preview: String,
}

#[derive(Debug, Clone)]
struct VerificationSummary {
    timestamp: DateTime<Utc>,
    passed: bool,
    commands: Vec<VerificationCommandSummary>,
}

#[derive(Debug, Clone, Default)]
struct TaskSidecarSnapshot {
    context: Option<String>,
    history: Vec<HistoryEvent>,
    verification: Option<VerificationSummary>,
    artifact_paths: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct TakSnapshot {
    tasks: Vec<Task>,
    task_titles: HashMap<u64, String>,
    blocked_ids: HashSet<u64>,
    dependents: HashMap<u64, Vec<u64>>,
    task_sidecars: HashMap<u64, TaskSidecarSnapshot>,
    learnings: Vec<Learning>,
    learnings_by_id: HashMap<u64, Learning>,
    notes: Vec<DbNote>,
    agents: Vec<DbRegistration>,
    agent_inbox: HashMap<String, Vec<DbMessage>>,
    reservations: Vec<DbReservation>,
    feed: Vec<DbEvent>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TaskStatusCounts {
    pending: usize,
    in_progress: usize,
    done: usize,
    cancelled: usize,
}

impl TakSnapshot {
    fn status_counts(&self) -> TaskStatusCounts {
        let mut counts = TaskStatusCounts::default();
        for task in &self.tasks {
            match task.status {
                Status::Pending => counts.pending += 1,
                Status::InProgress => counts.in_progress += 1,
                Status::Done => counts.done += 1,
                Status::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }
}

#[derive(Debug, Clone)]
struct SectionView {
    list_title: String,
    detail_title: String,
    rows: Vec<String>,
    selected: Option<usize>,
    detail: String,
}

#[derive(Debug, Clone)]
struct TakTuiApp {
    repo_root: PathBuf,
    focus: TuiSection,
    query: String,
    search_mode: bool,
    help_visible: bool,
    needs_refresh: bool,
    refresh_count: u32,
    last_refreshed_at: Option<DateTime<Utc>>,
    last_refresh_instant: Option<Instant>,
    last_error: Option<String>,
    tick_rate: StdDuration,
    auto_refresh: StdDuration,
    selected: [usize; SECTION_COUNT],
    detail_scroll: usize,
    snapshot: TakSnapshot,
}

impl TakTuiApp {
    fn new(repo_root: &Path, config: TakTuiConfig) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            focus: config.focus,
            query: config.query.trim().to_string(),
            search_mode: false,
            help_visible: false,
            needs_refresh: true,
            refresh_count: 0,
            last_refreshed_at: None,
            last_refresh_instant: None,
            last_error: None,
            tick_rate: config.tick_rate,
            auto_refresh: config.auto_refresh,
            selected: [0; SECTION_COUNT],
            detail_scroll: 0,
            snapshot: TakSnapshot::default(),
        }
    }

    fn should_auto_refresh(&self) -> bool {
        self.last_refresh_instant
            .is_none_or(|last| last.elapsed() >= self.auto_refresh)
    }

    fn refresh(&mut self) {
        match refresh_snapshot(&self.repo_root) {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(err.to_string());
            }
        }

        self.refresh_count += 1;
        self.last_refreshed_at = Some(Utc::now());
        self.last_refresh_instant = Some(Instant::now());
        self.needs_refresh = false;
        self.normalize_selection();
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }

        if self.search_mode {
            return self.handle_search_key(key);
        }

        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('?') => {
                self.help_visible = !self.help_visible;
                false
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                false
            }
            KeyCode::Char('c') => {
                self.query.clear();
                self.detail_scroll = 0;
                self.normalize_selection();
                false
            }
            KeyCode::Char('r') => {
                self.needs_refresh = true;
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.focus = self.focus.next();
                self.detail_scroll = 0;
                self.normalize_selection();
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.focus = self.focus.prev();
                self.detail_scroll = 0;
                self.normalize_selection();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                false
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.select_first();
                false
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.select_last();
                false
            }
            KeyCode::PageUp | KeyCode::Char('u') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(DETAIL_SCROLL_STEP);
                false
            }
            KeyCode::PageDown | KeyCode::Char('d') => {
                self.detail_scroll = self.detail_scroll.saturating_add(DETAIL_SCROLL_STEP);
                false
            }
            _ => false,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search_mode = false;
                self.detail_scroll = 0;
                self.normalize_selection();
                false
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.detail_scroll = 0;
                self.normalize_selection();
                false
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.query.push(ch);
                    self.detail_scroll = 0;
                    self.normalize_selection();
                }
                false
            }
            _ => false,
        }
    }

    fn select_first(&mut self) {
        self.selected[self.focus.index()] = 0;
        self.detail_scroll = 0;
    }

    fn select_last(&mut self) {
        let len = self.filtered_indices(self.focus).len();
        self.selected[self.focus.index()] = len.saturating_sub(1);
        self.detail_scroll = 0;
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.filtered_indices(self.focus).len();
        if len == 0 {
            self.selected[self.focus.index()] = 0;
            return;
        }

        let selected = &mut self.selected[self.focus.index()];
        if delta.is_negative() {
            *selected = selected.saturating_sub(delta.unsigned_abs());
        } else {
            *selected = selected
                .saturating_add(delta as usize)
                .min(len.saturating_sub(1));
        }
        self.detail_scroll = 0;
    }

    fn normalize_selection(&mut self) {
        for section in TuiSection::ALL {
            let len = self.filtered_indices(section).len();
            let selected = &mut self.selected[section.index()];
            if len == 0 {
                *selected = 0;
            } else if *selected >= len {
                *selected = len - 1;
            }
        }
    }

    fn query_tokens(&self) -> Vec<String> {
        self.query
            .split_whitespace()
            .map(|token| token.to_ascii_lowercase())
            .collect()
    }

    fn filtered_indices(&self, section: TuiSection) -> Vec<usize> {
        let tokens = self.query_tokens();

        match section {
            TuiSection::Tasks => self
                .snapshot
                .tasks
                .iter()
                .enumerate()
                .filter(|(_, task)| self.task_matches(task, &tokens))
                .map(|(idx, _)| idx)
                .collect(),
            TuiSection::Learnings => self
                .snapshot
                .learnings
                .iter()
                .enumerate()
                .filter(|(_, learning)| self.learning_matches(learning, &tokens))
                .map(|(idx, _)| idx)
                .collect(),
            TuiSection::Blackboard => self
                .snapshot
                .notes
                .iter()
                .enumerate()
                .filter(|(_, note)| self.note_matches(note, &tokens))
                .map(|(idx, _)| idx)
                .collect(),
            TuiSection::Mesh => self
                .snapshot
                .agents
                .iter()
                .enumerate()
                .filter(|(_, agent)| self.agent_matches(agent, &tokens))
                .map(|(idx, _)| idx)
                .collect(),
            TuiSection::Feed => self
                .snapshot
                .feed
                .iter()
                .enumerate()
                .filter(|(_, event)| self.feed_matches(event, &tokens))
                .map(|(idx, _)| idx)
                .collect(),
        }
    }

    fn task_matches(&self, task: &Task, tokens: &[String]) -> bool {
        let sidecar = self.snapshot.task_sidecars.get(&task.id);

        let candidate = format!(
            "{} {} {} {} {} {} {} {} {} {} {} {} {}",
            TaskId::from(task.id),
            task.title,
            task.description.as_deref().unwrap_or_default(),
            task.status,
            task.kind,
            task.assignee.as_deref().unwrap_or_default(),
            task.tags.join(" "),
            task.contract.objective.as_deref().unwrap_or_default(),
            task.execution.last_error.as_deref().unwrap_or_default(),
            task.execution.blocked_reason.as_deref().unwrap_or_default(),
            task.depends_on
                .iter()
                .map(|dep| TaskId::from(dep.id).to_string())
                .collect::<Vec<_>>()
                .join(" "),
            task.depends_on
                .iter()
                .filter_map(|dep| dep.reason.as_deref())
                .collect::<Vec<_>>()
                .join(" "),
            sidecar
                .and_then(|entry| entry.context.as_deref())
                .unwrap_or_default(),
        );

        contains_all_tokens(&candidate, tokens)
    }

    fn learning_matches(&self, learning: &Learning, tokens: &[String]) -> bool {
        let candidate = format!(
            "{} {} {} {} {} {}",
            learning.id,
            learning.title,
            learning.description.as_deref().unwrap_or_default(),
            learning.category,
            learning.tags.join(" "),
            learning
                .task_ids
                .iter()
                .map(|id| TaskId::from(*id).to_string())
                .collect::<Vec<_>>()
                .join(" "),
        );
        contains_all_tokens(&candidate, tokens)
    }

    fn note_matches(&self, note: &DbNote, tokens: &[String]) -> bool {
        let candidate = format!(
            "{} {} {} {} {} {}",
            note.id,
            note.from_agent,
            note.status,
            note.tags.join(" "),
            note.task_ids.join(" "),
            note.message,
        );
        contains_all_tokens(&candidate, tokens)
    }

    fn agent_matches(&self, agent: &DbRegistration, tokens: &[String]) -> bool {
        let reservations = self
            .snapshot
            .reservations
            .iter()
            .filter(|reservation| reservation.agent == agent.name)
            .map(|reservation| reservation.path.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let inbox = self
            .snapshot
            .agent_inbox
            .get(&agent.name)
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| message.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();

        let candidate = format!(
            "{} {} {} {} {} {} {}",
            agent.name,
            agent.session_id,
            agent.cwd,
            agent.status,
            agent.metadata,
            reservations,
            inbox,
        );

        contains_all_tokens(&candidate, tokens)
    }

    fn feed_matches(&self, event: &DbEvent, tokens: &[String]) -> bool {
        let candidate = format!(
            "{} {} {} {} {} {}",
            event.id,
            event.agent.as_deref().unwrap_or_default(),
            event.event_type,
            event.target.as_deref().unwrap_or_default(),
            event.preview.as_deref().unwrap_or_default(),
            event.detail.as_deref().unwrap_or_default(),
        );

        contains_all_tokens(&candidate, tokens)
    }

    fn current_view(&self) -> SectionView {
        match self.focus {
            TuiSection::Tasks => self.tasks_view(),
            TuiSection::Learnings => self.learnings_view(),
            TuiSection::Blackboard => self.blackboard_view(),
            TuiSection::Mesh => self.mesh_view(),
            TuiSection::Feed => self.feed_view(),
        }
    }

    fn tasks_view(&self) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Tasks);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Tasks.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.task_row(&self.snapshot.tasks[*idx]))
            .collect::<Vec<_>>();

        let detail = if let Some(position) = selected {
            let task = &self.snapshot.tasks[filtered[position]];
            self.task_detail(task)
        } else {
            no_results_detail("tasks", &self.query)
        };

        let detail_title = if let Some(position) = selected {
            let task = &self.snapshot.tasks[filtered[position]];
            format!("Task {}", TaskId::from(task.id))
        } else {
            "Task detail".to_string()
        };

        SectionView {
            list_title: format!("Tasks ({}/{})", filtered.len(), self.snapshot.tasks.len()),
            detail_title,
            rows,
            selected,
            detail,
        }
    }

    fn learnings_view(&self) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Learnings);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Learnings.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.learning_row(&self.snapshot.learnings[*idx]))
            .collect::<Vec<_>>();

        let detail = if let Some(position) = selected {
            let learning = &self.snapshot.learnings[filtered[position]];
            self.learning_detail(learning)
        } else {
            no_results_detail("learnings", &self.query)
        };

        let detail_title = if let Some(position) = selected {
            let learning = &self.snapshot.learnings[filtered[position]];
            format!("Learning #{}", learning.id)
        } else {
            "Learning detail".to_string()
        };

        SectionView {
            list_title: format!(
                "Learnings ({}/{})",
                filtered.len(),
                self.snapshot.learnings.len()
            ),
            detail_title,
            rows,
            selected,
            detail,
        }
    }

    fn blackboard_view(&self) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Blackboard);
        let selected = pick_selected(
            filtered.len(),
            self.selected[TuiSection::Blackboard.index()],
        );

        let rows = filtered
            .iter()
            .map(|idx| self.note_row(&self.snapshot.notes[*idx]))
            .collect::<Vec<_>>();

        let detail = if let Some(position) = selected {
            let note = &self.snapshot.notes[filtered[position]];
            self.note_detail(note)
        } else {
            no_results_detail("blackboard notes", &self.query)
        };

        let detail_title = if let Some(position) = selected {
            let note = &self.snapshot.notes[filtered[position]];
            format!("Note B{}", note.id)
        } else {
            "Blackboard detail".to_string()
        };

        SectionView {
            list_title: format!(
                "Blackboard ({}/{})",
                filtered.len(),
                self.snapshot.notes.len()
            ),
            detail_title,
            rows,
            selected,
            detail,
        }
    }

    fn mesh_view(&self) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Mesh);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Mesh.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.agent_row(&self.snapshot.agents[*idx]))
            .collect::<Vec<_>>();

        let detail = if let Some(position) = selected {
            let agent = &self.snapshot.agents[filtered[position]];
            self.agent_detail(agent)
        } else {
            no_results_detail("mesh agents", &self.query)
        };

        let detail_title = if let Some(position) = selected {
            let agent = &self.snapshot.agents[filtered[position]];
            format!("Mesh agent {}", agent.name)
        } else {
            "Mesh detail".to_string()
        };

        SectionView {
            list_title: format!(
                "Mesh agents ({}/{})",
                filtered.len(),
                self.snapshot.agents.len()
            ),
            detail_title,
            rows,
            selected,
            detail,
        }
    }

    fn feed_view(&self) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Feed);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Feed.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.feed_row(&self.snapshot.feed[*idx]))
            .collect::<Vec<_>>();

        let detail = if let Some(position) = selected {
            let event = &self.snapshot.feed[filtered[position]];
            self.feed_detail(event)
        } else {
            no_results_detail("feed events", &self.query)
        };

        let detail_title = if let Some(position) = selected {
            let event = &self.snapshot.feed[filtered[position]];
            format!("Feed event #{}", event.id)
        } else {
            "Feed detail".to_string()
        };

        SectionView {
            list_title: format!("Feed ({}/{})", filtered.len(), self.snapshot.feed.len()),
            detail_title,
            rows,
            selected,
            detail,
        }
    }

    fn task_row(&self, task: &Task) -> String {
        let id = TaskId::from(task.id);
        let priority = task
            .planning
            .priority
            .map(|priority| priority.to_string())
            .unwrap_or_else(|| "-".to_string());
        let blocked = if self.snapshot.blocked_ids.contains(&task.id) {
            "B"
        } else {
            "-"
        };
        let assignee = task.assignee.as_deref().unwrap_or("-");

        format!(
            "{} [{}] {:11} {:7} p:{:<8} @{:12} {}",
            id,
            blocked,
            task.status,
            task.kind,
            priority,
            truncate_display(assignee, 12),
            truncate_display(&task.title, 60),
        )
    }

    fn learning_row(&self, learning: &Learning) -> String {
        format!(
            "#{:<4} {:8} {:>2} tasks  {}",
            learning.id,
            learning.category,
            learning.task_ids.len(),
            truncate_display(&learning.title, 60),
        )
    }

    fn note_row(&self, note: &DbNote) -> String {
        format!(
            "B{:<4} {:8} {:14} {}",
            note.id,
            note.status,
            truncate_display(&note.from_agent, 14),
            truncate_display(note.message.lines().next().unwrap_or_default(), 58),
        )
    }

    fn agent_row(&self, agent: &DbRegistration) -> String {
        let inbox = self
            .snapshot
            .agent_inbox
            .get(&agent.name)
            .map_or(0, std::vec::Vec::len);
        let reservations = self
            .snapshot
            .reservations
            .iter()
            .filter(|reservation| reservation.agent == agent.name)
            .count();

        format!(
            "{} [{}] inbox={} reservations={} session={}",
            truncate_display(&agent.name, 22),
            truncate_display(&agent.status, 6),
            inbox,
            reservations,
            truncate_display(&agent.session_id, 16),
        )
    }

    fn feed_row(&self, event: &DbEvent) -> String {
        format!(
            "#{:<4} {:16} {:14} {}",
            event.id,
            truncate_display(&event.event_type, 16),
            truncate_display(event.agent.as_deref().unwrap_or("-"), 14),
            truncate_display(
                event
                    .preview
                    .as_deref()
                    .or(event.target.as_deref())
                    .unwrap_or_default(),
                56,
            ),
        )
    }

    fn task_detail(&self, task: &Task) -> String {
        let mut lines = Vec::new();

        lines.push(format!("id: {}", TaskId::from(task.id)));
        lines.push(format!("title: {}", task.title));
        lines.push(format!(
            "status: {} (blocked={})",
            task.status,
            self.snapshot.blocked_ids.contains(&task.id)
        ));
        lines.push(format!("kind: {}", task.kind));
        lines.push(format!(
            "assignee: {}",
            task.assignee.as_deref().unwrap_or("<unassigned>")
        ));

        lines.push(format!(
            "parent: {}",
            task.parent
                .map(|parent| self.format_task_link(parent))
                .unwrap_or_else(|| "<none>".to_string())
        ));

        if task.depends_on.is_empty() {
            lines.push("depends_on: <none>".to_string());
        } else {
            lines.push("depends_on:".to_string());
            for dependency in &task.depends_on {
                let dep_type = dependency
                    .dep_type
                    .map(|dep_type| dep_type.to_string())
                    .unwrap_or_else(|| "unspecified".to_string());
                let reason = dependency.reason.as_deref().unwrap_or("<none>");
                lines.push(format!(
                    "  - {} type={} reason={}",
                    self.format_task_link(dependency.id),
                    dep_type,
                    reason,
                ));
            }
        }

        let dependents = self
            .snapshot
            .dependents
            .get(&task.id)
            .cloned()
            .unwrap_or_default();

        if dependents.is_empty() {
            lines.push("dependents: <none>".to_string());
        } else {
            lines.push("dependents:".to_string());
            for dependent in dependents {
                lines.push(format!("  - {}", self.format_task_link(dependent)));
            }
        }

        lines.push(format!(
            "tags: {}",
            if task.tags.is_empty() {
                "<none>".to_string()
            } else {
                task.tags.join(", ")
            }
        ));

        lines.push(format!(
            "planning: priority={}, estimate={}, risk={}, required_skills={}",
            task.planning
                .priority
                .map(|priority| priority.to_string())
                .unwrap_or_else(|| "-".to_string()),
            task.planning
                .estimate
                .map(|estimate| estimate.to_string())
                .unwrap_or_else(|| "-".to_string()),
            task.planning
                .risk
                .map(|risk| risk.to_string())
                .unwrap_or_else(|| "-".to_string()),
            if task.planning.required_skills.is_empty() {
                "-".to_string()
            } else {
                task.planning.required_skills.join(", ")
            }
        ));

        lines.push(format!(
            "execution: attempts={}, last_error={}, handoff_summary={}, blocked_reason={}",
            task.execution.attempt_count,
            task.execution.last_error.as_deref().unwrap_or("-"),
            task.execution.handoff_summary.as_deref().unwrap_or("-"),
            task.execution.blocked_reason.as_deref().unwrap_or("-"),
        ));

        lines.push(format!(
            "git: branch={}, start_commit={}, end_commit={}, commits={}, pr={}",
            task.git.branch.as_deref().unwrap_or("-"),
            task.git.start_commit.as_deref().unwrap_or("-"),
            task.git.end_commit.as_deref().unwrap_or("-"),
            task.git.commits.len(),
            task.git.pr.as_deref().unwrap_or("-"),
        ));

        lines.push(format!("created_at: {}", task.created_at.to_rfc3339()));
        lines.push(format!("updated_at: {}", task.updated_at.to_rfc3339()));

        lines.push(String::new());
        lines.push("description:".to_string());
        push_multiline(
            &mut lines,
            "  ",
            task.description.as_deref().unwrap_or("<none>"),
        );

        lines.push(String::new());
        lines.push("contract.objective:".to_string());
        push_multiline(
            &mut lines,
            "  ",
            task.contract.objective.as_deref().unwrap_or("<none>"),
        );

        lines.push("contract.acceptance_criteria:".to_string());
        if task.contract.acceptance_criteria.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for criterion in &task.contract.acceptance_criteria {
                lines.push(format!("  - {}", criterion));
            }
        }

        lines.push("contract.verification:".to_string());
        if task.contract.verification.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for command in &task.contract.verification {
                lines.push(format!("  - {}", command));
            }
        }

        lines.push("contract.constraints:".to_string());
        if task.contract.constraints.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for constraint in &task.contract.constraints {
                lines.push(format!("  - {}", constraint));
            }
        }

        lines.push(String::new());
        lines.push("linked learnings:".to_string());
        if task.learnings.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for learning_id in &task.learnings {
                if let Some(learning) = self.snapshot.learnings_by_id.get(learning_id) {
                    lines.push(format!(
                        "  - #{} {} ({})",
                        learning.id, learning.title, learning.category
                    ));
                } else {
                    lines.push(format!("  - #{} <missing>", learning_id));
                }
            }
        }

        lines.push(String::new());
        lines.push("sidecars.context:".to_string());

        if let Some(sidecar) = self.snapshot.task_sidecars.get(&task.id) {
            push_multiline(
                &mut lines,
                "  ",
                sidecar.context.as_deref().unwrap_or("<none>"),
            );

            lines.push(String::new());
            lines.push(format!(
                "sidecars.history ({} events):",
                sidecar.history.len()
            ));
            if sidecar.history.is_empty() {
                lines.push("  <none>".to_string());
            } else {
                let start = sidecar.history.len().saturating_sub(50);
                if start > 0 {
                    lines.push(format!(
                        "  ... showing newest 50 of {} total",
                        sidecar.history.len()
                    ));
                }

                for event in sidecar.history.iter().skip(start) {
                    lines.push(format!(
                        "  - {} {} agent={} id={}",
                        event.timestamp.to_rfc3339(),
                        event.event,
                        event.agent.as_deref().unwrap_or("-"),
                        event.id.as_deref().unwrap_or("-")
                    ));

                    if !event.detail.is_empty() {
                        lines.push(format!(
                            "    detail: {}",
                            serde_json::to_string(&event.detail)
                                .unwrap_or_else(|_| "<invalid detail json>".to_string())
                        ));
                    }
                    if !event.links.is_empty() {
                        lines.push(format!(
                            "    links: {}",
                            serde_json::to_string(&event.links)
                                .unwrap_or_else(|_| "<invalid links json>".to_string())
                        ));
                    }
                }
            }

            lines.push(String::new());
            lines.push("sidecars.verification:".to_string());
            if let Some(verification) = &sidecar.verification {
                lines.push(format!(
                    "  timestamp={} passed={} command_count={}",
                    verification.timestamp.to_rfc3339(),
                    verification.passed,
                    verification.commands.len()
                ));

                for (idx, command) in verification.commands.iter().enumerate() {
                    lines.push(format!(
                        "  {}. pass={} exit={} command={}",
                        idx + 1,
                        command.passed,
                        command.exit_code,
                        command.command
                    ));

                    if !command.stdout_preview.is_empty() {
                        lines.push("     stdout:".to_string());
                        push_multiline(&mut lines, "       ", &command.stdout_preview);
                    }

                    if !command.stderr_preview.is_empty() {
                        lines.push("     stderr:".to_string());
                        push_multiline(&mut lines, "       ", &command.stderr_preview);
                    }
                }
            } else {
                lines.push("  <none>".to_string());
            }

            lines.push(String::new());
            lines.push(format!(
                "sidecars.artifacts ({}):",
                sidecar.artifact_paths.len()
            ));
            if sidecar.artifact_paths.is_empty() {
                lines.push("  <none>".to_string());
            } else {
                for artifact in &sidecar.artifact_paths {
                    lines.push(format!("  - {}", artifact));
                }
            }
        } else {
            lines.push("  <unavailable>".to_string());
        }

        lines.join("\n")
    }

    fn learning_detail(&self, learning: &Learning) -> String {
        let mut lines = vec![
            format!("id: #{}", learning.id),
            format!("title: {}", learning.title),
            format!("category: {}", learning.category),
            format!(
                "tags: {}",
                if learning.tags.is_empty() {
                    "<none>".to_string()
                } else {
                    learning.tags.join(", ")
                }
            ),
            format!("created_at: {}", learning.created_at.to_rfc3339()),
            format!("updated_at: {}", learning.updated_at.to_rfc3339()),
            String::new(),
            "description:".to_string(),
        ];

        push_multiline(
            &mut lines,
            "  ",
            learning.description.as_deref().unwrap_or("<none>"),
        );

        lines.push(String::new());
        lines.push(format!("linked tasks ({}):", learning.task_ids.len()));
        if learning.task_ids.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for task_id in &learning.task_ids {
                lines.push(format!("  - {}", self.format_task_link(*task_id)));
            }
        }

        lines.push(String::new());
        lines.push(format!(
            "extensions: {}",
            if learning.extensions.is_empty() {
                "<none>".to_string()
            } else {
                serde_json::to_string_pretty(&learning.extensions)
                    .unwrap_or_else(|_| "<invalid extension payload>".to_string())
            }
        ));

        lines.join("\n")
    }

    fn note_detail(&self, note: &DbNote) -> String {
        let mut lines = vec![
            format!("id: B{}", note.id),
            format!("status: {}", note.status),
            format!("from_agent: {}", note.from_agent),
            format!(
                "tags: {}",
                if note.tags.is_empty() {
                    "<none>".to_string()
                } else {
                    note.tags.join(", ")
                }
            ),
        ];

        if note.task_ids.is_empty() {
            lines.push("task_ids: <none>".to_string());
        } else {
            lines.push("task_ids:".to_string());
            for task_id in &note.task_ids {
                if let Ok(parsed) = TaskId::parse_cli(task_id) {
                    lines.push(format!("  - {}", self.format_task_link(parsed.into())));
                } else {
                    lines.push(format!("  - {}", task_id));
                }
            }
        }

        lines.push(format!("created_at: {}", note.created_at.to_rfc3339()));
        lines.push(format!("updated_at: {}", note.updated_at.to_rfc3339()));
        lines.push(format!(
            "closed_by: {}",
            note.closed_by.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!(
            "closed_reason: {}",
            note.closed_reason.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!(
            "closed_at: {}",
            note.closed_at
                .map(|timestamp| timestamp.to_rfc3339())
                .unwrap_or_else(|| "<none>".to_string())
        ));

        lines.push(String::new());
        lines.push("message:".to_string());
        push_multiline(&mut lines, "  ", &note.message);

        lines.join("\n")
    }

    fn agent_detail(&self, agent: &DbRegistration) -> String {
        let mut lines = vec![
            format!("name: {}", agent.name),
            format!("generation: {}", agent.generation),
            format!("session_id: {}", agent.session_id),
            format!("cwd: {}", agent.cwd),
            format!(
                "pid: {}",
                agent
                    .pid
                    .map_or_else(|| "<none>".to_string(), |pid| pid.to_string())
            ),
            format!("host: {}", agent.host.as_deref().unwrap_or("<none>")),
            format!("status: {}", agent.status),
            format!("started_at: {}", agent.started_at.to_rfc3339()),
            format!("updated_at: {}", agent.updated_at.to_rfc3339()),
            format!("metadata: {}", agent.metadata),
            String::new(),
        ];

        let reservations = self
            .snapshot
            .reservations
            .iter()
            .filter(|reservation| reservation.agent == agent.name)
            .collect::<Vec<_>>();

        lines.push(format!("reservations ({}):", reservations.len()));
        if reservations.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for reservation in reservations {
                lines.push(format!(
                    "  - path={} reason={} created={} expires={}",
                    reservation.path,
                    reservation.reason.as_deref().unwrap_or("<none>"),
                    reservation.created_at.to_rfc3339(),
                    reservation.expires_at.to_rfc3339()
                ));
            }
        }

        lines.push(String::new());

        let inbox = self
            .snapshot
            .agent_inbox
            .get(&agent.name)
            .cloned()
            .unwrap_or_default();

        lines.push(format!("inbox ({} unacked):", inbox.len()));
        if inbox.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for message in inbox.iter().take(30) {
                lines.push(format!(
                    "  - {} from={} id={} text={}",
                    message.created_at.to_rfc3339(),
                    message.from_agent,
                    message.id,
                    truncate_display(&message.text, 160),
                ));
            }

            if inbox.len() > 30 {
                lines.push(format!("  ... {} more messages", inbox.len() - 30));
            }
        }

        lines.push(String::new());

        let recent_events = self
            .snapshot
            .feed
            .iter()
            .filter(|event| event.agent.as_deref() == Some(agent.name.as_str()))
            .take(30)
            .collect::<Vec<_>>();

        lines.push(format!(
            "recent feed events by agent ({}):",
            recent_events.len()
        ));
        if recent_events.is_empty() {
            lines.push("  <none>".to_string());
        } else {
            for event in recent_events {
                lines.push(format!(
                    "  - #{} {} target={} preview={}",
                    event.id,
                    event.event_type,
                    event.target.as_deref().unwrap_or("-"),
                    event.preview.as_deref().unwrap_or("-"),
                ));
            }
        }

        lines.join("\n")
    }

    fn feed_detail(&self, event: &DbEvent) -> String {
        let mut lines = vec![
            format!("id: {}", event.id),
            format!("agent: {}", event.agent.as_deref().unwrap_or("<none>")),
            format!("event_type: {}", event.event_type),
            format!("target: {}", event.target.as_deref().unwrap_or("<none>")),
            format!("created_at: {}", event.created_at.to_rfc3339()),
            String::new(),
            "preview:".to_string(),
        ];

        push_multiline(
            &mut lines,
            "  ",
            event.preview.as_deref().unwrap_or("<none>"),
        );

        lines.push(String::new());
        lines.push("detail:".to_string());
        push_multiline(
            &mut lines,
            "  ",
            event.detail.as_deref().unwrap_or("<none>"),
        );

        lines.join("\n")
    }

    fn format_task_link(&self, id: u64) -> String {
        let title = self
            .snapshot
            .task_titles
            .get(&id)
            .map(String::as_str)
            .unwrap_or("<missing>");
        format!("{} ({})", TaskId::from(id), title)
    }

    fn controls_line(&self) -> String {
        let query = if self.query.is_empty() {
            "<none>"
        } else {
            self.query.as_str()
        };

        if self.search_mode {
            format!("search mode: type to edit query | Enter/Esc exit | current query: {query}")
        } else {
            format!(
                "q quit | ←/→ or h/l tabs | ↑/↓ or j/k select | PgUp/PgDn detail | / search | c clear | r refresh | ? help | query={query}"
            )
        }
    }

    fn render(&self, frame: &mut Frame) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let tab_titles = TuiSection::ALL
            .iter()
            .map(|section| Line::from(section.label()))
            .collect::<Vec<_>>();

        frame.render_widget(
            Tabs::new(tab_titles)
                .select(self.focus.index())
                .block(Block::default().borders(Borders::ALL).title("tak tui"))
                .highlight_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            outer[0],
        );

        let status = self.snapshot.status_counts();
        let refreshed_at = self
            .last_refreshed_at
            .as_ref()
            .map(|timestamp| timestamp.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());

        let mut summary = format!(
            "tasks={} (p:{} ip:{} d:{} c:{})  learnings={}  notes={}  agents={}  reservations={}  feed={}  refreshes={}  last={}",
            self.snapshot.tasks.len(),
            status.pending,
            status.in_progress,
            status.done,
            status.cancelled,
            self.snapshot.learnings.len(),
            self.snapshot.notes.len(),
            self.snapshot.agents.len(),
            self.snapshot.reservations.len(),
            self.snapshot.feed.len(),
            self.refresh_count,
            refreshed_at,
        );

        if let Some(error) = &self.last_error {
            summary.push_str(&format!("  last_error={error}"));
        }

        frame.render_widget(
            Paragraph::new(summary)
                .block(Block::default().borders(Borders::ALL).title("Summary"))
                .wrap(Wrap { trim: true }),
            outer[1],
        );

        let center = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(outer[2]);

        let view = self.current_view();
        self.render_list(frame, center[0], &view);

        frame.render_widget(
            Paragraph::new(view.detail)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(view.detail_title),
                )
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll.min(u16::MAX as usize) as u16, 0)),
            center[1],
        );

        frame.render_widget(
            Paragraph::new(self.controls_line())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if self.search_mode {
                            "Search"
                        } else {
                            "Controls"
                        }),
                )
                .wrap(Wrap { trim: true }),
            outer[3],
        );

        if self.help_visible {
            let popup = centered_rect(86, 74, frame.area());
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(
                    "tak tui controls\n\n\
                     Navigation:\n\
                     - Tab / Shift+Tab, Left/Right, h/l: switch section\n\
                     - Up/Down, j/k: move selection\n\
                     - Home/End (or g/G): jump to top/bottom\n\
                     - PgUp/PgDn (or u/d): scroll detail panel\n\n\
                     Search:\n\
                     - /: enter search mode\n\
                     - type to edit query, Backspace to delete\n\
                     - Enter/Esc: exit search mode\n\
                     - c: clear query\n\n\
                     Other:\n\
                     - r: refresh snapshot now\n\
                     - q: quit\n\
                     - ?: toggle this help\n\n\
                     Data coverage:\n\
                     - Tasks (with context/history/verification/artifacts detail)\n\
                     - Learnings\n\
                     - Blackboard notes\n\
                     - Mesh agents/inbox/reservations\n\
                     - Activity feed",
                )
                .block(Block::default().borders(Borders::ALL).title("Help"))
                .wrap(Wrap { trim: true }),
                popup,
            );
        }
    }

    fn render_list(&self, frame: &mut Frame, area: Rect, view: &SectionView) {
        if view.rows.is_empty() {
            frame.render_widget(
                Paragraph::new("(no results)")
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(view.list_title.as_str()),
                    )
                    .wrap(Wrap { trim: true }),
                area,
            );
            return;
        }

        let mut state = ListState::default();
        state.select(view.selected);

        let items = view
            .rows
            .iter()
            .map(|row| ListItem::new(row.as_str()))
            .collect::<Vec<_>>();

        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(view.list_title.as_str()),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("❯ "),
            area,
            &mut state,
        );
    }
}

pub fn run(
    repo_root: &Path,
    focus: TuiSection,
    query: Option<String>,
    _format: Format,
) -> Result<()> {
    let config = TakTuiConfig {
        focus,
        query: query.unwrap_or_default(),
        ..TakTuiConfig::default()
    };

    run_tui(repo_root, config)
}

fn run_tui(repo_root: &Path, config: TakTuiConfig) -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TakTuiApp::new(repo_root, config);
    let run_result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    run_result
}

fn run_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut TakTuiApp) -> Result<()> {
    app.refresh();
    let mut last_tick = Instant::now();

    loop {
        app.normalize_selection();
        terminal
            .draw(|frame| app.render(frame))
            .map_err(|err| std::io::Error::other(err.to_string()))?;

        let timeout = app.tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            break;
        }

        if app.needs_refresh || app.should_auto_refresh() {
            app.refresh();
        }

        if last_tick.elapsed() >= app.tick_rate {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn refresh_snapshot(repo_root: &Path) -> Result<TakSnapshot> {
    let repo = Repo::open(repo_root)?;

    let mut tasks = repo.store.list_all()?;
    sort_tasks(&mut tasks);

    let task_titles = tasks
        .iter()
        .map(|task| (task.id, task.title.clone()))
        .collect::<HashMap<_, _>>();

    let blocked_ids: HashSet<u64> = repo.index.blocked()?.into_iter().map(u64::from).collect();
    let dependents = build_dependents(&tasks);

    let mut task_sidecars = HashMap::new();
    for task in &tasks {
        task_sidecars.insert(task.id, load_sidecars(&repo, task.id)?);
    }

    let mut learnings = repo.learnings.list_all()?;
    learnings.sort_by(|left, right| right.id.cmp(&left.id));
    let learnings_by_id = learnings
        .iter()
        .cloned()
        .map(|learning| (learning.id, learning))
        .collect::<HashMap<_, _>>();

    let db = CoordinationDb::from_repo(repo_root)?;

    let notes = db.list_notes(None, None, None, Some(MAX_NOTES))?;
    let agents = db.list_agents()?;
    let reservations = db.list_reservations()?;
    let feed = db.read_events(Some(MAX_EVENTS))?;

    let mut agent_inbox = HashMap::new();
    for agent in &agents {
        agent_inbox.insert(agent.name.clone(), db.read_inbox(&agent.name)?);
    }

    Ok(TakSnapshot {
        tasks,
        task_titles,
        blocked_ids,
        dependents,
        task_sidecars,
        learnings,
        learnings_by_id,
        notes,
        agents,
        agent_inbox,
        reservations,
        feed,
    })
}

fn load_sidecars(repo: &Repo, task_id: u64) -> Result<TaskSidecarSnapshot> {
    let context = repo.sidecars.read_context(task_id)?;
    let history = repo.sidecars.read_history(task_id)?;
    let verification = repo
        .sidecars
        .read_verification(task_id)?
        .map(summarize_verification);

    let artifact_paths = collect_artifact_paths(
        &repo.sidecars.artifacts_dir(&TaskId::from(task_id)),
        MAX_ARTIFACT_PATHS,
    )?;

    Ok(TaskSidecarSnapshot {
        context,
        history,
        verification,
        artifact_paths,
    })
}

fn summarize_verification(result: VerificationResult) -> VerificationSummary {
    let commands = result
        .results
        .into_iter()
        .map(|command| VerificationCommandSummary {
            command: command.command,
            exit_code: command.exit_code,
            passed: command.passed,
            stdout_preview: preview_text(&command.stdout, 3, 160),
            stderr_preview: preview_text(&command.stderr, 3, 160),
        })
        .collect();

    VerificationSummary {
        timestamp: result.timestamp,
        passed: result.passed,
        commands,
    }
}

fn collect_artifact_paths(root: &Path, max_paths: usize) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(vec![]);
    }

    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if path.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path.as_path())
                    .display()
                    .to_string();
                files.push(relative);
            }
        }
    }

    files.sort();

    if files.len() > max_paths {
        let remaining = files.len() - max_paths;
        files.truncate(max_paths);
        files.push(format!("... ({remaining} more files)"));
    }

    Ok(files)
}

fn sort_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|left, right| {
        priority_rank(left.planning.priority)
            .cmp(&priority_rank(right.planning.priority))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn build_dependents(tasks: &[Task]) -> HashMap<u64, Vec<u64>> {
    let mut map: HashMap<u64, Vec<u64>> = HashMap::new();

    for task in tasks {
        for dependency in &task.depends_on {
            map.entry(dependency.id).or_default().push(task.id);
        }
    }

    for dependents in map.values_mut() {
        dependents.sort_unstable();
        dependents.dedup();
    }

    map
}

fn priority_rank(priority: Option<Priority>) -> u8 {
    priority.map(Priority::rank).unwrap_or(4)
}

fn contains_all_tokens(candidate: &str, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }

    let normalized = candidate.to_ascii_lowercase();
    tokens.iter().all(|token| normalized.contains(token))
}

fn truncate_display(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut output = String::new();

    for _ in 0..max_chars {
        if let Some(ch) = chars.next() {
            output.push(ch);
        } else {
            return output;
        }
    }

    if chars.next().is_some() {
        output.push('…');
    }

    output
}

fn preview_text(value: &str, max_lines: usize, max_line_chars: usize) -> String {
    if value.trim().is_empty() {
        return String::new();
    }

    let lines = value.lines().collect::<Vec<_>>();
    let mut preview = lines
        .iter()
        .take(max_lines)
        .map(|line| truncate_display(line, max_line_chars))
        .collect::<Vec<_>>();

    if lines.len() > max_lines {
        preview.push(format!("… ({} more lines)", lines.len() - max_lines));
    }

    preview.join("\n")
}

fn push_multiline(lines: &mut Vec<String>, prefix: &str, text: &str) {
    if text.trim().is_empty() {
        lines.push(format!("{prefix}<empty>"));
        return;
    }

    for line in text.lines() {
        lines.push(format!("{prefix}{line}"));
    }
}

fn pick_selected(len: usize, candidate: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(candidate.min(len - 1))
    }
}

fn no_results_detail(noun: &str, query: &str) -> String {
    if query.trim().is_empty() {
        format!("No {noun} available.")
    } else {
        format!("No {noun} match query: {:?}", query)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{TimeZone, Utc};
    use crossterm::event::KeyEvent;
    use ratatui::backend::TestBackend;

    use crate::model::{
        Contract, DepType, Dependency, Execution, GitInfo, Kind, LearningCategory, Planning,
    };

    fn task(id: u64, title: &str) -> Task {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        Task {
            id,
            title: title.to_string(),
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
            learnings: vec![],
            created_at: ts,
            updated_at: ts,
            extensions: serde_json::Map::new(),
        }
    }

    fn learning(id: u64, title: &str) -> Learning {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        Learning {
            id,
            title: title.to_string(),
            description: None,
            category: LearningCategory::Insight,
            tags: vec![],
            task_ids: vec![],
            created_at: ts,
            updated_at: ts,
            extensions: serde_json::Map::new(),
        }
    }

    fn app_with_snapshot() -> TakTuiApp {
        let mut app = TakTuiApp::new(
            Path::new("."),
            TakTuiConfig {
                focus: TuiSection::Tasks,
                query: String::new(),
                tick_rate: StdDuration::from_millis(200),
                auto_refresh: StdDuration::from_secs(5),
            },
        );

        let mut alpha = task(1, "Alpha task");
        alpha.tags.push("alpha-only".into());

        let mut beta = task(2, "Beta task");
        beta.depends_on = vec![Dependency {
            id: 1,
            dep_type: Some(DepType::Hard),
            reason: Some("needs alpha".into()),
        }];

        app.snapshot.tasks = vec![alpha.clone(), beta.clone()];
        app.snapshot.task_titles =
            HashMap::from([(1, alpha.title.clone()), (2, beta.title.clone())]);
        app.snapshot.dependents = HashMap::from([(1, vec![2])]);
        app.snapshot.learnings = vec![learning(1, "Alpha learning")];
        app.snapshot.learnings_by_id = HashMap::from([(1, learning(1, "Alpha learning"))]);

        app
    }

    #[test]
    fn section_navigation_wraps() {
        assert_eq!(TuiSection::Tasks.prev(), TuiSection::Feed);
        assert_eq!(TuiSection::Feed.next(), TuiSection::Tasks);
    }

    #[test]
    fn search_mode_updates_query() {
        let mut app = app_with_snapshot();
        app.search_mode = true;

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('b')));
        assert_eq!(app.query, "ab");

        let _ = app.handle_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(app.query, "a");

        let _ = app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(!app.search_mode);
    }

    #[test]
    fn task_filter_matches_query_tokens() {
        let mut app = app_with_snapshot();
        app.query = "alpha-only".into();

        let filtered = app.filtered_indices(TuiSection::Tasks);
        assert_eq!(filtered.len(), 1);
        assert_eq!(app.snapshot.tasks[filtered[0]].title, "Alpha task");

        app.query = "beta needs".into();
        let filtered = app.filtered_indices(TuiSection::Tasks);
        assert_eq!(filtered.len(), 1);
        assert_eq!(app.snapshot.tasks[filtered[0]].title, "Beta task");
    }

    #[test]
    fn normalize_selection_clamps_when_query_reduces_results() {
        let mut app = app_with_snapshot();
        app.selected[TuiSection::Tasks.index()] = 1;
        app.query = "alpha-only".into();

        app.normalize_selection();

        assert_eq!(app.selected[TuiSection::Tasks.index()], 0);
    }

    #[test]
    fn render_smoke() {
        let app = app_with_snapshot();
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn preview_text_truncates_lines_and_count() {
        let text = "line1\nline2\nline3\nline4";
        let preview = preview_text(text, 2, 10);
        assert!(preview.contains("line1"));
        assert!(preview.contains("line2"));
        assert!(preview.contains("more lines"));
    }
}
