use chrono::NaiveDate;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::model::Kind;

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum MetricsBucket {
    #[default]
    Day,
    Week,
}

impl std::fmt::Display for MetricsBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Day => write!(f, "day"),
            Self::Week => write!(f, "week"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsWindow {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl MetricsWindow {
    pub fn normalize(&mut self) {
        if self.to < self.from {
            std::mem::swap(&mut self.from, &mut self.to);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetricsFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children_of: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_cancelled: bool,
}

impl MetricsFilters {
    pub fn normalize(&mut self) {
        for tag in &mut self.tags {
            *tag = tag.trim().to_string();
        }
        self.tags.retain(|tag| !tag.is_empty());
        self.tags.sort();
        self.tags.dedup();

        self.assignee = self
            .assignee
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsQuery {
    pub window: MetricsWindow,
    #[serde(default)]
    pub bucket: MetricsBucket,
    #[serde(default)]
    pub filters: MetricsFilters,
}

impl MetricsQuery {
    pub fn normalize(&mut self) {
        self.window.normalize();
        self.filters.normalize();
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum CompletionMetric {
    Lead,
    #[default]
    Cycle,
}

impl std::fmt::Display for CompletionMetric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lead => write!(f, "lead"),
            Self::Cycle => write!(f, "cycle"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum CompletionStat {
    #[default]
    Avg,
    P50,
    P90,
}

impl std::fmt::Display for CompletionStat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Avg => write!(f, "avg"),
            Self::P50 => write!(f, "p50"),
            Self::P90 => write!(f, "p90"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataQuality {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub missing_history_tasks: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub inferred_samples: usize,
}

impl DataQuality {
    pub fn is_empty(&self) -> bool {
        self.missing_history_tasks == 0 && self.inferred_samples == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurndownPoint {
    pub date: NaiveDate,
    pub remaining: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdealBurndownPoint {
    pub date: NaiveDate,
    pub remaining: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountPoint {
    pub date: NaiveDate,
    pub count: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurndownSeries {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actual: Vec<BurndownPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ideal: Vec<IdealBurndownPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_added: Vec<CountPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_removed: Vec<CountPoint>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurndownSummary {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start_remaining: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub end_remaining: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub completed_in_window: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reopened_in_window: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurndownReport {
    pub window: MetricsWindow,
    #[serde(default)]
    pub bucket: MetricsBucket,
    #[serde(default)]
    pub filters: MetricsFilters,
    pub series: BurndownSeries,
    pub summary: BurndownSummary,
    #[serde(default, skip_serializing_if = "DataQuality::is_empty")]
    pub data_quality: DataQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionTimePoint {
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub samples: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionTimeSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub samples: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionTimeReport {
    pub window: MetricsWindow,
    #[serde(default)]
    pub bucket: MetricsBucket,
    pub metric: CompletionMetric,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<CompletionTimePoint>,
    pub summary: CompletionTimeSummary,
    #[serde(default, skip_serializing_if = "DataQuality::is_empty")]
    pub data_quality: DataQuality,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn metrics_bucket_and_metric_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&MetricsBucket::Week).unwrap(),
            r#""week""#
        );
        assert_eq!(
            serde_json::to_string(&CompletionMetric::Lead).unwrap(),
            r#""lead""#
        );
        assert_eq!(
            serde_json::to_string(&CompletionStat::P90).unwrap(),
            r#""p90""#
        );
    }

    #[test]
    fn completion_metric_value_enum_parses_cycle() {
        let parsed = CompletionMetric::from_str("cycle", true).unwrap();
        assert_eq!(parsed, CompletionMetric::Cycle);
    }

    #[test]
    fn filters_normalize_tags_and_assignee() {
        let mut filters = MetricsFilters {
            kind: Some(Kind::Meta),
            tags: vec![
                "  alpha  ".into(),
                "beta".into(),
                "alpha".into(),
                " ".into(),
            ],
            assignee: Some("  agent-1  ".into()),
            children_of: None,
            include_cancelled: false,
        };

        filters.normalize();

        assert_eq!(filters.tags, vec!["alpha", "beta"]);
        assert_eq!(filters.assignee.as_deref(), Some("agent-1"));
    }

    #[test]
    fn query_normalize_orders_window_bounds() {
        let mut query = MetricsQuery {
            window: MetricsWindow {
                from: date("2026-02-10"),
                to: date("2026-02-01"),
            },
            bucket: MetricsBucket::Day,
            filters: MetricsFilters::default(),
        };

        query.normalize();

        assert_eq!(query.window.from, date("2026-02-01"));
        assert_eq!(query.window.to, date("2026-02-10"));
    }

    #[test]
    fn data_quality_empty_only_when_all_counts_zero() {
        let empty = DataQuality::default();
        assert!(empty.is_empty());

        let not_empty = DataQuality {
            missing_history_tasks: 1,
            inferred_samples: 0,
        };
        assert!(!not_empty.is_empty());
    }
}
