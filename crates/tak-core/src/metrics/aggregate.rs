use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

use crate::metrics::derive::NormalizedTimeline;
use crate::metrics::model::{
    CompletionMetric, CompletionTimePoint, CompletionTimeReport, CompletionTimeSummary,
    DataQuality, MetricsBucket, MetricsQuery,
};

#[derive(Debug, Clone)]
struct DurationSample {
    finished_at: DateTime<Utc>,
    hours: f64,
}

/// Aggregate completion-time trend samples by requested time bucket.
///
/// Output includes per-bucket avg/p50/p90 and overall summary stats, with
/// data-quality rollups from timeline derivation.
pub fn aggregate_completion_time(
    query: &MetricsQuery,
    metric: CompletionMetric,
    timelines: &[NormalizedTimeline],
) -> CompletionTimeReport {
    let mut data_quality = DataQuality::default();
    for timeline in timelines {
        if timeline.missing_history {
            data_quality.missing_history_tasks += 1;
        }
        data_quality.inferred_samples += timeline.inferred_samples;
    }

    let (samples, dropped_samples) = collect_completion_samples(query, metric, timelines);
    data_quality.inferred_samples += dropped_samples;

    let mut bucketed: BTreeMap<NaiveDate, Vec<f64>> = BTreeMap::new();
    for sample in &samples {
        let bucket = bucket_start(sample.finished_at.date_naive(), query.bucket);
        bucketed.entry(bucket).or_default().push(sample.hours);
    }

    let mut series = Vec::with_capacity(bucketed.len());
    for (bucket_date, durations) in bucketed {
        let stats = summarize_values(&durations);
        series.push(CompletionTimePoint {
            bucket: bucket_label(bucket_date, query.bucket),
            avg_hours: stats.avg_hours,
            p50_hours: stats.p50_hours,
            p90_hours: stats.p90_hours,
            samples: stats.samples,
        });
    }

    let summary = summarize_values(
        &samples
            .iter()
            .map(|sample| sample.hours)
            .collect::<Vec<_>>(),
    );

    CompletionTimeReport {
        window: query.window,
        bucket: query.bucket,
        metric,
        series,
        summary,
        data_quality,
    }
}

fn collect_completion_samples(
    query: &MetricsQuery,
    metric: CompletionMetric,
    timelines: &[NormalizedTimeline],
) -> (Vec<DurationSample>, usize) {
    let mut samples = Vec::new();
    let mut dropped_samples = 0;

    for timeline in timelines {
        for episode in &timeline.completion_episodes {
            let finished_date = episode.finished_at.date_naive();
            if finished_date < query.window.from || finished_date > query.window.to {
                continue;
            }

            let started_at = match metric {
                CompletionMetric::Lead => timeline.created_at,
                CompletionMetric::Cycle => episode.started_at,
            };
            let duration = episode.finished_at.signed_duration_since(started_at);
            if duration < Duration::zero() {
                dropped_samples += 1;
                continue;
            }

            let hours = duration.num_seconds() as f64 / 3600.0;
            samples.push(DurationSample {
                finished_at: episode.finished_at,
                hours,
            });
        }
    }

    (samples, dropped_samples)
}

fn bucket_start(date: NaiveDate, bucket: MetricsBucket) -> NaiveDate {
    match bucket {
        MetricsBucket::Day => date,
        MetricsBucket::Week => {
            date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
        }
    }
}

fn bucket_label(date: NaiveDate, bucket: MetricsBucket) -> String {
    match bucket {
        MetricsBucket::Day => date.format("%Y-%m-%d").to_string(),
        MetricsBucket::Week => {
            let iso = date.iso_week();
            format!("{:04}-W{:02}", iso.year(), iso.week())
        }
    }
}

