use std::collections::HashMap;
use std::path::Path;

use chrono::{Duration, NaiveDate, Utc};

use crate::error::{Result, TakError};
use crate::metrics::{
    BurndownReport, CompletionMetric, CompletionTimeReport, MetricsBucket, MetricsFilters,
    MetricsQuery, MetricsTuiConfig, MetricsWindow, aggregate_burndown, aggregate_completion_time,
    derive_timelines, run_tui,
};
use crate::model::{Kind, Status, Task};
use crate::output::Format;
use crate::store::repo::Repo;

const DEFAULT_WINDOW_DAYS: i64 = 30;
const MAX_DAY_BUCKETS: i64 = 366;
const MAX_WEEK_BUCKETS: i64 = 520;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricsMode {
    Burndown,
    CompletionTime,
}

fn invalid_query(message: impl Into<String>) -> TakError {
    TakError::MetricsInvalidQuery(message.into())
}

#[allow(clippy::too_many_arguments)]
fn build_query(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    bucket: MetricsBucket,
    kind: Option<Kind>,
    tags: Vec<String>,
    assignee: Option<String>,
    children_of: Option<u64>,
    include_cancelled: bool,
    mode: MetricsMode,
) -> Result<MetricsQuery> {
    let to = to.unwrap_or_else(|| Utc::now().date_naive());
    let from = from.unwrap_or_else(|| to - Duration::days(DEFAULT_WINDOW_DAYS));

    if from > to {
        return Err(invalid_query(format!(
            "date window is inverted: --from {from} is after --to {to}"
        )));
    }

    let had_tags = !tags.is_empty();
    let had_assignee = assignee.is_some();

    let mut query = MetricsQuery {
        window: MetricsWindow { from, to },
        bucket,
        filters: MetricsFilters {
            kind,
            tags,
            assignee,
            children_of,
            include_cancelled,
        },
    };
    query.normalize();

    if had_tags && query.filters.tags.is_empty() {
        return Err(invalid_query(
            "tag filter requires at least one non-empty tag",
        ));
    }

    if had_assignee && query.filters.assignee.is_none() {
        return Err(invalid_query("assignee filter cannot be empty"));
    }

    validate_query(&query, mode)?;
    Ok(query)
}

fn bucket_count(window: MetricsWindow, bucket: MetricsBucket) -> i64 {
    let span_days = (window.to - window.from).num_days() + 1;
    match bucket {
        MetricsBucket::Day => span_days,
        MetricsBucket::Week => (span_days + 6) / 7,
    }
}

fn validate_query(query: &MetricsQuery, mode: MetricsMode) -> Result<()> {
    let buckets = bucket_count(query.window, query.bucket);

    match query.bucket {
        MetricsBucket::Day if buckets > MAX_DAY_BUCKETS => {
            return Err(invalid_query(format!(
                "day bucket window is too large ({buckets} buckets > {MAX_DAY_BUCKETS}); narrow the date range or use --bucket week"
            )));
        }
        MetricsBucket::Week if buckets > MAX_WEEK_BUCKETS => {
            return Err(invalid_query(format!(
                "week bucket window is too large ({buckets} buckets > {MAX_WEEK_BUCKETS}); narrow the date range"
            )));
        }
        _ => {}
    }

    if matches!(mode, MetricsMode::CompletionTime) && query.filters.include_cancelled {
        return Err(invalid_query(
            "--include-cancelled is only supported for `metrics burndown`",
        ));
    }

    Ok(())
}

