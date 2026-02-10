use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration, Utc};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::error::Result;
use crate::metrics::{
    CompletionMetric, MetricsBucket, MetricsFilters, MetricsQuery, MetricsWindow,
    aggregate_burndown, aggregate_completion_time, derive_timelines,
};
use crate::model::{Status, Task};
use crate::store::repo::Repo;
use crate::store::sidecars::HistoryEvent;

#[derive(Debug, Clone)]
pub struct MetricsTuiConfig {
    pub query: MetricsQuery,
    pub metric: CompletionMetric,
    pub tick_rate: StdDuration,
}

impl Default for MetricsTuiConfig {
    fn default() -> Self {
        let to = Utc::now().date_naive();
        let from = to - Duration::days(30);
        Self {
            query: MetricsQuery {
                window: MetricsWindow { from, to },
                bucket: MetricsBucket::Day,
                filters: MetricsFilters::default(),
            },
            metric: CompletionMetric::Cycle,
            tick_rate: StdDuration::from_millis(250),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MetricsSnapshot {
    total_tasks: usize,
    done_tasks: usize,
    burndown_points: usize,
    burndown_start_remaining: usize,
    burndown_end_remaining: usize,
    burndown_completed_in_window: usize,
    burndown_reopened_in_window: usize,
    burndown_scope_added_total: usize,
    burndown_scope_removed_total: usize,
    burndown_actual_sparkline: String,
    burndown_ideal_sparkline: String,
    completion_points: usize,
    completion_summary_avg_hours: Option<f64>,
    completion_summary_samples: usize,
    completion_latest_bucket: Option<String>,
    completion_latest_avg_hours: Option<f64>,
    completion_sparkline: String,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct MetricsTuiApp {
    query: MetricsQuery,
    metric: CompletionMetric,
    tick_rate: StdDuration,
    help_visible: bool,
    needs_refresh: bool,
    refresh_count: u32,
    last_refreshed_at: Option<chrono::DateTime<Utc>>,
    snapshot: MetricsSnapshot,
}

impl MetricsTuiApp {
    fn new(mut config: MetricsTuiConfig) -> Self {
        config.query.normalize();
        Self {
            query: config.query,
            metric: config.metric,
            tick_rate: config.tick_rate,
            help_visible: false,
            needs_refresh: true,
            refresh_count: 0,
            last_refreshed_at: None,
            snapshot: MetricsSnapshot::default(),
        }
    }

    fn refresh(&mut self, repo_root: &Path) {
        match refresh_snapshot(repo_root, &self.query, self.metric) {
            Ok(snapshot) => self.snapshot = snapshot,
            Err(err) => self.snapshot.last_error = Some(err.to_string()),
        }

        self.refresh_count += 1;
        self.last_refreshed_at = Some(Utc::now());
        self.needs_refresh = false;
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }

        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('r') => {
                self.needs_refresh = true;
                false
            }
            KeyCode::Char('b') => {
                self.query.bucket = match self.query.bucket {
                    MetricsBucket::Day => MetricsBucket::Week,
                    MetricsBucket::Week => MetricsBucket::Day,
                };
                self.needs_refresh = true;
                false
            }
            KeyCode::Char('m') => {
                self.metric = match self.metric {
                    CompletionMetric::Lead => CompletionMetric::Cycle,
                    CompletionMetric::Cycle => CompletionMetric::Lead,
                };
                self.needs_refresh = true;
                false
            }
            KeyCode::Char('[') => {
                shrink_window(&mut self.query.window, self.query.bucket);
                self.needs_refresh = true;
                false
            }
            KeyCode::Char(']') => {
                expand_window(&mut self.query.window, self.query.bucket);
                self.needs_refresh = true;
                false
            }
            KeyCode::Char('?') => {
                self.help_visible = !self.help_visible;
                false
            }
            _ => false,
        }
    }

    fn render(&self, frame: &mut Frame) {
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let header = format!(
            "window={}..{}  bucket={}  metric={}  refresh_count={}  last_refresh={}",
            self.query.window.from,
            self.query.window.to,
            self.query.bucket,
            self.metric,
            self.refresh_count,
            self.last_refreshed_at
                .as_ref()
                .map(|ts| ts.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string())
        );

        frame.render_widget(
            Paragraph::new(header)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("tak metrics tui"),
                )
                .wrap(Wrap { trim: true }),
            vertical[0],
        );

        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(vertical[1]);

        let burndown_panel = format!(
            "Burndown panel\n\nTotal tasks: {}\nDone tasks: {}\nSeries buckets: {}\nRemaining: {} -> {}\nCompleted/Reopened: {}/{}\nScope +/−: {}/{}\nActual: {}\nIdeal: {}\n{}",
            self.snapshot.total_tasks,
            self.snapshot.done_tasks,
            self.snapshot.burndown_points,
            self.snapshot.burndown_start_remaining,
            self.snapshot.burndown_end_remaining,
            self.snapshot.burndown_completed_in_window,
            self.snapshot.burndown_reopened_in_window,
            self.snapshot.burndown_scope_added_total,
            self.snapshot.burndown_scope_removed_total,
            if self.snapshot.burndown_actual_sparkline.is_empty() {
                "-"
            } else {
                self.snapshot.burndown_actual_sparkline.as_str()
            },
            if self.snapshot.burndown_ideal_sparkline.is_empty() {
                "-"
            } else {
                self.snapshot.burndown_ideal_sparkline.as_str()
            },
            self.snapshot
                .last_error
                .as_ref()
                .map(|err| format!("Last refresh error: {err}"))
                .unwrap_or_else(|| "Refresh: OK".to_string())
        );
        frame.render_widget(
            Paragraph::new(burndown_panel)
                .block(Block::default().borders(Borders::ALL).title("Burndown"))
                .wrap(Wrap { trim: true }),
            horizontal[0],
        );

        let latest_bucket = self
            .snapshot
            .completion_latest_bucket
            .as_deref()
            .unwrap_or("-");
        let completion_panel = format!(
            "Completion-time panel\n\nMetric: {}\nSeries buckets: {}\nLatest bucket: {} (avg {})\nSummary avg: {} (samples: {})\nTrend: {}",
            self.metric,
            self.snapshot.completion_points,
            latest_bucket,
            format_optional_hours(self.snapshot.completion_latest_avg_hours),
            format_optional_hours(self.snapshot.completion_summary_avg_hours),
            self.snapshot.completion_summary_samples,
            if self.snapshot.completion_sparkline.is_empty() {
                "-"
            } else {
                self.snapshot.completion_sparkline.as_str()
            },
        );
        frame.render_widget(
            Paragraph::new(completion_panel)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Completion time"),
                )
                .wrap(Wrap { trim: true }),
            horizontal[1],
        );

        frame.render_widget(
            Paragraph::new(
                "q quit | r refresh | b bucket | m metric | [ shrink | ] expand | ? help",
            )
            .block(Block::default().borders(Borders::ALL).title("Controls")),
            vertical[2],
        );

        if self.help_visible {
            let popup = centered_rect(80, 60, frame.area());
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(
                    "Metrics TUI scaffold controls:\n\n- q: quit\n- r: refresh snapshot\n- b: toggle bucket day/week\n- m: toggle completion metric lead/cycle\n- [: shrink window by one bucket\n- ]: expand window by one bucket\n- ?: toggle this help",
                )
                .block(Block::default().borders(Borders::ALL).title("Help"))
                .wrap(Wrap { trim: true }),
                popup,
            );
        }
    }
}