fn summarize_values(values: &[f64]) -> CompletionTimeSummary {
    if values.is_empty() {
        return CompletionTimeSummary::default();
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    let samples = sorted.len();
    let avg_hours = Some(sorted.iter().sum::<f64>() / samples as f64);
    let p50_hours = percentile(&sorted, 0.50);
    let p90_hours = percentile(&sorted, 0.90);

    CompletionTimeSummary {
        avg_hours,
        p50_hours,
        p90_hours,
        samples,
    }
}

fn percentile(sorted_values: &[f64], quantile: f64) -> Option<f64> {
    if sorted_values.is_empty() {
        return None;
    }

    let rank = ((quantile * sorted_values.len() as f64).ceil() as usize).saturating_sub(1);
    sorted_values.get(rank).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::metrics::derive::{CompletionEpisode, NormalizedTimeline};
    use crate::metrics::model::{MetricsFilters, MetricsWindow};

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn dt(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn query(from: &str, to: &str, bucket: MetricsBucket) -> MetricsQuery {
        MetricsQuery {
            window: MetricsWindow {
                from: date(from),
                to: date(to),
            },
            bucket,
            filters: MetricsFilters::default(),
        }
    }

    fn timeline(
        task_id: u64,
        created_at: &str,
        episodes: &[(&str, &str)],
        missing_history: bool,
        inferred_samples: usize,
    ) -> NormalizedTimeline {
        NormalizedTimeline {
            task_id,
            created_at: dt(created_at),
            events: vec![],
            completion_episodes: episodes
                .iter()
                .map(|(start, finish)| CompletionEpisode {
                    started_at: dt(start),
                    finished_at: dt(finish),
                    inferred_start: false,
                    inferred_finish: false,
                })
                .collect(),
            missing_history,
            inferred_samples,
        }
    }

    fn approx_eq(left: f64, right: f64) {
        assert!((left - right).abs() < 1e-6, "left={left}, right={right}");
    }

    #[test]
    fn aggregates_cycle_time_in_daily_buckets() {
        let report = aggregate_completion_time(
            &query("2026-02-01", "2026-02-01", MetricsBucket::Day),
            CompletionMetric::Cycle,
            &[
                timeline(
                    1,
                    "2026-01-30T00:00:00Z",
                    &[("2026-02-01T00:00:00Z", "2026-02-01T12:00:00Z")],
                    false,
                    0,
                ),
                timeline(
                    2,
                    "2026-01-30T00:00:00Z",
                    &[("2026-01-31T20:00:00Z", "2026-02-01T20:00:00Z")],
                    false,
                    0,
                ),
            ],
        );

        assert_eq!(report.series.len(), 1);
        let point = &report.series[0];
        assert_eq!(point.bucket, "2026-02-01");
        assert_eq!(point.samples, 2);
        approx_eq(point.avg_hours.unwrap(), 18.0);
        approx_eq(point.p50_hours.unwrap(), 12.0);
        approx_eq(point.p90_hours.unwrap(), 24.0);

        assert_eq!(report.summary.samples, 2);
        approx_eq(report.summary.avg_hours.unwrap(), 18.0);
    }

    #[test]
    fn lead_time_uses_task_creation_timestamp() {
        let report = aggregate_completion_time(
            &query("2026-02-03", "2026-02-03", MetricsBucket::Day),
            CompletionMetric::Lead,
            &[timeline(
                9,
                "2026-02-01T00:00:00Z",
                &[("2026-02-03T00:00:00Z", "2026-02-03T12:00:00Z")],
                false,
                0,
            )],
        );

        assert_eq!(report.summary.samples, 1);
        // 2 days + 12h
        approx_eq(report.summary.avg_hours.unwrap(), 60.0);
    }

    #[test]
    fn excludes_samples_outside_requested_window() {
        let report = aggregate_completion_time(
            &query("2026-02-02", "2026-02-02", MetricsBucket::Day),
            CompletionMetric::Cycle,
            &[timeline(
                3,
                "2026-02-01T00:00:00Z",
                &[
                    ("2026-02-01T00:00:00Z", "2026-02-01T12:00:00Z"),
                    ("2026-02-02T00:00:00Z", "2026-02-02T12:00:00Z"),
                ],
                false,
                0,
            )],
        );

        assert_eq!(report.series.len(), 1);
        assert_eq!(report.series[0].bucket, "2026-02-02");
        assert_eq!(report.series[0].samples, 1);
    }

    #[test]
    fn rolls_up_data_quality_counts() {
        let report = aggregate_completion_time(
            &query("2026-02-01", "2026-02-02", MetricsBucket::Day),
            CompletionMetric::Cycle,
            &[
                timeline(
                    1,
                    "2026-02-01T00:00:00Z",
                    &[("2026-02-01T00:00:00Z", "2026-02-01T01:00:00Z")],
                    true,
                    2,
                ),
                timeline(
                    2,
                    "2026-02-01T00:00:00Z",
                    &[("2026-02-02T00:00:00Z", "2026-02-02T01:00:00Z")],
                    false,
                    1,
                ),
            ],
        );

        assert_eq!(report.data_quality.missing_history_tasks, 1);
        assert_eq!(report.data_quality.inferred_samples, 3);
    }

    #[test]
    fn week_bucket_uses_iso_week_label() {
        let report = aggregate_completion_time(
            &query("2026-02-02", "2026-02-08", MetricsBucket::Week),
            CompletionMetric::Cycle,
            &[timeline(
                5,
                "2026-02-01T00:00:00Z",
                &[
                    ("2026-02-02T00:00:00Z", "2026-02-02T12:00:00Z"),
                    ("2026-02-04T00:00:00Z", "2026-02-04T12:00:00Z"),
                ],
                false,
                0,
            )],
        );

        assert_eq!(report.series.len(), 1);

        let iso = date("2026-02-04").iso_week();
        let expected = format!("{:04}-W{:02}", iso.year(), iso.week());
        assert_eq!(report.series[0].bucket, expected);
        assert_eq!(report.series[0].samples, 2);
    }

    #[test]
    fn drops_negative_duration_samples_and_tracks_quality_count() {
        let report = aggregate_completion_time(
            &query("2026-02-01", "2026-02-01", MetricsBucket::Day),
            CompletionMetric::Cycle,
            &[timeline(
                6,
                "2026-02-01T00:00:00Z",
                &[
                    ("2026-02-01T10:00:00Z", "2026-02-01T09:00:00Z"),
                    ("2026-02-01T00:00:00Z", "2026-02-01T02:00:00Z"),
                ],
                false,
                0,
            )],
        );

        assert_eq!(report.summary.samples, 1);
        approx_eq(report.summary.avg_hours.unwrap(), 2.0);
        assert_eq!(report.data_quality.inferred_samples, 1);
    }

    #[test]
    fn empty_window_summary_has_no_stats() {
        let report = aggregate_completion_time(
            &query("2026-02-05", "2026-02-05", MetricsBucket::Day),
            CompletionMetric::Cycle,
            &[timeline(
                7,
                "2026-02-01T00:00:00Z",
                &[("2026-02-01T00:00:00Z", "2026-02-01T02:00:00Z")],
                false,
                0,
            )],
        );

        assert!(report.series.is_empty());
        assert_eq!(report.summary.samples, 0);
        assert!(report.summary.avg_hours.is_none());
        assert!(report.summary.p50_hours.is_none());
        assert!(report.summary.p90_hours.is_none());
    }
}
