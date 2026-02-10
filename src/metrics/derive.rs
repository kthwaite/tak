use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::model::{Status, Task};
use crate::store::sidecars::HistoryEvent;

/// Lifecycle event kinds relevant for metrics derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEventKind {
    Created,
    Claimed,
    Started,
    Finished,
    Cancelled,
    Reopened,
}

impl TimelineEventKind {
    fn from_history_event(value: &str) -> Option<Self> {
        match value {
            "claimed" => Some(Self::Claimed),
            "started" => Some(Self::Started),
            "finished" => Some(Self::Finished),
            "cancelled" => Some(Self::Cancelled),
            "reopened" => Some(Self::Reopened),
            _ => None,
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Self::Created => 0,
            Self::Claimed => 1,
            Self::Started => 2,
            Self::Finished => 3,
            Self::Cancelled => 4,
            Self::Reopened => 5,
        }
    }
}

/// A normalized lifecycle point on a task timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelinePoint {
    pub timestamp: DateTime<Utc>,
    pub kind: TimelineEventKind,
    pub inferred: bool,
}

/// A single completion episode for cycle-time style analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEpisode {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub inferred_start: bool,
    pub inferred_finish: bool,
}

/// Deterministically normalized lifecycle output for one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedTimeline {
    pub task_id: u64,
    pub created_at: DateTime<Utc>,
    pub events: Vec<TimelinePoint>,
    pub completion_episodes: Vec<CompletionEpisode>,
    pub missing_history: bool,
    pub inferred_samples: usize,
}

#[derive(Debug, Clone)]
struct RankedPoint {
    point: TimelinePoint,
    source_index: usize,
}

/// Derive a normalized lifecycle timeline for a single task.
///
/// Rules:
/// - Start from task `created_at`.
/// - Include lifecycle events from sidecar history (`claimed`, `started`, `finished`,
///   `cancelled`, `reopened`).
/// - Sort deterministically by timestamp, lifecycle precedence, inferred flag, then source order.
/// - Apply missing-event fallback for terminal/active states when needed.
/// - Derive completion episodes for lead/cycle metrics consumers.
pub fn derive_timeline(task: &Task, history: &[HistoryEvent]) -> NormalizedTimeline {
    let mut points = Vec::with_capacity(history.len() + 2);
    points.push(RankedPoint {
        point: TimelinePoint {
            timestamp: task.created_at,
            kind: TimelineEventKind::Created,
            inferred: false,
        },
        source_index: 0,
    });

    for (index, entry) in history.iter().enumerate() {
        if let Some(kind) = TimelineEventKind::from_history_event(&entry.event) {
            points.push(RankedPoint {
                point: TimelinePoint {
                    timestamp: entry.timestamp,
                    kind,
                    inferred: false,
                },
                source_index: index + 1,
            });
        }
    }

    let mut inferred_samples = 0;
    let mut next_source_index = history.len() + 1;
    apply_status_fallback(
        task,
        &mut points,
        &mut inferred_samples,
        &mut next_source_index,
    );

    sort_points(&mut points);

    let ordered_points: Vec<TimelinePoint> =
        points.into_iter().map(|ranked| ranked.point).collect();
    let completion_episodes =
        derive_completion_episodes(task.created_at, &ordered_points, &mut inferred_samples);

    NormalizedTimeline {
        task_id: task.id,
        created_at: task.created_at,
        events: ordered_points,
        completion_episodes,
        missing_history: history.is_empty(),
        inferred_samples,
    }
}

/// Derive normalized timelines for many tasks, returning deterministic task-id ordering.
pub fn derive_timelines(
    tasks: &[Task],
    history_by_task: &HashMap<u64, Vec<HistoryEvent>>,
) -> Vec<NormalizedTimeline> {
    let mut ordered: Vec<&Task> = tasks.iter().collect();
    ordered.sort_by_key(|task| task.id);

    ordered
        .into_iter()
        .map(|task| {
            let history = history_by_task
                .get(&task.id)
                .map_or(&[][..], |events| events.as_slice());
            derive_timeline(task, history)
        })
        .collect()
}

fn has_kind(points: &[RankedPoint], kind: TimelineEventKind) -> bool {
    points.iter().any(|point| point.point.kind == kind)
}

fn has_in_progress_signal(points: &[RankedPoint]) -> bool {
    points.iter().any(|point| {
        matches!(
            point.point.kind,
            TimelineEventKind::Claimed | TimelineEventKind::Started | TimelineEventKind::Reopened
        )
    })
}

