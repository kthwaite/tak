# Task Lifecycle and State Transitions

This page documents current lifecycle behavior for task execution commands.

## What this subsystem does

Lifecycle commands move tasks through status states and attach execution provenance:

- status transitions (`pending`, `in_progress`, `done`, `cancelled`)
- assignment updates
- execution metadata updates (`attempt_count`, `last_error`, `handoff_summary`)
- git provenance capture (`start_commit`, `end_commit`, commit summaries)
- append-only history events in sidecar JSONL

## Source of truth and invariants

Primary implementation:

- `src/commands/lifecycle.rs`
- `src/commands/claim.rs`
- `src/store/index.rs` (blocked checks)
- `src/store/sidecars.rs` (history logging)

Core invariant: transitions are enforced by a strict state machine (`transition()` in `lifecycle.rs`).

## Allowed transitions

| From | To | Command(s) |
|---|---|---|
| `pending` | `in_progress` | `start`, `claim` |
| `pending` | `cancelled` | `cancel` |
| `in_progress` | `done` | `finish` |
| `in_progress` | `cancelled` | `cancel` |
| `in_progress` | `pending` | `handoff` |
| `done` | `pending` | `reopen` |
| `cancelled` | `pending` | `reopen` |

Invalid edges return `TakError::InvalidTransition`.

## Command behavior

### `start TASK_ID [--assignee ...]`

- Validates `pending -> in_progress`.
- Rejects blocked tasks via `repo.index.is_blocked(id)` (`TakError::TaskBlocked`).
- Increments `execution.attempt_count`.
- Sets assignee if provided (or CLI-resolved agent when available).
- Captures git `branch` + `start_commit` only if not already present.
- Persists task + index upsert.
- Appends history event: `started` (best effort).

### `claim [--assignee ...] [--tag ...]`

- Acquires `.tak/claim.lock` for atomic select+start behavior.
- Chooses from `index.available(...)` ordering (priority -> created_at -> id).
- Optional `--tag` filters candidates by exact task tag membership.
- Sets `status = in_progress`, assignee, increments `attempt_count`.
- Captures git start provenance on first start.
- Persists task + index upsert.
- Appends history event: `claimed` (best effort).

### `finish TASK_ID`

- Validates `in_progress -> done`.
- Captures git `end_commit`; if `start_commit` exists, records commit summaries between start/end.
- Persists task + index upsert.
- Appends history event: `finished` (best effort).

### `work done [--assignee ...] [--pause]`

- Reads per-agent work-loop state from `.tak/runtime/work/states/<agent>.json`.
- If `current_task_id` points to an owned `in_progress` task, applies the same validated finish path as `finish TASK_ID` (including history + git provenance).
- Always attempts reservation cleanup for the calling agent (`mesh release --all` semantics for that agent) and reports release status in output.
- Emits a structured `done` report in JSON output with:
  - `lifecycle_transition` (`finished`, `detached_without_finish`, or `no_current_task`)
  - `finished_task_id` (when a task was actually finished)
  - `reservation_release` (`released`, `paths`, optional `error`)
  - `paused` and `loop_active`
- `--pause` deactivates the loop after cleanup; without `--pause`, the loop remains active after successful finish.

Practical examples:

```bash
# Finish current unit, release reservations, keep loop active
tak work done --assignee agent-1

# Finish + release + pause loop
tak work done --assignee agent-1 --pause
```

Troubleshooting:

- Re-running `tak work done` with no attached task is safe/idempotent (`lifecycle_transition = no_current_task`).
- If loop state points to a stale/non-owned task, `lifecycle_transition = detached_without_finish` and the stale pointer is cleared.
- If reservation release partially fails, inspect `done.reservation_release.error` in JSON output and retry `tak mesh release --name <agent> --all`.

### `cancel TASK_ID [--reason ...]`

- Validates `pending/in_progress -> cancelled`.
- Stores `--reason` into `execution.last_error` when provided.
- Persists task + index upsert.
- Appends history event: `cancelled` with optional `reason` detail (best effort).

### `handoff TASK_ID --summary ...`

- Validates `in_progress -> pending`.
- Clears assignee.
- Stores summary in `execution.handoff_summary`.
- Persists task + index upsert.
- Appends history event: `handoff` with `summary` detail (best effort).

### `reopen TASK_ID`

- Validates `done/cancelled -> pending`.
- Clears assignee.
- Persists task + index upsert.
- Appends history event: `reopened` (best effort).

### `unassign TASK_ID`

- Clears assignee without changing status.
- Persists task + index upsert.
- Appends history event: `unassigned` (best effort).

## Blocked semantics

Blocked is derived from dependency graph state in SQLite (`index.is_blocked` / `index.blocked`):

- a pending task is blocked when any dependency is not `done`/`cancelled`
- blocked status is not stored as a task field

## History logging behavior

Lifecycle commands call `SidecarStore::append_history` to `.tak/history/<task-id>.jsonl`.

Important nuance: history appends are best effort (`let _ = ...`), so lifecycle commands do not fail solely because history write failed.

## Common edge cases

- Re-starting a task after reopen/handoff increments `attempt_count` again.
- `start` does not overwrite existing `git.start_commit`.
- `finish` may have no commit list if git info is unavailable or no start commit was captured.
- `handoff` requires `--summary` at CLI argument level.

## Code pointers

- `src/commands/lifecycle.rs`
- `src/commands/claim.rs`
- `src/main.rs` (assignee resolution and command wiring)
- `src/store/index.rs` (`available`, `blocked`, `is_blocked`)
- `src/store/sidecars.rs` (`append_history`, `read_history`)

## Test pointers

In `tests/integration.rs`:

- `test_start_rejects_blocked_task`
- `test_reopen_transitions`
- `test_claim_assigns_next_available`
- `test_start_captures_git_info`
- `test_finish_captures_commit_range`
- `test_start_increments_attempt_count`
- `test_cancel_with_reason_sets_last_error`
- `test_handoff_records_summary_and_returns_to_pending`
- `test_log_shows_lifecycle_history`

In `tests/work_done_integration.rs`:

- `work_done_finishes_current_task_and_releases_reservations_integration`
- `work_done_pause_deactivates_loop_integration`
- `work_done_is_idempotent_on_repeated_invocation_integration`
- `work_done_detaches_stale_current_pointer_and_releases_reservations_integration`

In `src/commands/work.rs` unit tests:

- `done_finishes_current_task_releases_reservations_and_reports_subactions`
- `done_reports_detached_transition_when_current_task_is_not_owned_in_progress`
- `render_json_done_output_includes_subaction_report`
