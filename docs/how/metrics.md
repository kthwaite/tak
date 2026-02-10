# Metrics reporting (`tak metrics burndown`, `completion-time`, `tui`)

This page documents the current behavior of Tak’s metrics surfaces, including chart semantics and interpretation caveats.

## 1) What this subsystem does

Tak exposes three metrics entry points:

- `tak metrics burndown` — remaining-work trend with scope-change overlays
- `tak metrics completion-time` — lead/cycle completion-duration trend
- `tak metrics tui` — interactive terminal dashboard combining both views

The metrics pipeline reads task JSON + task history sidecars and derives timelines on demand (no separate persisted metrics store).

## 2) Source of truth + invariants

Primary implementation:

- `src/commands/metrics.rs`
- `src/metrics/derive.rs`
- `src/metrics/burndown.rs`
- `src/metrics/aggregate.rs`
- `src/metrics/tui.rs`

Core invariants:

- Metrics queries default to a 30-day window when `--from/--to` are omitted.
- Date windows must be non-inverted (`from <= to`).
- Bucket limits are enforced:
  - `day`: max 366 buckets
  - `week`: max 520 buckets
- `metrics completion-time --include-cancelled` is rejected (`metrics_invalid_query`).
- Filters are normalized before evaluation:
  - tags are trimmed/deduped/sorted
  - assignee is trimmed
- Cancelled tasks are excluded by default unless `--include-cancelled` is set (burndown/TUI only).

## 3) Execution flow (happy path)

1. Build + validate `MetricsQuery` from CLI flags (`--from`, `--to`, `--bucket`, filters).
2. Load tasks from repo store and apply filters (`kind`, tags, assignee, direct `children_of`).
3. Read per-task history sidecars (`.tak/history/<task-id>.jsonl`).
4. Derive normalized lifecycle timelines (including inferred fallback events when history is incomplete).
5. Aggregate:
   - burndown report (`actual`, `ideal`, `scope_added`, `scope_removed`, summary)
   - completion-time report (`series` buckets + summary avg/p50/p90)
6. Render in JSON/pretty/minimal, or map to TUI snapshot panels.

## 4) Chart semantics and interpretation

### Burndown

Burndown tracks **remaining in-scope tasks** over time.

Event effects:

- `created` => `+1` remaining
- `finished` => `-1` remaining
- `cancelled` => `-1` remaining
- `reopened` => `+1` remaining

Summary fields:

- `start_remaining`: remaining count immediately before window start
- `end_remaining`: remaining count after final bucket
- `completed_in_window`: number of finish events in window
- `reopened_in_window`: number of reopen events in window

Series overlays:

- `scope_added`: task creations in window
- `scope_removed`: cancellations in window
- `ideal`: straight-line burn from `start_remaining` to `0` across bucket count

Bucket semantics:

- `day`: one point per date
- `week`: 7-day buckets anchored to query `--from` (not ISO week)

Interpretation tips:

- Actual above ideal over multiple buckets => burn is slower than plan.
- High `scope_added` with flat remaining => scope growth is absorbing throughput.
- Rising `reopened_in_window` => churn/regression/rework pressure.

### Completion-time

Completion-time tracks how long completion episodes take.

Episode model:

- Each `finished` event contributes one sample.
- Reopened tasks can contribute multiple samples (one per subsequent finish).

Metric semantics:

- `cycle`: `finished_at - episode_started_at`
  - episode start is first `started`/`claimed`, or `reopened` for a reopened pass
- `lead`: `finished_at - task.created_at`

Bucketing + labels:

- Samples are bucketed by `finished_at` date.
- `day` labels: `YYYY-MM-DD`
- `week` labels: ISO week (`YYYY-Www`, Monday-based)

Stats:

- `avg_hours`, `p50_hours`, `p90_hours`, `samples`
- Percentiles use nearest-rank (`ceil(q * n)`) on sorted durations

Interpretation tips:

- Compare lead vs cycle to separate queue/wait time from active execution time.
- Trust trends more when sample counts are healthy.
- Watch p90 for long-tail blocker behavior.

## 5) Data-quality fields and caveats

Both reports may include `data_quality`:

- `missing_history_tasks`: tasks with empty history sidecar
- `inferred_samples`: inferred lifecycle fallbacks + dropped invalid duration samples

Additional caveats:

- `children_of` filter is direct-child only (non-recursive).
- Empty result windows produce empty series; summary stats may be absent (`null` in JSON, `-` in pretty/minimal).
- Negative-duration completion samples are dropped and counted in `inferred_samples`.

## 6) TUI behavior (`tak metrics tui`)

The metrics TUI refreshes periodically (default 250ms), rendering:

- left panel: burndown summary + sparklines (actual/ideal)
- right panel: completion summary + trend sparkline

Controls:

- `q` quit
- `r` refresh
- `b` toggle bucket (`day`/`week`)
- `m` toggle metric (`cycle`/`lead`)
- `[` shrink window start by one bucket
- `]` expand window start by one bucket
- `?` help overlay

## 7) JSON contract anchors

Stable top-level payloads:

- Burndown: `window`, `bucket`, `filters`, `series`, `summary`, optional `data_quality`
- Completion-time: `window`, `bucket`, `metric`, `series`, `summary`, optional `data_quality`

Invalid queries return structured stderr JSON with `error = "metrics_invalid_query"`.

## 8) Code pointers

- `src/commands/metrics.rs`
- `src/metrics/model.rs`
- `src/metrics/derive.rs`
- `src/metrics/burndown.rs`
- `src/metrics/aggregate.rs`
- `src/metrics/tui.rs`

## 9) Test pointers

- `tests/metrics_completion_time_cli_integration.rs`
- `src/commands/metrics.rs` unit tests (query validation/filter behavior)
- `src/metrics/derive.rs` unit tests (timeline normalization + inferred events)
- `src/metrics/burndown.rs` unit tests (scope/remaining aggregation)
- `src/metrics/aggregate.rs` unit tests (lead/cycle bucketing + stats)
- `src/metrics/tui.rs` unit tests (controls + render/refresh smoke)