fn format_filters(filters: &MetricsFilters) -> String {
    let mut parts = Vec::new();

    if let Some(kind) = filters.kind {
        parts.push(format!("kind={kind}"));
    }

    if !filters.tags.is_empty() {
        parts.push(format!("tags={}", filters.tags.join(",")));
    }

    if let Some(assignee) = filters.assignee.as_deref() {
        parts.push(format!("assignee={assignee}"));
    }

    if let Some(parent) = filters.children_of {
        parts.push(format!("children_of={parent}"));
    }

    if filters.include_cancelled {
        parts.push("include_cancelled=true".to_string());
    }

    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

fn format_hours_cell(value: Option<f64>) -> String {
    value
        .map(|hours| format!("{hours:.1}"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_hours_summary(value: Option<f64>) -> String {
    value
        .map(|hours| format!("{hours:.1}h"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_count_cell(value: usize) -> String {
    if value == 0 {
        "-".to_string()
    } else {
        value.to_string()
    }
}

fn render_burndown(report: &BurndownReport, format: Format) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(report)?),
        Format::Pretty => print_burndown_pretty(report),
        Format::Minimal => print_burndown_minimal(report),
    }
    Ok(())
}

fn print_burndown_pretty(report: &BurndownReport) {
    println!("Burndown metrics (bucket={})", report.bucket);
    println!("window: {} -> {}", report.window.from, report.window.to);
    println!("filters: {}", format_filters(&report.filters));
    println!(
        "summary: start={} end={} completed={} reopened={}",
        report.summary.start_remaining,
        report.summary.end_remaining,
        report.summary.completed_in_window,
        report.summary.reopened_in_window,
    );

    if !report.data_quality.is_empty() {
        println!(
            "data quality: missing_history_tasks={} inferred_samples={}",
            report.data_quality.missing_history_tasks, report.data_quality.inferred_samples,
        );
    }

    if report.series.actual.is_empty() {
        println!("series: (no points)");
        return;
    }

    let ideal_by_date: HashMap<NaiveDate, f64> = report
        .series
        .ideal
        .iter()
        .map(|point| (point.date, point.remaining))
        .collect();
    let scope_added_by_date: HashMap<NaiveDate, usize> = report
        .series
        .scope_added
        .iter()
        .map(|point| (point.date, point.count))
        .collect();
    let scope_removed_by_date: HashMap<NaiveDate, usize> = report
        .series
        .scope_removed
        .iter()
        .map(|point| (point.date, point.count))
        .collect();

    println!();
    println!(
        "{:<10} {:>8} {:>8} {:>7} {:>7}",
        "DATE", "REMAIN", "IDEAL", "+SCOPE", "-SCOPE"
    );

    for point in &report.series.actual {
        let ideal = ideal_by_date.get(&point.date).copied();
        let scope_added = scope_added_by_date.get(&point.date).copied().unwrap_or(0);
        let scope_removed = scope_removed_by_date.get(&point.date).copied().unwrap_or(0);

        println!(
            "{:<10} {:>8} {:>8} {:>7} {:>7}",
            point.date,
            point.remaining,
            format_hours_cell(ideal),
            format_count_cell(scope_added),
            format_count_cell(scope_removed),
        );
    }
}

fn print_burndown_minimal(report: &BurndownReport) {
    println!(
        "burndown bucket={} window={}..{} start={} end={} completed={} reopened={}",
        report.bucket,
        report.window.from,
        report.window.to,
        report.summary.start_remaining,
        report.summary.end_remaining,
        report.summary.completed_in_window,
        report.summary.reopened_in_window,
    );

    if !report.data_quality.is_empty() {
        println!(
            "quality missing_history_tasks={} inferred_samples={}",
            report.data_quality.missing_history_tasks, report.data_quality.inferred_samples,
        );
    }
}

fn render_completion_time(
    report: &CompletionTimeReport,
    filters: &MetricsFilters,
    format: Format,
) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string(report)?),
        Format::Pretty => print_completion_time_pretty(report, filters),
        Format::Minimal => print_completion_time_minimal(report),
    }
    Ok(())
}

fn print_completion_time_pretty(report: &CompletionTimeReport, filters: &MetricsFilters) {
    println!(
        "Completion-time metrics (metric={}, bucket={})",
        report.metric, report.bucket
    );
    println!("window: {} -> {}", report.window.from, report.window.to);
    println!("filters: {}", format_filters(filters));
    println!(
        "summary: avg={} p50={} p90={} samples={}",
        format_hours_summary(report.summary.avg_hours),
        format_hours_summary(report.summary.p50_hours),
        format_hours_summary(report.summary.p90_hours),
        report.summary.samples,
    );

    if !report.data_quality.is_empty() {
        println!(
            "data quality: missing_history_tasks={} inferred_samples={}",
            report.data_quality.missing_history_tasks, report.data_quality.inferred_samples,
        );
    }

    if report.series.is_empty() {
        println!("series: (no points)");
        return;
    }

    println!();
    println!(
        "{:<12} {:>9} {:>9} {:>9} {:>7}",
        "BUCKET", "AVG_H", "P50_H", "P90_H", "SAMPLES"
    );

    for point in &report.series {
        println!(
            "{:<12} {:>9} {:>9} {:>9} {:>7}",
            point.bucket,
            format_hours_cell(point.avg_hours),
            format_hours_cell(point.p50_hours),
            format_hours_cell(point.p90_hours),
            point.samples,
        );
    }
}

fn print_completion_time_minimal(report: &CompletionTimeReport) {
    println!(
        "completion-time metric={} bucket={} window={}..{} avg={} p50={} p90={} samples={}",
        report.metric,
        report.bucket,
        report.window.from,
        report.window.to,
        format_hours_summary(report.summary.avg_hours),
        format_hours_summary(report.summary.p50_hours),
        format_hours_summary(report.summary.p90_hours),
        report.summary.samples,
    );

    if let Some(last) = report.series.last() {
        println!(
            "latest bucket={} avg={} samples={}",
            last.bucket,
            format_hours_summary(last.avg_hours),
            last.samples,
        );
    }

    if !report.data_quality.is_empty() {
        println!(
            "quality missing_history_tasks={} inferred_samples={}",
            report.data_quality.missing_history_tasks, report.data_quality.inferred_samples,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn burndown(
    repo_root: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    bucket: MetricsBucket,
    kind: Option<Kind>,
    tags: Vec<String>,
    assignee: Option<String>,
    children_of: Option<u64>,
    include_cancelled: bool,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    let query = build_query(
        from,
        to,
        bucket,
        kind,
        tags,
        assignee,
        children_of,
        include_cancelled,
        MetricsMode::Burndown,
    )?;

    let tasks = filter_tasks(repo.store.list_all()?, &query.filters);

    let mut history_by_task = HashMap::new();
    for task in &tasks {
        history_by_task.insert(task.id, repo.sidecars.read_history(task.id)?);
    }

    let timelines = derive_timelines(&tasks, &history_by_task);
    let report = aggregate_burndown(&timelines, &query);

    render_burndown(&report, format)
}

#[allow(clippy::too_many_arguments)]
pub fn completion_time(
    repo_root: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    bucket: MetricsBucket,
    kind: Option<Kind>,
    tags: Vec<String>,
    assignee: Option<String>,
    children_of: Option<u64>,
    include_cancelled: bool,
    metric: CompletionMetric,
    format: Format,
) -> Result<()> {
    let repo = Repo::open(repo_root)?;

    let query = build_query(
        from,
        to,
        bucket,
        kind,
        tags,
        assignee,
        children_of,
        include_cancelled,
        MetricsMode::CompletionTime,
    )?;

    let tasks = filter_tasks(repo.store.list_all()?, &query.filters);

    let mut history_by_task = HashMap::new();
    for task in &tasks {
        history_by_task.insert(task.id, repo.sidecars.read_history(task.id)?);
    }

    let timelines = derive_timelines(&tasks, &history_by_task);
    let report = aggregate_completion_time(&query, metric, &timelines);

    render_completion_time(&report, &query.filters, format)
}

#[allow(clippy::too_many_arguments)]
pub fn tui(
    repo_root: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    bucket: MetricsBucket,
    metric: CompletionMetric,
    kind: Option<Kind>,
    tags: Vec<String>,
    assignee: Option<String>,
    children_of: Option<u64>,
    include_cancelled: bool,
    _format: Format,
) -> Result<()> {
    let query = build_query(
        from,
        to,
        bucket,
        kind,
        tags,
        assignee,
        children_of,
        include_cancelled,
        MetricsMode::Burndown,
    )?;

    let config = MetricsTuiConfig {
        query,
        metric,
        ..MetricsTuiConfig::default()
    };

    run_tui(repo_root, config)
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

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{TimeZone, Utc};

    use crate::model::{Contract, Execution, GitInfo, Planning};

    fn ts(year: i32, month: u32, day: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .unwrap()
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn task(
        id: u64,
        kind: Kind,
        status: Status,
        tags: &[&str],
        assignee: Option<&str>,
        parent: Option<u64>,
    ) -> Task {
        Task {
            id,
            title: format!("task-{id}"),
            description: None,
            status,
            kind,
            parent,
            depends_on: vec![],
            assignee: assignee.map(str::to_string),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            contract: Contract::default(),
            planning: Planning::default(),
            git: GitInfo::default(),
            execution: Execution::default(),
            learnings: vec![],
            created_at: ts(2026, 2, 1),
            updated_at: ts(2026, 2, 1),
            extensions: serde_json::Map::new(),
        }
    }

    #[test]
    fn build_query_rejects_inverted_date_window() {
        let err = build_query(
            Some(date("2026-02-10")),
            Some(date("2026-02-01")),
            MetricsBucket::Day,
            None,
            vec![],
            None,
            None,
            false,
            MetricsMode::Burndown,
        )
        .unwrap_err();

        assert!(matches!(err, TakError::MetricsInvalidQuery(_)));
        assert!(err.to_string().contains("inverted"));
    }

    #[test]
    fn build_query_rejects_blank_tag_filter() {
        let err = build_query(
            Some(date("2026-02-01")),
            Some(date("2026-02-02")),
            MetricsBucket::Day,
            None,
            vec!["   ".into()],
            None,
            None,
            false,
            MetricsMode::Burndown,
        )
        .unwrap_err();

        assert!(matches!(err, TakError::MetricsInvalidQuery(_)));
        assert!(err.to_string().contains("tag filter"));
    }

    #[test]
    fn build_query_rejects_blank_assignee_filter() {
        let err = build_query(
            Some(date("2026-02-01")),
            Some(date("2026-02-02")),
            MetricsBucket::Day,
            None,
            vec![],
            Some("   ".into()),
            None,
            false,
            MetricsMode::Burndown,
        )
        .unwrap_err();

        assert!(matches!(err, TakError::MetricsInvalidQuery(_)));
        assert!(err.to_string().contains("assignee"));
    }

    #[test]
    fn build_query_limits_day_bucket_span() {
        let err = build_query(
            Some(date("2024-01-01")),
            Some(date("2025-12-31")),
            MetricsBucket::Day,
            None,
            vec![],
            None,
            None,
            false,
            MetricsMode::Burndown,
        )
        .unwrap_err();

        assert!(matches!(err, TakError::MetricsInvalidQuery(_)));
        assert!(err.to_string().contains("day bucket window is too large"));
    }

    #[test]
    fn build_query_rejects_include_cancelled_for_completion_time() {
        let err = build_query(
            Some(date("2026-02-01")),
            Some(date("2026-02-03")),
            MetricsBucket::Day,
            None,
            vec![],
            None,
            None,
            true,
            MetricsMode::CompletionTime,
        )
        .unwrap_err();

        assert!(matches!(err, TakError::MetricsInvalidQuery(_)));
        assert!(err.to_string().contains("--include-cancelled"));
    }

    #[test]
    fn filter_tasks_applies_kind_tags_assignee_and_parent() {
        let tasks = vec![
            task(
                1,
                Kind::Task,
                Status::Done,
                &["metrics", "alpha"],
                Some("a"),
                Some(10),
            ),
            task(
                2,
                Kind::Feature,
                Status::Done,
                &["metrics"],
                Some("a"),
                Some(10),
            ),
            task(
                3,
                Kind::Task,
                Status::Done,
                &["metrics"],
                Some("b"),
                Some(10),
            ),
            task(4, Kind::Task, Status::Done, &["other"], Some("a"), Some(10)),
            task(
                5,
                Kind::Task,
                Status::Done,
                &["metrics", "alpha"],
                Some("a"),
                Some(11),
            ),
        ];

        let filtered = filter_tasks(
            tasks,
            &MetricsFilters {
                kind: Some(Kind::Task),
                tags: vec!["metrics".into(), "alpha".into()],
                assignee: Some("a".into()),
                children_of: Some(10),
                include_cancelled: true,
            },
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 1);
    }

    #[test]
    fn filter_tasks_excludes_cancelled_unless_requested() {
        let tasks = vec![
            task(1, Kind::Task, Status::Cancelled, &["metrics"], None, None),
            task(2, Kind::Task, Status::Done, &["metrics"], None, None),
        ];

        let default_filtered = filter_tasks(
            tasks.clone(),
            &MetricsFilters {
                include_cancelled: false,
                ..MetricsFilters::default()
            },
        );
        assert_eq!(
            default_filtered
                .iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            vec![2]
        );

        let include_filtered = filter_tasks(
            tasks,
            &MetricsFilters {
                include_cancelled: true,
                ..MetricsFilters::default()
            },
        );
        assert_eq!(include_filtered.len(), 2);
    }
}
