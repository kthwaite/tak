# Metrics operator guide: burndown + completion-time interpretation

Use this guide when you need to answer:

- “Are we burning down scope at a healthy rate?”
- “Are tasks taking longer to finish, and is that queue time or execution time?”
- “How much should I trust this chart if history is incomplete?”

For implementation-level details, see [`docs/how/metrics.md`](./metrics.md).

## 1) Command quick map

- `tak metrics burndown` → remaining work trend (+ scope overlays)
- `tak metrics completion-time` → duration trend (`cycle` or `lead`)
- `tak metrics tui` → both views in one terminal dashboard

Common window controls:

- `--from YYYY-MM-DD`
- `--to YYYY-MM-DD`
- `--bucket day|week`

Defaults: if omitted, Tak uses a 30-day window ending today.

## 2) Burndown semantics (what moves the line)

Burndown tracks **remaining in-scope tasks** over time.

Lifecycle deltas:

- `created` → `+1` remaining
- `finished` → `-1` remaining
- `cancelled` → `-1` remaining
- `reopened` → `+1` remaining

Important fields:

- `series.actual` → real remaining line
- `series.ideal` → straight-line burn from `start_remaining` to `0`
- `series.scope_added` → creates in window
- `series.scope_removed` → cancellations in window

How to read it:

- Actual line above ideal for multiple buckets → burn is slower than plan.
- Flat remaining with non-zero completions often means scope growth is absorbing throughput.
- Reopen spikes suggest churn/regression/rework.

## 3) Completion-time semantics (lead vs cycle)

`tak metrics completion-time` reports durations for completion samples.

### Cycle time

`cycle = finished_at - episode_started_at`

Where episode start is:

- first `started`/`claimed`, or
- `reopened` when measuring a reopened pass.

### Lead time

`lead = finished_at - task.created_at`

Use this to include queue/wait time before active execution.

### Reopened behavior

A reopened task can contribute **multiple samples** (one for each subsequent finish).

How to use both metrics together:

- Lead rising, cycle stable → queueing/prioritization bottleneck.
- Cycle rising, lead rising → execution complexity/blockers are increasing.
- p90 rising faster than average → long-tail blockers are getting worse.

## 4) Bucketing caveat (easy to miss)

Weekly buckets are not labeled the same way across reports:

- Burndown week buckets are anchored to query `--from`.
- Completion-time week labels are ISO weeks (`YYYY-Www`).

So week-to-week overlays across the two reports are directional, not perfectly aligned by bucket boundary.

## 5) Data-quality confidence checks

Inspect `data_quality` before making strong claims.

Key indicators:

- `missing_history_tasks` → tasks lacking history sidecars
- `inferred_samples` → inferred fallback events and/or dropped invalid duration samples

Interpretation guidance:

- Low/zero values → trend confidence is higher.
- Higher values → treat exact values cautiously; use coarse direction over precise deltas.

## 6) Filter behavior and constraints

- `--children-of` is direct-child only (not recursive).
- `--include-cancelled` is supported for burndown/TUI.
- `--include-cancelled` is **invalid** for completion-time (returns `metrics_invalid_query`).

## 7) Troubleshooting playbook

### Symptom: “Burndown says no progress, but team finished tasks.”

Check:

1. Was scope added in same window? (`scope_added`)
2. Are you filtering to a slice that excludes completed items? (`--tag`, `--kind`, `--assignee`, `--children-of`)
3. Did reopen events offset finishes?

### Symptom: “Completion-time looks too good/too bad suddenly.”

Check:

1. Sample count (`summary.samples`) dropped?
2. `data_quality.inferred_samples` spiked?
3. Metric mismatch (`cycle` vs `lead`) across comparisons?
4. Window/bucket changed (`day` vs `week`)?

### Symptom: “TUI and CLI numbers seem different.”

Check:

1. TUI currently selected metric (`m`) and bucket (`b`)
2. Whether TUI window was shifted with `[` or `]`
3. Inclusion of cancelled tasks in the active query

## 8) Practical command recipes

```bash
tak metrics burndown \
  --from 2026-01-01 --to 2026-02-10 \
  --bucket week --tag metrics

tak metrics completion-time --metric cycle --from 2026-01-01 --to 2026-02-10
tak metrics completion-time --metric lead  --from 2026-01-01 --to 2026-02-10

tak metrics tui --from 2026-01-01 --to 2026-02-10
```

## 9) Operator checklist before reporting metrics

- Confirm window/bucket and filter scope.
- Confirm metric (`lead` vs `cycle`) is the one you intend.
- Read `data_quality` and sample counts.
- Check scope-change overlays before claiming throughput changes.
- Call out caveats (especially inferred/missing history) in your summary.