fn push_inferred_point(
    points: &mut Vec<RankedPoint>,
    inferred_samples: &mut usize,
    next_source_index: &mut usize,
    timestamp: DateTime<Utc>,
    kind: TimelineEventKind,
) {
    points.push(RankedPoint {
        point: TimelinePoint {
            timestamp,
            kind,
            inferred: true,
        },
        source_index: *next_source_index,
    });
    *next_source_index += 1;
    *inferred_samples += 1;
}

fn apply_status_fallback(
    task: &Task,
    points: &mut Vec<RankedPoint>,
    inferred_samples: &mut usize,
    next_source_index: &mut usize,
) {
    match task.status {
        Status::Done if !has_kind(points, TimelineEventKind::Finished) => {
            push_inferred_point(
                points,
                inferred_samples,
                next_source_index,
                task.updated_at,
                TimelineEventKind::Finished,
            );
        }
        Status::Cancelled if !has_kind(points, TimelineEventKind::Cancelled) => {
            push_inferred_point(
                points,
                inferred_samples,
                next_source_index,
                task.updated_at,
                TimelineEventKind::Cancelled,
            );
        }
        Status::InProgress if !has_in_progress_signal(points) => {
            push_inferred_point(
                points,
                inferred_samples,
                next_source_index,
                task.updated_at,
                TimelineEventKind::Started,
            );
        }
        _ => {}
    }
}

fn sort_points(points: &mut [RankedPoint]) {
    points.sort_by(|left, right| {
        left.point
            .timestamp
            .cmp(&right.point.timestamp)
            .then_with(|| {
                left.point
                    .kind
                    .sort_rank()
                    .cmp(&right.point.kind.sort_rank())
            })
            .then_with(|| left.point.inferred.cmp(&right.point.inferred))
            .then_with(|| left.source_index.cmp(&right.source_index))
    });
}

