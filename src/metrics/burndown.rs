use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};

use crate::metrics::derive::{NormalizedTimeline, TimelineEventKind};
use crate::metrics::model::{
    BurndownPoint, BurndownReport, BurndownSeries, BurndownSummary, CountPoint, DataQuality,
    IdealBurndownPoint, MetricsBucket, MetricsQuery,
};

/// Aggregate normalized lifecycle timelines into a burndown report.
pub fn aggregate_burndown(
    timelines: &[NormalizedTimeline],
    query: &MetricsQuery,
) -> BurndownReport {
    let mut normalized = query.clone();
    normalized.normalize();

    let window = normalized.window;
    let bucket = normalized.bucket;
    let filters = normalized.filters;

    let bucket_dates = bucket_dates(window.from, window.to, bucket);

    let mut delta_by_bucket = BTreeMap::<NaiveDate, i64>::new();
    let mut scope_added_by_bucket = BTreeMap::<NaiveDate, usize>::new();
    let mut scope_removed_by_bucket = BTreeMap::<NaiveDate, usize>::new();

    let mut initial_remaining: i64 = 0;
    let mut completed_in_window = 0usize;
    let mut reopened_in_window = 0usize;
    let mut data_quality = DataQuality::default();

    for timeline in timelines {
        if !filters.include_cancelled && final_status_is_cancelled(timeline) {
            continue;
        }

        data_quality.missing_history_tasks += usize::from(timeline.missing_history);
        data_quality.inferred_samples += timeline.inferred_samples;

        for event in &timeline.events {
            let event_date = event.timestamp.date_naive();
            let bucket_date = bucket_for(event_date, window.from, window.to, bucket);

            match event.kind {
                TimelineEventKind::Created => {
                    if event_date < window.from {
                        initial_remaining += 1;
                    } else if let Some(date) = bucket_date {
                        *delta_by_bucket.entry(date).or_insert(0) += 1;
                        *scope_added_by_bucket.entry(date).or_insert(0) += 1;
                    }
                }
                TimelineEventKind::Finished => {
                    if event_date < window.from {
                        initial_remaining -= 1;
                    } else if let Some(date) = bucket_date {
                        *delta_by_bucket.entry(date).or_insert(0) -= 1;
                        completed_in_window += 1;
                    }
                }
                TimelineEventKind::Cancelled => {
                    if event_date < window.from {
                        initial_remaining -= 1;
                    } else if let Some(date) = bucket_date {
                        *delta_by_bucket.entry(date).or_insert(0) -= 1;
                        *scope_removed_by_bucket.entry(date).or_insert(0) += 1;
                    }
                }
                TimelineEventKind::Reopened => {
                    if event_date < window.from {
                        initial_remaining += 1;
                    } else if let Some(date) = bucket_date {
                        *delta_by_bucket.entry(date).or_insert(0) += 1;
                        reopened_in_window += 1;
                    }
                }
                TimelineEventKind::Claimed | TimelineEventKind::Started => {}
            }
        }
    }

    let mut remaining = initial_remaining.max(0);
    let start_remaining = remaining as usize;

    let mut actual = Vec::with_capacity(bucket_dates.len());
    let mut ideal = Vec::with_capacity(bucket_dates.len());

    let denominator = bucket_dates.len().max(1) as f64;

    for (index, date) in bucket_dates.iter().copied().enumerate() {
        remaining = (remaining + delta_by_bucket.get(&date).copied().unwrap_or(0)).max(0);
        actual.push(BurndownPoint { date, remaining });

        let progress = (index + 1) as f64 / denominator;
        let ideal_remaining = (start_remaining as f64 * (1.0 - progress)).max(0.0);
        ideal.push(IdealBurndownPoint {
            date,
            remaining: ideal_remaining,
        });
    }

    let scope_added = count_points(&bucket_dates, &scope_added_by_bucket);
    let scope_removed = count_points(&bucket_dates, &scope_removed_by_bucket);

    BurndownReport {
        window,
        bucket,
        filters,
        series: BurndownSeries {
            actual,
            ideal,
            scope_added,
            scope_removed,
        },
        summary: BurndownSummary {
            start_remaining,
            end_remaining: remaining as usize,
            completed_in_window,
            reopened_in_window,
        },
        data_quality,
    }
}

fn final_status_is_cancelled(timeline: &NormalizedTimeline) -> bool {
    let mut cancelled = false;

    for point in &timeline.events {
        match point.kind {
            TimelineEventKind::Cancelled => cancelled = true,
            TimelineEventKind::Finished | TimelineEventKind::Reopened => cancelled = false,
            TimelineEventKind::Created
            | TimelineEventKind::Claimed
            | TimelineEventKind::Started => {}
        }
    }

    cancelled
}

fn bucket_for(
    date: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
    bucket: MetricsBucket,
) -> Option<NaiveDate> {
    if date < from || date > to {
        return None;
    }

    let bucket_start = match bucket {
        MetricsBucket::Day => date,
        MetricsBucket::Week => {
            let elapsed_days = (date - from).num_days();
            from + Duration::days((elapsed_days / 7) * 7)
        }
    };

    Some(bucket_start)
}

fn bucket_dates(from: NaiveDate, to: NaiveDate, bucket: MetricsBucket) -> Vec<NaiveDate> {
    let step_days = match bucket {
        MetricsBucket::Day => 1,
        MetricsBucket::Week => 7,
    };

    let mut dates = Vec::new();
    let mut cursor = from;

    while cursor <= to {
        dates.push(cursor);
        cursor += Duration::days(step_days);
    }

    dates
}

