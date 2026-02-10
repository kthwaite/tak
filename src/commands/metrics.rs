use std::collections::HashMap;
use std::path::Path;

use chrono::{Duration, NaiveDate, Utc};

use crate::error::Result;
use crate::metrics::{
    CompletionMetric, MetricsBucket, MetricsFilters, MetricsQuery, MetricsWindow,
    aggregate_burndown, aggregate_completion_time, derive_timelines,
};
use crate::model::{Kind, Status, Task};
use crate::output::Format;
use crate::store::repo::Repo;

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

    let to = to.unwrap_or_else(|| Utc::now().date_naive());
    let from = from.unwrap_or_else(|| to - Duration::days(30));

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

    let tasks = filter_tasks(repo.store.list_all()?, &query.filters);

    let mut history_by_task = HashMap::new();
    for task in &tasks {
        history_by_task.insert(task.id, repo.sidecars.read_history(task.id)?);
    }

    let timelines = derive_timelines(&tasks, &history_by_task);
    let report = aggregate_burndown(&timelines, &query);

    match format {
        Format::Json | Format::Minimal => println!("{}", serde_json::to_string(&report)?),
        Format::Pretty => println!("{}", serde_json::to_string_pretty(&report)?),
    }

    Ok(())
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

    let to = to.unwrap_or_else(|| Utc::now().date_naive());
    let from = from.unwrap_or_else(|| to - Duration::days(30));

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

    let tasks = filter_tasks(repo.store.list_all()?, &query.filters);

    let mut history_by_task = HashMap::new();
    for task in &tasks {
        history_by_task.insert(task.id, repo.sidecars.read_history(task.id)?);
    }

    let timelines = derive_timelines(&tasks, &history_by_task);
    let report = aggregate_completion_time(&query, metric, &timelines);

    match format {
        Format::Json | Format::Minimal => println!("{}", serde_json::to_string(&report)?),
        Format::Pretty => println!("{}", serde_json::to_string_pretty(&report)?),
    }

    Ok(())
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