fn derive_completion_episodes(
    created_at: DateTime<Utc>,
    points: &[TimelinePoint],
    inferred_samples: &mut usize,
) -> Vec<CompletionEpisode> {
    let mut episodes = Vec::new();
    let mut active_start: Option<(DateTime<Utc>, bool)> = None;

    for point in points {
        match point.kind {
            TimelineEventKind::Created => {}
            TimelineEventKind::Claimed | TimelineEventKind::Started => {
                if active_start.is_none() {
                    active_start = Some((point.timestamp, point.inferred));
                }
            }
            TimelineEventKind::Reopened => {
                active_start = Some((point.timestamp, point.inferred));
            }
            TimelineEventKind::Finished => {
                let (started_at, inferred_start) = if let Some(start) = active_start.take() {
                    start
                } else {
                    *inferred_samples += 1;
                    (created_at, true)
                };
                episodes.push(CompletionEpisode {
                    started_at,
                    finished_at: point.timestamp,
                    inferred_start,
                    inferred_finish: point.inferred,
                });
            }
            TimelineEventKind::Cancelled => {
                active_start = None;
            }
        }
    }

    episodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    use crate::model::{Contract, Execution, GitInfo, Kind, Planning};
    use crate::store::coordination::CoordinationLinks;

    fn ts(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .single()
            .unwrap()
    }

    fn task(id: u64, status: Status, created_at: DateTime<Utc>, updated_at: DateTime<Utc>) -> Task {
        Task {
            id,
            title: format!("task-{id}"),
            description: None,
            status,
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
            created_at,
            updated_at,
            extensions: serde_json::Map::new(),
        }
    }

    fn event(kind: &str, timestamp: DateTime<Utc>) -> HistoryEvent {
        HistoryEvent {
            id: None,
            timestamp,
            event: kind.to_string(),
            agent: None,
            detail: serde_json::Map::new(),
            links: CoordinationLinks::default(),
        }
    }

    fn kinds(timeline: &NormalizedTimeline) -> Vec<TimelineEventKind> {
        timeline.events.iter().map(|point| point.kind).collect()
    }

    #[test]
    fn orders_same_timestamp_events_deterministically() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let shared = ts(2026, 2, 2, 9, 0, 0);
        let finished = ts(2026, 2, 2, 12, 0, 0);
        let timeline = derive_timeline(
            &task(1, Status::Pending, created, finished),
            &[
                event("finished", finished),
                event("started", shared),
                event("claimed", shared),
                event("reopened", finished),
            ],
        );

        assert_eq!(
            kinds(&timeline),
            vec![
                TimelineEventKind::Created,
                TimelineEventKind::Claimed,
                TimelineEventKind::Started,
                TimelineEventKind::Finished,
                TimelineEventKind::Reopened,
            ]
        );
    }

    #[test]
    fn infers_terminal_done_event_when_history_is_missing() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let updated = ts(2026, 2, 3, 12, 0, 0);
        let timeline = derive_timeline(&task(7, Status::Done, created, updated), &[]);

        assert!(timeline.missing_history);
        assert_eq!(
            kinds(&timeline),
            vec![TimelineEventKind::Created, TimelineEventKind::Finished]
        );
        assert!(timeline.events[1].inferred);
        assert_eq!(timeline.events[1].timestamp, updated);

        assert_eq!(timeline.completion_episodes.len(), 1);
        let episode = &timeline.completion_episodes[0];
        assert_eq!(episode.started_at, created);
        assert_eq!(episode.finished_at, updated);
        assert!(episode.inferred_start);
        assert!(episode.inferred_finish);

        // one inferred terminal event + one inferred cycle start fallback
        assert_eq!(timeline.inferred_samples, 2);
    }

    #[test]
    fn uses_claimed_as_cycle_start_when_started_is_absent() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let claimed = ts(2026, 2, 1, 8, 0, 0);
        let finished = ts(2026, 2, 1, 10, 0, 0);

        let timeline = derive_timeline(
            &task(9, Status::Done, created, finished),
            &[event("claimed", claimed), event("finished", finished)],
        );

        assert_eq!(timeline.completion_episodes.len(), 1);
        let episode = &timeline.completion_episodes[0];
        assert_eq!(episode.started_at, claimed);
        assert_eq!(episode.finished_at, finished);
        assert!(!episode.inferred_start);
        assert!(!episode.inferred_finish);
        assert_eq!(timeline.inferred_samples, 0);
    }

    #[test]
    fn reopened_event_starts_new_completion_episode() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let start1 = ts(2026, 2, 1, 8, 0, 0);
        let finish1 = ts(2026, 2, 1, 9, 0, 0);
        let reopened = ts(2026, 2, 1, 10, 0, 0);
        let finish2 = ts(2026, 2, 1, 12, 0, 0);

        let timeline = derive_timeline(
            &task(11, Status::Done, created, finish2),
            &[
                event("started", start1),
                event("finished", finish1),
                event("reopened", reopened),
                event("finished", finish2),
            ],
        );

        assert_eq!(timeline.completion_episodes.len(), 2);
        assert_eq!(timeline.completion_episodes[0].started_at, start1);
        assert_eq!(timeline.completion_episodes[0].finished_at, finish1);
        assert_eq!(timeline.completion_episodes[1].started_at, reopened);
        assert_eq!(timeline.completion_episodes[1].finished_at, finish2);
        assert!(!timeline.completion_episodes[1].inferred_start);
        assert_eq!(timeline.inferred_samples, 0);
    }

    #[test]
    fn in_progress_without_lifecycle_history_gets_inferred_started() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let updated = ts(2026, 2, 1, 18, 0, 0);

        let timeline = derive_timeline(
            &task(13, Status::InProgress, created, updated),
            &[event("handoff", ts(2026, 2, 1, 2, 0, 0))],
        );

        assert!(!timeline.missing_history);
        assert_eq!(
            kinds(&timeline),
            vec![TimelineEventKind::Created, TimelineEventKind::Started]
        );
        assert!(timeline.events[1].inferred);
        assert_eq!(timeline.events[1].timestamp, updated);
        assert_eq!(timeline.inferred_samples, 1);
    }

    #[test]
    fn derive_timelines_returns_task_id_order() {
        let created = ts(2026, 2, 1, 0, 0, 0);
        let updated = ts(2026, 2, 1, 1, 0, 0);

        let tasks = vec![
            task(2, Status::Done, created, updated),
            task(1, Status::Done, created, updated),
        ];

        let mut history_by_task = HashMap::new();
        history_by_task.insert(2, vec![event("finished", updated)]);
        history_by_task.insert(1, vec![event("finished", updated)]);

        let timelines = derive_timelines(&tasks, &history_by_task);

        assert_eq!(
            timelines
                .iter()
                .map(|timeline| timeline.task_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
}