pub fn run_tui(repo_root: &Path, config: MetricsTuiConfig) -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = MetricsTuiApp::new(config);
    let run_result = run_loop(repo_root, &mut terminal, &mut app);

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    run_result
}

fn run_loop<B: Backend>(
    repo_root: &Path,
    terminal: &mut Terminal<B>,
    app: &mut MetricsTuiApp,
) -> Result<()> {
    app.refresh(repo_root);
    let mut last_tick = Instant::now();

    loop {
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

        if app.needs_refresh || last_tick.elapsed() >= app.tick_rate {
            app.refresh(repo_root);
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn refresh_snapshot(
    repo_root: &Path,
    query: &MetricsQuery,
    metric: CompletionMetric,
) -> Result<MetricsSnapshot> {
    let repo = Repo::open(repo_root)?;
    let tasks = filter_tasks(repo.store.list_all()?, &query.filters);

    let mut history_by_task: HashMap<u64, Vec<HistoryEvent>> = HashMap::new();
    for task in &tasks {
        history_by_task.insert(task.id, repo.sidecars.read_history(task.id)?);
    }

    let timelines = derive_timelines(&tasks, &history_by_task);
    let burndown = aggregate_burndown(&timelines, query);
    let completion = aggregate_completion_time(query, metric, &timelines);

    let burndown_actual_values: Vec<f64> = burndown
        .series
        .actual
        .iter()
        .map(|point| point.remaining as f64)
        .collect();
    let burndown_ideal_values: Vec<f64> = burndown
        .series
        .ideal
        .iter()
        .map(|point| point.remaining)
        .collect();

    let completion_values: Vec<f64> = completion
        .series
        .iter()
        .filter_map(|point| point.avg_hours)
        .collect();
    let latest = completion.series.last();

    Ok(MetricsSnapshot {
        total_tasks: tasks.len(),
        done_tasks: tasks
            .iter()
            .filter(|task| task.status == Status::Done)
            .count(),
        burndown_points: burndown.series.actual.len(),
        burndown_start_remaining: burndown.summary.start_remaining,
        burndown_end_remaining: burndown.summary.end_remaining,
        burndown_completed_in_window: burndown.summary.completed_in_window,
        burndown_reopened_in_window: burndown.summary.reopened_in_window,
        burndown_scope_added_total: burndown.series.scope_added.iter().map(|p| p.count).sum(),
        burndown_scope_removed_total: burndown.series.scope_removed.iter().map(|p| p.count).sum(),
        burndown_actual_sparkline: build_sparkline(&burndown_actual_values),
        burndown_ideal_sparkline: build_sparkline(&burndown_ideal_values),
        completion_points: completion.series.len(),
        completion_summary_avg_hours: completion.summary.avg_hours,
        completion_summary_samples: completion.summary.samples,
        completion_latest_bucket: latest.map(|point| point.bucket.clone()),
        completion_latest_avg_hours: latest.and_then(|point| point.avg_hours),
        completion_sparkline: build_sparkline(&completion_values),
        last_error: None,
    })
}

fn filter_tasks(mut tasks: Vec<Task>, filters: &MetricsFilters) -> Vec<Task> {
    if let Some(kind) = filters.kind {
        tasks.retain(|task| task.kind == kind);
    }

    if !filters.tags.is_empty() {
        tasks.retain(|task| {
            filters
                .tags
                .iter()
                .all(|required_tag| task.tags.contains(required_tag))
        });
    }

    if let Some(ref assignee) = filters.assignee {
        tasks.retain(|task| task.assignee.as_deref() == Some(assignee.as_str()));
    }

    if let Some(parent_id) = filters.children_of {
        tasks.retain(|task| task.parent == Some(parent_id));
    }

    if !filters.include_cancelled {
        tasks.retain(|task| task.status != Status::Cancelled);
    }

    tasks
}

fn format_optional_hours(value: Option<f64>) -> String {
    value
        .map(|hours| format!("{hours:.1}h"))
        .unwrap_or_else(|| "-".to_string())
}

fn build_sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }

    const TICKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    let min = values
        .iter()
        .fold(f64::INFINITY, |acc, value| acc.min(*value));
    let max = values
        .iter()
        .fold(f64::NEG_INFINITY, |acc, value| acc.max(*value));

    if (max - min).abs() < f64::EPSILON {
        return std::iter::repeat(TICKS[3]).take(values.len()).collect();
    }

    values
        .iter()
        .map(|value| {
            let normalized = (value - min) / (max - min);
            let idx = (normalized * (TICKS.len() - 1) as f64).round() as usize;
            TICKS[idx.min(TICKS.len() - 1)]
        })
        .collect()
}

fn bucket_span_days(bucket: MetricsBucket) -> i64 {
    match bucket {
        MetricsBucket::Day => 1,
        MetricsBucket::Week => 7,
    }
}

fn shrink_window(window: &mut MetricsWindow, bucket: MetricsBucket) {
    let step = Duration::days(bucket_span_days(bucket));
    let candidate = window.from + step;
    if candidate <= window.to {
        window.from = candidate;
    }
}

fn expand_window(window: &mut MetricsWindow, bucket: MetricsBucket) {
    let step = Duration::days(bucket_span_days(bucket));
    window.from -= step;
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

    use chrono::NaiveDate;
    use crossterm::event::KeyEvent;
    use ratatui::backend::TestBackend;
    use tempfile::tempdir;

    use crate::model::{Contract, Planning};
    use crate::store::files::FileStore;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn app(bucket: MetricsBucket, metric: CompletionMetric) -> MetricsTuiApp {
        let mut config = MetricsTuiConfig::default();
        config.query.window = MetricsWindow {
            from: date("2026-02-01"),
            to: date("2026-02-10"),
        };
        config.query.bucket = bucket;
        config.metric = metric;
        MetricsTuiApp::new(config)
    }

    #[test]
    fn bucket_toggle_switches_between_day_and_week() {
        let mut app = app(MetricsBucket::Day, CompletionMetric::Cycle);
        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('b')));
        assert_eq!(app.query.bucket, MetricsBucket::Week);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('b')));
        assert_eq!(app.query.bucket, MetricsBucket::Day);
    }

    #[test]
    fn metric_toggle_switches_between_cycle_and_lead() {
        let mut app = app(MetricsBucket::Day, CompletionMetric::Cycle);
        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        assert_eq!(app.metric, CompletionMetric::Lead);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        assert_eq!(app.metric, CompletionMetric::Cycle);
    }

    #[test]
    fn bracket_controls_adjust_window_start_by_bucket() {
        let mut app = app(MetricsBucket::Week, CompletionMetric::Cycle);
        let original_from = app.query.window.from;

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char(']')));
        assert_eq!(app.query.window.from, original_from - Duration::days(7));

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('[')));
        assert_eq!(app.query.window.from, original_from);
    }

    #[test]
    fn refresh_and_help_controls_toggle_expected_flags() {
        let mut app = app(MetricsBucket::Day, CompletionMetric::Cycle);
        app.needs_refresh = false;
        app.help_visible = false;

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('r')));
        assert!(app.needs_refresh);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        assert!(app.help_visible);

        let _ = app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        assert!(!app.help_visible);
    }

    #[test]
    fn quit_control_returns_true() {
        let mut app = app(MetricsBucket::Day, CompletionMetric::Cycle);
        assert!(app.handle_key(KeyEvent::from(KeyCode::Char('q'))));
    }

    #[test]
    fn sparkline_builder_handles_empty_and_non_empty_inputs() {
        assert_eq!(build_sparkline(&[]), "");

        let sparkline = build_sparkline(&[1.0, 2.0, 3.0, 2.5]);
        assert_eq!(sparkline.chars().count(), 4);

        let flat = build_sparkline(&[2.0, 2.0, 2.0]);
        assert_eq!(flat, "▄▄▄");
    }

    #[test]
    fn format_optional_hours_formats_presence_and_absence() {
        assert_eq!(format_optional_hours(Some(3.25)), "3.2h");
        assert_eq!(format_optional_hours(None), "-");
    }

    #[test]
    fn render_smoke_draws_panels_and_help_overlay() {
        let mut app = app(MetricsBucket::Day, CompletionMetric::Cycle);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.render(frame)).unwrap();

        app.help_visible = true;
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn refresh_snapshot_smoke_on_initialized_repo() {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let _task = store
            .create(
                "Sample".into(),
                crate::model::Kind::Task,
                None,
                None,
                vec![],
                vec![],
                Contract::default(),
                Planning::default(),
            )
            .unwrap();

        let query = MetricsQuery {
            window: MetricsWindow {
                from: date("2026-02-01"),
                to: date("2026-02-10"),
            },
            bucket: MetricsBucket::Day,
            filters: MetricsFilters::default(),
        };

        let snapshot = refresh_snapshot(dir.path(), &query, CompletionMetric::Cycle).unwrap();

        assert_eq!(snapshot.total_tasks, 1);
        assert!(snapshot.burndown_points > 0);
    }
}
