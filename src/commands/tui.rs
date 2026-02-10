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
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};

use crate::error::Result;
use crate::model::{Kind, Learning, Priority, Status, Task};
use crate::output::Format;
use crate::store::coordination_db::{
    CoordinationDb, DbEvent, DbMessage, DbNote, DbRegistration, DbReservation,
};
use crate::store::repo::Repo;
use crate::store::sidecars::{HistoryEvent, VerificationResult};
use crate::task_id::TaskId;

const SECTION_COUNT: usize = 5;
const DETAIL_SCROLL_STEP: usize = 8;
const DETAIL_KEY_WIDTH: usize = 18;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DensityMode {
    Compact,
    #[default]
    Comfortable,
}

impl DensityMode {
    fn toggle(self) -> Self {
        match self {
            Self::Compact => Self::Comfortable,
            Self::Comfortable => Self::Compact,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }
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
struct TuiTheme {
    text: Style,
    muted: Style,
    tab_idle: Style,
    tab_active: Style,
    panel_idle_border: Style,
    panel_search_border: Style,
    list_selected: Style,
    badge_neutral: Style,
    badge_ready: Style,
    badge_blocked: Style,
    badge_warning: Style,
    status_pending: Style,
    status_in_progress: Style,
    status_done: Style,
    status_cancelled: Style,
    priority_critical: Style,
    priority_high: Style,
    priority_medium: Style,
    priority_low: Style,
    priority_none: Style,
    kind_epic: Style,
    kind_feature: Style,
    kind_task: Style,
    kind_bug: Style,
    kind_meta: Style,
    kind_idea: Style,
    detail_heading: Style,
    detail_key: Style,
    detail_value: Style,
    detail_separator: Style,
    match_highlight: Style,
}

impl Default for TuiTheme {
    fn default() -> Self {
        Self {
            text: Style::default().fg(Color::White),
            muted: Style::default().fg(Color::DarkGray),
            tab_idle: Style::default().fg(Color::Gray),
            tab_active: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            panel_idle_border: Style::default().fg(Color::DarkGray),
            panel_search_border: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            list_selected: Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            badge_neutral: Style::default().fg(Color::Cyan),
            badge_ready: Style::default().fg(Color::Green),
            badge_blocked: Style::default().fg(Color::Red),
            badge_warning: Style::default().fg(Color::Yellow),
            status_pending: Style::default().fg(Color::Yellow),
            status_in_progress: Style::default().fg(Color::Blue),
            status_done: Style::default().fg(Color::Green),
            status_cancelled: Style::default().fg(Color::DarkGray),
            priority_critical: Style::default().fg(Color::Red),
            priority_high: Style::default().fg(Color::Yellow),
            priority_medium: Style::default().fg(Color::Cyan),
            priority_low: Style::default().fg(Color::Gray),
            priority_none: Style::default().fg(Color::DarkGray),
            kind_epic: Style::default().fg(Color::Magenta),
            kind_feature: Style::default().fg(Color::Cyan),
            kind_task: Style::default().fg(Color::White),
            kind_bug: Style::default().fg(Color::Red),
            kind_meta: Style::default().fg(Color::Blue),
            kind_idea: Style::default().fg(Color::Yellow),
            detail_heading: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            detail_key: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            detail_value: Style::default().fg(Color::White),
            detail_separator: Style::default().fg(Color::DarkGray),
            match_highlight: Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        }
    }
}

impl TuiTheme {
    fn section_style(&self, section: TuiSection) -> Style {
        match section {
            TuiSection::Tasks => Style::default().fg(Color::Cyan),
            TuiSection::Learnings => Style::default().fg(Color::Magenta),
            TuiSection::Blackboard => Style::default().fg(Color::Yellow),
            TuiSection::Mesh => Style::default().fg(Color::Blue),
            TuiSection::Feed => Style::default().fg(Color::Green),
        }
    }

    fn panel_focus_border(&self, section: TuiSection) -> Style {
        self.section_style(section).add_modifier(Modifier::BOLD)
    }

    fn section_highlight_style(&self, section: TuiSection) -> Style {
        match section {
            TuiSection::Tasks => self.list_selected.fg(Color::Cyan),
            TuiSection::Learnings => self.list_selected.fg(Color::Magenta),
            TuiSection::Blackboard => self.list_selected.fg(Color::Yellow),
            TuiSection::Mesh => self.list_selected.fg(Color::Blue),
            TuiSection::Feed => self.list_selected.fg(Color::Green),
        }
    }

    fn status_style(&self, status: Status) -> Style {
        match status {
            Status::Pending => self.status_pending,
            Status::InProgress => self.status_in_progress,
            Status::Done => self.status_done,
            Status::Cancelled => self.status_cancelled,
        }
    }

    fn priority_style(&self, priority: Option<Priority>) -> Style {
        match priority {
            Some(Priority::Critical) => self.priority_critical,
            Some(Priority::High) => self.priority_high,
            Some(Priority::Medium) => self.priority_medium,
            Some(Priority::Low) => self.priority_low,
            None => self.priority_none,
        }
    }

    fn kind_style(&self, kind: Kind) -> Style {
        match kind {
            Kind::Epic => self.kind_epic,
            Kind::Feature => self.kind_feature,
            Kind::Task => self.kind_task,
            Kind::Bug => self.kind_bug,
            Kind::Meta => self.kind_meta,
            Kind::Idea => self.kind_idea,
        }
    }

    fn note_status_style(&self, status: &str) -> Style {
        if status.eq_ignore_ascii_case("open") {
            self.status_in_progress
        } else if status.eq_ignore_ascii_case("closed") {
            self.status_done
        } else {
            self.badge_warning
        }
    }

    fn agent_status_style(&self, status: &str) -> Style {
        if status.eq_ignore_ascii_case("active") {
            self.status_in_progress
        } else {
            self.status_cancelled
        }
    }

    fn feed_event_style(&self, event_type: &str) -> Style {
        let normalized = event_type.to_ascii_lowercase();

        if normalized.contains("error") || normalized.contains("fail") {
            self.badge_blocked
        } else if normalized.contains("heartbeat") {
            self.muted
        } else if normalized.contains("reserve") || normalized.contains("release") {
            self.kind_feature
        } else if normalized.contains("join") || normalized.contains("leave") {
            self.kind_meta
        } else {
            self.badge_neutral
        }
    }
}

#[derive(Debug, Clone)]
struct SectionView {
    list_title: String,
    detail_title: String,
    rows: Vec<Line<'static>>,
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
    density_mode: DensityMode,
    snapshot: TakSnapshot,
    theme: TuiTheme,
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
            density_mode: DensityMode::default(),
            snapshot: TakSnapshot::default(),
            theme: TuiTheme::default(),
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
            KeyCode::Char('z') => {
                self.density_mode = self.density_mode.toggle();
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

    fn adaptive_list_limit(
        &self,
        list_width: u16,
        compact_cap: usize,
        comfortable_cap: usize,
    ) -> usize {
        let width = usize::from(list_width).max(28);
        let (cap, reserved_columns) = match self.density_mode {
            DensityMode::Compact => (compact_cap, 30),
            DensityMode::Comfortable => (comfortable_cap, 38),
        };

        let min_chars = cap.min(12).max(4);
        width
            .saturating_sub(reserved_columns)
            .clamp(min_chars, cap.max(min_chars))
    }

    fn adaptive_detail_limit(&self, detail_width: u16) -> usize {
        let width = usize::from(detail_width).max(40);
        let reserved = match self.density_mode {
            DensityMode::Compact => 10,
            DensityMode::Comfortable => 4,
        };

        width.saturating_sub(reserved).max(24)
    }

    fn current_view(&self, list_width: u16) -> SectionView {
        match self.focus {
            TuiSection::Tasks => self.tasks_view(list_width),
            TuiSection::Learnings => self.learnings_view(list_width),
            TuiSection::Blackboard => self.blackboard_view(list_width),
            TuiSection::Mesh => self.mesh_view(list_width),
            TuiSection::Feed => self.feed_view(list_width),
        }
    }

    fn tasks_view(&self, list_width: u16) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Tasks);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Tasks.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.task_row(&self.snapshot.tasks[*idx], list_width))
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

    fn learnings_view(&self, list_width: u16) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Learnings);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Learnings.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.learning_row(&self.snapshot.learnings[*idx], list_width))
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

    fn blackboard_view(&self, list_width: u16) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Blackboard);
        let selected = pick_selected(
            filtered.len(),
            self.selected[TuiSection::Blackboard.index()],
        );

        let rows = filtered
            .iter()
            .map(|idx| self.note_row(&self.snapshot.notes[*idx], list_width))
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

    fn mesh_view(&self, list_width: u16) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Mesh);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Mesh.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.agent_row(&self.snapshot.agents[*idx], list_width))
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

    fn feed_view(&self, list_width: u16) -> SectionView {
        let filtered = self.filtered_indices(TuiSection::Feed);
        let selected = pick_selected(filtered.len(), self.selected[TuiSection::Feed.index()]);

        let rows = filtered
            .iter()
            .map(|idx| self.feed_row(&self.snapshot.feed[*idx], list_width))
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

    fn task_row(&self, task: &Task, list_width: u16) -> Line<'static> {
        let id = TaskId::from(task.id).to_string();
        let blocked = self.snapshot.blocked_ids.contains(&task.id);
        let priority = task
            .planning
            .priority
            .map(|priority| priority.to_string())
            .unwrap_or_else(|| "none".to_string());
        let assignee = truncate_display(
            task.assignee.as_deref().unwrap_or("unassigned"),
            self.adaptive_list_limit(list_width, 10, 14),
        );
        let title = truncate_display(&task.title, self.adaptive_list_limit(list_width, 24, 56));

        let mut spans = vec![
            self.badge(id, self.theme.badge_neutral),
            Span::raw(" "),
            self.badge(
                if blocked { "blocked" } else { "ready" },
                if blocked {
                    self.theme.badge_blocked
                } else {
                    self.theme.badge_ready
                },
            ),
            Span::raw(" "),
            self.badge(
                task.status.to_string(),
                self.theme.status_style(task.status),
            ),
            Span::raw(" "),
            self.badge(task.kind.to_string(), self.theme.kind_style(task.kind)),
            Span::raw(" "),
            self.badge(
                format!("p:{priority}"),
                self.theme.priority_style(task.planning.priority),
            ),
            Span::raw(" "),
        ];

        spans.extend(self.highlighted_text(&format!("@{} ", assignee), self.theme.muted));
        spans.extend(self.highlighted_text(&title, self.theme.text));

        Line::from(spans)
    }

    fn learning_row(&self, learning: &Learning, list_width: u16) -> Line<'static> {
        let title = truncate_display(
            &learning.title,
            self.adaptive_list_limit(list_width, 26, 60),
        );
        let mut spans = vec![
            self.badge(format!("#{}", learning.id), self.theme.badge_neutral),
            Span::raw(" "),
            self.badge(learning.category.to_string(), self.theme.kind_meta),
            Span::raw(" "),
            self.badge(
                format!("tasks:{}", learning.task_ids.len()),
                self.theme.priority_medium,
            ),
            Span::raw(" "),
        ];

        spans.extend(self.highlighted_text(&title, self.theme.text));
        Line::from(spans)
    }

    fn note_row(&self, note: &DbNote, list_width: u16) -> Line<'static> {
        let author = format!(
            "{} ",
            truncate_display(
                &note.from_agent,
                self.adaptive_list_limit(list_width, 10, 16)
            ),
        );
        let message = truncate_display(
            note.message.lines().next().unwrap_or_default(),
            self.adaptive_list_limit(list_width, 24, 56),
        );

        let mut spans = vec![
            self.badge(format!("B{}", note.id), self.theme.badge_neutral),
            Span::raw(" "),
            self.badge(
                note.status.clone(),
                self.theme.note_status_style(note.status.as_str()),
            ),
            Span::raw(" "),
        ];

        spans.extend(self.highlighted_text(&author, self.theme.muted));
        spans.extend(self.highlighted_text(&message, self.theme.text));

        Line::from(spans)
    }

    fn agent_row(&self, agent: &DbRegistration, list_width: u16) -> Line<'static> {
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

        let inbox_style = if inbox > 0 {
            self.theme.badge_warning
        } else {
            self.theme.badge_ready
        };
        let reservation_style = if reservations > 0 {
            self.theme.kind_feature
        } else {
            self.theme.muted
        };

        let mut spans = vec![
            self.badge(
                truncate_display(&agent.name, self.adaptive_list_limit(list_width, 12, 20)),
                self.theme.badge_neutral,
            ),
            Span::raw(" "),
            self.badge(
                truncate_display(&agent.status, self.adaptive_list_limit(list_width, 6, 8)),
                self.theme.agent_status_style(&agent.status),
            ),
            Span::raw(" "),
            self.badge(format!("inbox:{inbox}"), inbox_style),
            Span::raw(" "),
            self.badge(format!("resv:{reservations}"), reservation_style),
            Span::raw(" "),
        ];

        spans.extend(self.highlighted_text(
            &format!(
                "session={} ",
                truncate_display(
                    &agent.session_id,
                    self.adaptive_list_limit(list_width, 10, 16)
                )
            ),
            self.theme.muted,
        ));

        Line::from(spans)
    }

    fn feed_row(&self, event: &DbEvent, list_width: u16) -> Line<'static> {
        let preview = event
            .preview
            .as_deref()
            .or(event.target.as_deref())
            .unwrap_or_default();

        let preview = truncate_display(preview, self.adaptive_list_limit(list_width, 24, 52));
        let mut spans = vec![
            self.badge(format!("#{}", event.id), self.theme.badge_neutral),
            Span::raw(" "),
            self.badge(
                truncate_display(
                    &event.event_type,
                    self.adaptive_list_limit(list_width, 10, 16),
                ),
                self.theme.feed_event_style(&event.event_type),
            ),
            Span::raw(" "),
            self.badge(
                truncate_display(
                    event.agent.as_deref().unwrap_or("-"),
                    self.adaptive_list_limit(list_width, 10, 14),
                ),
                self.theme.kind_meta,
            ),
            Span::raw(" "),
        ];

        spans.extend(self.highlighted_text(&preview, self.theme.text));
        Line::from(spans)
    }

    fn query_matches_text(&self, text: &str) -> bool {
        let tokens = self.query_tokens();
        if tokens.is_empty() {
            return false;
        }

        let normalized = text.to_ascii_lowercase();
        tokens.iter().any(|token| normalized.contains(token))
    }

    fn highlighted_text(&self, text: &str, base_style: Style) -> Vec<Span<'static>> {
        let tokens = self.query_tokens();
        highlight_text_spans_with_tokens(text, &tokens, base_style, self.theme.match_highlight)
    }

    fn badge<T: Into<String>>(&self, label: T, style: Style) -> Span<'static> {
        let label = label.into();
        let mut badge_style = style.add_modifier(Modifier::BOLD);

        if self.query_matches_text(&label) {
            badge_style = badge_style.patch(self.theme.match_highlight);
        }

        Span::styled(format!("[{label}]"), badge_style)
    }

    fn key_chip<T: Into<String>>(&self, label: T) -> Span<'static> {
        Span::styled(
            format!(" {} ", label.into()),
            self.theme.badge_neutral.add_modifier(Modifier::BOLD),
        )
    }

    fn detail_lines(&self, detail: &str, detail_width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut has_content = false;
        let detail_limit = self.adaptive_detail_limit(detail_width);

        for raw_line in detail.lines() {
            if raw_line.trim().is_empty() {
                lines.push(Line::default());
                continue;
            }

            if is_detail_heading(raw_line) {
                if has_content {
                    lines.push(Line::from(Span::styled(
                        "─".repeat(42),
                        self.theme.detail_separator,
                    )));
                }

                let mut heading_spans = vec![Span::styled("▸ ", self.theme.detail_separator)];
                let heading = truncate_display(
                    raw_line.trim_end_matches(':').trim(),
                    detail_limit.saturating_sub(2),
                );
                heading_spans.extend(self.highlighted_text(&heading, self.theme.detail_heading));
                lines.push(Line::from(heading_spans));
                has_content = true;
                continue;
            }

            if let Some((indent, key, value)) = parse_detail_key_value(raw_line) {
                let mut spans = vec![Span::styled(indent.clone(), self.theme.muted)];
                let padded_key = format!("{key:<width$}", width = DETAIL_KEY_WIDTH);
                spans.extend(self.highlighted_text(&padded_key, self.theme.detail_key));
                spans.push(Span::styled(": ", self.theme.detail_key));

                let value_budget = detail_limit
                    .saturating_sub(indent.len())
                    .saturating_sub(DETAIL_KEY_WIDTH + 2)
                    .max(8);
                let truncated_value = truncate_display(&value, value_budget);
                spans.extend(self.highlighted_text(&truncated_value, self.theme.detail_value));
                lines.push(Line::from(spans));
                has_content = true;
                continue;
            }

            let body_style = if raw_line.trim_start().starts_with("- ") {
                self.theme.text
            } else {
                self.theme.detail_value
            };

            let truncated_line = truncate_display(raw_line, detail_limit);
            lines.push(Line::from(
                self.highlighted_text(&truncated_line, body_style),
            ));
            has_content = true;
        }

        if lines.is_empty() {
            vec![Line::from(Span::styled("<empty>", self.theme.muted))]
        } else {
            lines
        }
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

    fn summary_cards(&self, status: TaskStatusCounts, refreshed_at: &str) -> Vec<Line<'static>> {
        let trend = status_distribution_sparkline(status);

        let mut lines = vec![
            Line::from(vec![
                self.badge(
                    format!("tasks:{}", self.snapshot.tasks.len()),
                    self.theme.badge_neutral,
                ),
                Span::raw(" "),
                self.badge(
                    format!("pending:{}", status.pending),
                    self.theme.status_style(Status::Pending),
                ),
                Span::raw(" "),
                self.badge(
                    format!("in-progress:{}", status.in_progress),
                    self.theme.status_style(Status::InProgress),
                ),
                Span::raw(" "),
                self.badge(
                    format!("done:{}", status.done),
                    self.theme.status_style(Status::Done),
                ),
                Span::raw(" "),
                self.badge(
                    format!("cancelled:{}", status.cancelled),
                    self.theme.status_style(Status::Cancelled),
                ),
            ]),
            Line::from(vec![
                self.badge(
                    format!("learnings:{}", self.snapshot.learnings.len()),
                    self.theme.kind_meta,
                ),
                Span::raw(" "),
                self.badge(
                    format!("notes:{}", self.snapshot.notes.len()),
                    self.theme.note_status_style("open"),
                ),
                Span::raw(" "),
                self.badge(
                    format!("agents:{}", self.snapshot.agents.len()),
                    self.theme.agent_status_style("active"),
                ),
                Span::raw(" "),
                self.badge(
                    format!("reservations:{}", self.snapshot.reservations.len()),
                    self.theme.kind_feature,
                ),
                Span::raw(" "),
                self.badge(
                    format!("feed:{}", self.snapshot.feed.len()),
                    self.theme.section_style(TuiSection::Feed),
                ),
                Span::raw(" "),
                self.badge(format!("trend:{trend}"), self.theme.badge_neutral),
            ]),
        ];

        let mut health_line = vec![
            self.badge(
                format!("refreshes:{}", self.refresh_count),
                self.theme.badge_neutral,
            ),
            Span::raw(" "),
            self.badge(format!("last:{}", refreshed_at), self.theme.muted),
            Span::raw(" "),
        ];

        if let Some(error) = &self.last_error {
            health_line.push(self.badge("ERROR", self.theme.badge_warning));
            health_line.push(Span::raw(" "));
            health_line.push(Span::styled(
                truncate_display(error, 72),
                self.theme.badge_warning.add_modifier(Modifier::BOLD),
            ));
        } else {
            health_line.push(self.badge("health:ok", self.theme.badge_ready));
        }

        lines.push(Line::from(health_line));
        lines
    }

    fn controls_line(&self) -> Line<'static> {
        let query = if self.query.is_empty() {
            "<none>".to_string()
        } else {
            self.query.clone()
        };

        if self.search_mode {
            Line::from(vec![
                self.badge("SEARCH", self.theme.badge_warning),
                Span::raw(" "),
                Span::styled("type query ", self.theme.text),
                self.key_chip("Enter/Esc"),
                Span::styled(" close ", self.theme.muted),
                self.key_chip("Backspace"),
                Span::styled(" delete ", self.theme.muted),
                Span::styled(format!("query={query}  "), self.theme.muted),
                self.badge(
                    format!("density:{}", self.density_mode.label()),
                    self.theme.muted,
                ),
            ])
        } else {
            Line::from(vec![
                self.key_chip("q"),
                Span::styled(" quit ", self.theme.muted),
                self.key_chip("←/→ h/l"),
                Span::styled(" tabs ", self.theme.muted),
                self.key_chip("↑/↓ j/k"),
                Span::styled(" select ", self.theme.muted),
                self.key_chip("PgUp/PgDn"),
                Span::styled(" detail ", self.theme.muted),
                self.key_chip("/"),
                Span::styled(" search ", self.theme.muted),
                self.key_chip("c"),
                Span::styled(" clear ", self.theme.muted),
                self.key_chip("r"),
                Span::styled(" refresh ", self.theme.muted),
                self.key_chip("z"),
                Span::styled(" density ", self.theme.muted),
                self.key_chip("?"),
                Span::styled(" help ", self.theme.muted),
                Span::styled(format!("query={query}  "), self.theme.muted),
                self.badge(
                    format!("density:{}", self.density_mode.label()),
                    self.theme.muted,
                ),
            ])
        }
    }

    fn help_heading<T: Into<String>>(&self, title: T) -> Line<'static> {
        Line::from(vec![
            Span::styled("▸ ", self.theme.detail_separator),
            Span::styled(
                title.into(),
                self.theme.detail_heading.add_modifier(Modifier::BOLD),
            ),
        ])
    }

    fn help_key_row<T: Into<String>, U: Into<String>>(
        &self,
        key: T,
        description: U,
    ) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!("  {:<28}", key.into()),
                self.theme.badge_neutral.add_modifier(Modifier::BOLD),
            ),
            Span::styled(description.into(), self.theme.text),
        ])
    }

    fn help_lines(&self) -> Vec<Line<'static>> {
        vec![
            self.help_heading("Navigation"),
            self.help_key_row("Tab / Shift+Tab / h,l", "switch section"),
            self.help_key_row("↑/↓ / j,k", "move selection"),
            self.help_key_row("Home/End (g/G)", "jump top/bottom"),
            self.help_key_row("PgUp/PgDn (u/d)", "scroll detail pane"),
            Line::default(),
            self.help_heading("Search"),
            self.help_key_row("/", "enter search mode"),
            self.help_key_row("type / Backspace", "edit query"),
            self.help_key_row("Enter or Esc", "exit search mode"),
            self.help_key_row("c", "clear query"),
            Line::default(),
            self.help_heading("Actions"),
            self.help_key_row("r", "refresh snapshot now"),
            self.help_key_row("z", "toggle compact/comfortable density"),
            self.help_key_row("?", "toggle help popup"),
            self.help_key_row("q", "quit"),
            Line::default(),
            self.help_heading("Data coverage"),
            self.help_key_row("Tasks", "context/history/verification/artifacts"),
            self.help_key_row("Learnings", "linked lessons and tags"),
            self.help_key_row("Blackboard", "coordination notes"),
            self.help_key_row("Mesh", "agents/inbox/reservations"),
            self.help_key_row("Feed", "recent coordination events"),
        ]
    }

    fn render(&self, frame: &mut Frame) {
        let summary_height = match self.density_mode {
            DensityMode::Compact => 4,
            DensityMode::Comfortable => 5,
        };

        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(summary_height),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let panel_focus = if self.search_mode {
            self.theme.panel_search_border
        } else {
            self.theme.panel_focus_border(self.focus)
        };

        let tab_titles = TuiSection::ALL
            .iter()
            .map(|section| {
                let style = if *section == self.focus {
                    self.theme
                        .section_style(*section)
                        .add_modifier(Modifier::BOLD)
                } else {
                    self.theme.tab_idle
                };
                Line::from(Span::styled(section.label(), style))
            })
            .collect::<Vec<_>>();

        frame.render_widget(
            Tabs::new(tab_titles)
                .select(self.focus.index())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if self.search_mode {
                            "tak tui • SEARCH"
                        } else {
                            "tak tui"
                        })
                        .border_style(panel_focus)
                        .title_style(self.theme.muted),
                )
                .highlight_style(self.theme.tab_active),
            outer[0],
        );

        let status = self.snapshot.status_counts();
        let refreshed_at = self
            .last_refreshed_at
            .as_ref()
            .map(|timestamp| timestamp.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());

        frame.render_widget(
            Paragraph::new(self.summary_cards(status, &refreshed_at))
                .style(self.theme.text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Summary")
                        .border_style(self.theme.panel_idle_border)
                        .title_style(self.theme.muted),
                )
                .wrap(Wrap { trim: true }),
            outer[1],
        );

        let center = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(outer[2]);

        let view = self.current_view(center[0].width);
        self.render_list(frame, center[0], &view, panel_focus);

        frame.render_widget(
            Paragraph::new(self.detail_lines(&view.detail, center[1].width))
                .style(self.theme.text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(view.detail_title)
                        .border_style(panel_focus)
                        .title_style(panel_focus),
                )
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll.min(u16::MAX as usize) as u16, 0)),
            center[1],
        );

        let controls_border = if self.search_mode {
            self.theme.panel_search_border
        } else {
            self.theme.panel_idle_border
        };

        frame.render_widget(
            Paragraph::new(self.controls_line())
                .style(self.theme.text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if self.search_mode {
                            "Search mode"
                        } else {
                            "Controls"
                        })
                        .border_style(controls_border)
                        .title_style(controls_border),
                )
                .wrap(Wrap { trim: true }),
            outer[3],
        );

        if self.help_visible {
            let popup = centered_rect(86, 74, frame.area());
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(self.help_lines())
                    .style(self.theme.text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Help")
                            .border_style(panel_focus)
                            .title_style(panel_focus),
                    )
                    .wrap(Wrap { trim: true }),
                popup,
            );
        }
    }

    fn render_list(&self, frame: &mut Frame, area: Rect, view: &SectionView, border_style: Style) {
        if view.rows.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("(no results)", self.theme.muted)))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(view.list_title.as_str())
                            .border_style(border_style)
                            .title_style(border_style),
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
            .cloned()
            .map(ListItem::new)
            .collect::<Vec<_>>();

        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(view.list_title.as_str())
                        .border_style(border_style)
                        .title_style(border_style),
                )
                .highlight_style(self.theme.section_highlight_style(self.focus))
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