fn count_points(order: &[NaiveDate], counts: &BTreeMap<NaiveDate, usize>) -> Vec<CountPoint> {
    order
        .iter()
        .filter_map(|date| {
            counts
                .get(date)
                .copied()
                .filter(|count| *count > 0)
                .map(|count| CountPoint { date: *date, count })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::*;
    use crate::metrics::derive::{CompletionEpisode, TimelinePoint};
    use crate::metrics::model::{MetricsFilters, MetricsWindow};

    fn ts(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .single()
            .unwrap()
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn timeline(
        task_id: u64,
        created_at: DateTime<Utc>,
        events: Vec<(TimelineEventKind, DateTime<Utc>)>,
        missing_history: bool,
        inferred_samples: usize,
    ) -> NormalizedTimeline {
        let points = std::iter::once(TimelinePoint {
            timestamp: created_at,
            kind: TimelineEventKind::Created,
            inferred: false,
        })
        .chain(events.into_iter().map(|(kind, timestamp)| TimelinePoint {
            timestamp,
            kind,
            inferred: false,
        }))
        .collect();

        NormalizedTimeline {
            task_id,
            created_at,
            events: points,
            completion_episodes: Vec::<CompletionEpisode>::new(),
            missing_history,
            inferred_samples,
        }
    }

    #[test]
    fn aggregates_daily_burndown_with_scope_and_completion_counts() {
        let from = date("2026-02-01");
        let to = date("2026-02-04");

        let t1 = timeline(
            1,
            ts(2026, 2, 1, 8, 0, 0),
            vec![(TimelineEventKind::Finished, ts(2026, 2, 3, 17, 0, 0))],
            false,
            0,
        );

        let t2 = timeline(
            2,
            ts(2026, 2, 2, 9, 0, 0),
            vec![(TimelineEventKind::Finished, ts(2026, 2, 4, 12, 0, 0))],
            false,
            0,
        );

        let report = aggregate_burndown(
            &[t1, t2],
            &MetricsQuery {
                window: MetricsWindow { from, to },
                bucket: MetricsBucket::Day,
                filters: MetricsFilters::default(),
            },
        );

        assert_eq!(
            report
                .series
                .actual
                .iter()
                .map(|point| point.remaining)
                .collect::<Vec<_>>(),
            vec![1, 2, 1, 0]
        );
        assert_eq!(report.summary.start_remaining, 0);
        assert_eq!(report.summary.end_remaining, 0);
        assert_eq!(report.summary.completed_in_window, 2);
        assert_eq!(report.summary.reopened_in_window, 0);

        assert_eq!(
            report
                .series
                .scope_added
                .iter()
                .map(|point| (point.date, point.count))
                .collect::<Vec<_>>(),
            vec![(date("2026-02-01"), 1), (date("2026-02-02"), 1)]
        );
    }

    #[test]
    fn excludes_finally_cancelled_tasks_by_default() {
        let from = date("2026-02-01");
        let to = date("2026-02-02");

        let cancelled = timeline(
            10,
            ts(2026, 2, 1, 8, 0, 0),
            vec![(TimelineEventKind::Cancelled, ts(2026, 2, 2, 10, 0, 0))],
            true,
            1,
        );

        let done = timeline(
            11,
            ts(2026, 2, 1, 9, 0, 0),
            vec![(TimelineEventKind::Finished, ts(2026, 2, 2, 15, 0, 0))],
            false,
            0,
        );

        let report_default = aggregate_burndown(
            &[cancelled.clone(), done.clone()],
            &MetricsQuery {
                window: MetricsWindow { from, to },
                bucket: MetricsBucket::Day,
                filters: MetricsFilters::default(),
            },
        );
        assert_eq!(report_default.summary.start_remaining, 0);
        assert_eq!(report_default.summary.completed_in_window, 1);
        assert_eq!(report_default.data_quality.missing_history_tasks, 0);

        let mut filters = MetricsFilters {
            include_cancelled: true,
            ..MetricsFilters::default()
        };
        filters.normalize();

        let report_including_cancelled = aggregate_burndown(
            &[cancelled, done],
            &MetricsQuery {
                window: MetricsWindow { from, to },
                bucket: MetricsBucket::Day,
                filters,
            },
        );
        assert_eq!(report_including_cancelled.summary.completed_in_window, 1);
        assert_eq!(
            report_including_cancelled
                .data_quality
                .missing_history_tasks,
            1
        );
        assert_eq!(
            report_including_cancelled
                .series
                .scope_removed
                .iter()
                .map(|point| point.count)
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn buckets_weekly_relative_to_window_start() {
        let from = date("2026-02-01");
        let to = date("2026-02-14");

        let timeline = timeline(
            20,
            ts(2026, 2, 2, 8, 0, 0),
            vec![(TimelineEventKind::Finished, ts(2026, 2, 10, 18, 0, 0))],
            false,
            0,
        );

        let report = aggregate_burndown(
            &[timeline],
            &MetricsQuery {
                window: MetricsWindow { from, to },
                bucket: MetricsBucket::Week,
                filters: MetricsFilters::default(),
            },
        );

        assert_eq!(
            report
                .series
                .actual
                .iter()
                .map(|point| (point.date, point.remaining))
                .collect::<Vec<_>>(),
            vec![(date("2026-02-01"), 1), (date("2026-02-08"), 0)]
        );
        assert_eq!(report.summary.completed_in_window, 1);
    }
}