fn status_distribution_sparkline(status: TaskStatusCounts) -> String {
    let values = [
        status.pending,
        status.in_progress,
        status.done,
        status.cancelled,
    ];
    let max = values.into_iter().max().unwrap_or(0);

    let glyph_for = |count: usize| {
        if max == 0 {
            '·'
        } else {
            const GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let idx = ((count.saturating_mul(7)) + max.saturating_sub(1)) / max;
            GLYPHS[idx.min(GLYPHS.len() - 1)]
        }
    };

    format!(
        "P{}I{}D{}C{}",
        glyph_for(status.pending),
        glyph_for(status.in_progress),
        glyph_for(status.done),
        glyph_for(status.cancelled),
    )
}

fn contains_all_tokens(candidate: &str, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }

    let normalized = candidate.to_ascii_lowercase();
    tokens.iter().all(|token| normalized.contains(token))
}

fn is_detail_heading(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with(':') && !trimmed.contains(": ")
}

fn parse_detail_key_value(line: &str) -> Option<(String, String, String)> {
    let trimmed_start = line.trim_start();
    if trimmed_start.starts_with("- ") || trimmed_start.starts_with("...") {
        return None;
    }

    let (key, value) = trimmed_start.split_once(": ")?;
    if key.is_empty() {
        return None;
    }

    let indent_len = line.len().saturating_sub(trimmed_start.len());
    let indent = " ".repeat(indent_len);

    Some((indent, key.to_string(), value.to_string()))
}

fn highlight_text_spans_with_tokens(
    text: &str,
    tokens: &[String],
    base_style: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    if text.is_empty() || tokens.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let normalized = text.to_ascii_lowercase();
    let mut ranges = Vec::new();

    for token in tokens.iter().filter(|token| !token.is_empty()) {
        let mut offset = 0;
        while let Some(found) = normalized[offset..].find(token.as_str()) {
            let start = offset + found;
            let end = start + token.len();
            ranges.push((start, end));
            offset = end;
        }
    }

    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    ranges.sort_unstable_by_key(|(start, _)| *start);

    let mut merged_ranges = Vec::new();
    for (start, end) in ranges {
        if let Some((_, merged_end)) = merged_ranges.last_mut()
            && start <= *merged_end
        {
            *merged_end = (*merged_end).max(end);
            continue;
        }

        merged_ranges.push((start, end));
    }

    let mut spans = Vec::new();
    let mut cursor = 0;

    for (start, end) in merged_ranges {
        if start > cursor {
            spans.push(Span::styled(text[cursor..start].to_string(), base_style));
        }

        spans.push(Span::styled(
            text[start..end].to_string(),
            base_style.patch(match_style),
        ));
        cursor = end;
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    spans
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
        Priority,
    };
    use crate::store::coordination_db::{
        DbEvent, DbMessage, DbNote, DbRegistration, DbReservation,
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

    fn note(id: i64, from_agent: &str, status: &str, message: &str) -> DbNote {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        DbNote {
            id,
            from_agent: from_agent.to_string(),
            message: message.to_string(),
            status: status.to_string(),
            note_type: None,
            supersedes_note_id: None,
            superseded_by_note_id: None,
            tags: vec!["coordination".to_string()],
            task_ids: vec![TaskId::from(1_u64).to_string()],
            created_at: ts,
            updated_at: ts,
            closed_by: None,
            closed_reason: None,
            closed_at: None,
        }
    }

    fn agent(name: &str, status: &str) -> DbRegistration {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        DbRegistration {
            name: name.to_string(),
            generation: 1,
            session_id: "session-1234567890".to_string(),
            cwd: "/tmp/tak".to_string(),
            pid: Some(1234),
            host: None,
            status: status.to_string(),
            started_at: ts,
            updated_at: ts,
            metadata: "{}".to_string(),
        }
    }

    fn inbox_message(from_agent: &str, to_agent: &str, text: &str) -> DbMessage {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        DbMessage {
            id: "msg-1".to_string(),
            from_agent: from_agent.to_string(),
            to_agent: to_agent.to_string(),
            text: text.to_string(),
            reply_to: None,
            created_at: ts,
            read_at: None,
            acked_at: None,
        }
    }

    fn reservation(agent: &str, path: &str) -> DbReservation {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        DbReservation {
            id: 1,
            agent: agent.to_string(),
            generation: 1,
            path: path.to_string(),
            reason: Some("editing".to_string()),
            created_at: ts,
            expires_at: ts,
        }
    }

    fn feed_event(id: i64, event_type: &str, agent: Option<&str>, preview: &str) -> DbEvent {
        let ts = Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).single().unwrap();
        DbEvent {
            id,
            agent: agent.map(str::to_string),
            event_type: event_type.to_string(),
            target: Some("src/commands/tui.rs".to_string()),
            preview: Some(preview.to_string()),
            detail: None,
            created_at: ts,
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

        let mesh_agent = agent("mesh-agent", "active");
        app.snapshot.notes = vec![note(
            1,
            "mesh-agent",
            "open",
            "Status update from mesh agent",
        )];
        app.snapshot.agents = vec![mesh_agent.clone()];
        app.snapshot.agent_inbox = HashMap::from([(
            mesh_agent.name.clone(),
            vec![inbox_message("peer", &mesh_agent.name, "please sync")],
        )]);
        app.snapshot.reservations = vec![reservation(&mesh_agent.name, "src/commands/tui.rs")];
        app.snapshot.feed = vec![feed_event(
            1,
            "reserve",
            Some(&mesh_agent.name),
            "Reserved src/commands/tui.rs",
        )];

        app
    }

    fn has_match_highlight(spans: &[Span<'_>]) -> bool {
        spans.iter().any(|span| {
            span.style.bg == Some(Color::Yellow)
                && span.style.add_modifier.contains(Modifier::UNDERLINED)
        })
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
    fn density_toggle_switches_modes() {
        let mut app = app_with_snapshot();
        assert_eq!(app.density_mode, DensityMode::Comfortable);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('z')));
        assert_eq!(app.density_mode, DensityMode::Compact);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('z')));
        assert_eq!(app.density_mode, DensityMode::Comfortable);
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
    fn row_renderers_include_semantic_badges() {
        let app = app_with_snapshot();

        let task_line = app.task_row(&app.snapshot.tasks[0], 80).to_string();
        assert!(task_line.contains("[pending]"));
        assert!(task_line.contains("[task]"));
        assert!(task_line.contains("[p:none]"));

        let note_line = app.note_row(&app.snapshot.notes[0], 80).to_string();
        assert!(note_line.contains("[B1]"));
        assert!(note_line.contains("[open]"));

        let agent_line = app.agent_row(&app.snapshot.agents[0], 80).to_string();
        assert!(agent_line.contains("[active]"));
        assert!(agent_line.contains("[inbox:1]"));
        assert!(agent_line.contains("[resv:1]"));

        let feed_line = app.feed_row(&app.snapshot.feed[0], 80).to_string();
        assert!(feed_line.contains("[#1]"));
        assert!(feed_line.contains("[reserve]"));
    }

    #[test]
    fn row_renderers_highlight_query_matches() {
        let mut app = app_with_snapshot();
        app.query = "alpha".to_string();

        let task_line = app.task_row(&app.snapshot.tasks[0], 80);
        assert!(has_match_highlight(&task_line.spans));

        let learning_line = app.learning_row(&app.snapshot.learnings[0], 80);
        assert!(has_match_highlight(&learning_line.spans));
    }

    #[test]
    fn detail_lines_highlight_query_matches() {
        let mut app = app_with_snapshot();
        app.query = "alpha".to_string();

        let lines = app.detail_lines("title: Alpha task\n\ndescription:\n  alpha note", 100);
        assert!(lines.iter().any(|line| has_match_highlight(&line.spans)));
    }

    #[test]
    fn row_truncation_adapts_to_width_and_density() {
        let mut app = app_with_snapshot();

        let wide = app.task_row(&app.snapshot.tasks[0], 120).to_string();
        let narrow = app.task_row(&app.snapshot.tasks[0], 52).to_string();
        assert!(wide.len() >= narrow.len());

        app.density_mode = DensityMode::Compact;
        let compact = app.task_row(&app.snapshot.tasks[0], 80).to_string();
        app.density_mode = DensityMode::Comfortable;
        let comfortable = app.task_row(&app.snapshot.tasks[0], 80).to_string();
        assert!(comfortable.len() >= compact.len());
    }

    #[test]
    fn controls_line_switches_to_search_mode_chip() {
        let mut app = app_with_snapshot();
        app.query = "alpha".to_string();

        let idle = app.controls_line().to_string();
        assert!(idle.contains("tabs"));
        assert!(idle.contains("density"));
        assert!(idle.contains("query=alpha"));

        app.search_mode = true;
        let search = app.controls_line().to_string();
        assert!(search.contains("[SEARCH]"));
        assert!(search.contains("density:comfortable"));
        assert!(search.contains("query=alpha"));
    }

    #[test]
    fn summary_cards_render_grouped_metric_chips() {
        let app = app_with_snapshot();
        let cards = app.summary_cards(app.snapshot.status_counts(), "12:34:56");

        assert!(cards.len() >= 3);

        let task_card_row = cards[0].to_string();
        assert!(task_card_row.contains("[tasks:2]"));
        assert!(task_card_row.contains("[pending:2]"));
        assert!(task_card_row.contains("[done:0]"));

        let domain_card_row = cards[1].to_string();
        assert!(domain_card_row.contains("[learnings:1]"));
        assert!(domain_card_row.contains("[notes:1]"));
        assert!(domain_card_row.contains("[agents:1]"));
        assert!(domain_card_row.contains("[feed:1]"));
        assert!(domain_card_row.contains("[trend:P"));

        let health_card_row = cards[2].to_string();
        assert!(health_card_row.contains("[refreshes:0]"));
        assert!(health_card_row.contains("[health:ok]"));
    }

    #[test]
    fn summary_cards_surface_warning_chip_when_error_present() {
        let mut app = app_with_snapshot();
        app.last_error = Some("coordination db unavailable".to_string());

        let cards = app.summary_cards(app.snapshot.status_counts(), "12:34:56");
        let health_card_row = cards[2].to_string();

        assert!(health_card_row.contains("[ERROR]"));
        assert!(health_card_row.contains("coordination db unavailable"));
    }

    #[test]
    fn status_distribution_sparkline_is_compact_for_narrow_terminals() {
        let spark = status_distribution_sparkline(TaskStatusCounts {
            pending: 12,
            in_progress: 6,
            done: 3,
            cancelled: 0,
        });

        assert!(spark.starts_with('P'));
        assert!(spark.contains('I'));
        assert!(spark.contains('D'));
        assert!(spark.contains('C'));
        assert!(spark.chars().count() <= 8);
    }

    #[test]
    fn detail_lines_apply_section_segmentation_and_key_alignment() {
        let app = app_with_snapshot();
        let detail = "id: 1\nstatus: pending\n\ndescription:\n  first line\n  second line";

        let rendered = app
            .detail_lines(detail, 100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("id")));
        assert!(rendered.iter().any(|line| line.contains(": 1")));
        assert!(rendered.iter().any(|line| line.contains("▸ description")));
        assert!(rendered.iter().any(|line| line.contains('─')));
    }

    #[test]
    fn detail_lines_keep_task_detail_scrollable_but_structured() {
        let app = app_with_snapshot();
        let detail = app.task_detail(&app.snapshot.tasks[0]);

        let rendered = app
            .detail_lines(&detail, 100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.len() > 20);
        assert!(rendered.iter().any(|line| line.contains("▸ description")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("▸ contract.objective"))
        );
    }

    #[test]
    fn help_popup_uses_grouped_keymap_sections() {
        let app = app_with_snapshot();
        let help = app
            .help_lines()
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(help.iter().any(|line| line.contains("▸ Navigation")));
        assert!(help.iter().any(|line| line.contains("▸ Search")));
        assert!(help.iter().any(|line| line.contains("▸ Actions")));
        assert!(help.iter().any(|line| line.contains("▸ Data coverage")));
        assert!(
            help.iter()
                .any(|line| line.contains("Tab / Shift+Tab / h,l"))
        );
        assert!(help.iter().any(|line| line.contains("switch section")));
        assert!(
            help.iter()
                .any(|line| line.contains("toggle compact/comfortable density"))
        );
    }

    #[test]
    fn theme_maps_status_priority_and_kind_semantically() {
        let theme = TuiTheme::default();

        assert_ne!(
            theme.status_style(Status::Pending),
            theme.status_style(Status::Done)
        );
        assert_ne!(
            theme.priority_style(Some(Priority::Critical)),
            theme.priority_style(Some(Priority::Low))
        );
        assert_ne!(theme.kind_style(Kind::Epic), theme.kind_style(Kind::Task));
        assert_eq!(theme.match_highlight.bg, Some(Color::Yellow));
        assert!(
            theme
                .match_highlight
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
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
