# Coordination Verbosity Levels (Low / Medium / High)

This page documents current operator guidance for choosing coordination detail level when using `tak mesh`, `tak blackboard`, and task-local sidecars.

## What this guidance does

Use a **low / medium / high** verbosity model to balance clarity and noise:

- keep routine pings short,
- provide durable structure when multiple agents are involved,
- escalate to high detail when blockers/risk are present.

## Source of truth and invariants

- Task lifecycle state remains in `.tak/tasks/*.json` and lifecycle history (`.tak/history/*.jsonl`).
- Mesh/blackboard messages are coordination artifacts, not task truth.
- There is currently **no persisted `verbosity` field** on tasks or blackboard notes.
- Agent-level defaults are persisted in work-loop runtime state (`.tak/runtime/work/states/*.json`) via `tak work --verbosity ...`.
- Per-message overrides use `--verbosity` on `tak handoff`, `tak mesh send`/`broadcast`, and `tak blackboard post`.
- Verbosity is an operator convention, aided by existing blackboard templates (`blocker`, `handoff`, `status`) and delta support (`--since-note`, `--no-change-since`).
- Sensitive-text checks on blackboard posts are warnings (non-blocking); redact before posting.

## CLI controls (default + per-command override)

Set an agent default for the active work loop:

```bash
tak work start --assignee agent-1 --verbosity high
```

Override on a specific coordination update without changing the saved default:

```bash
tak mesh send --from agent-1 --to agent-2 \
  --verbosity low \
  --message "Quick ACK: releasing reservation now."

tak blackboard post --from agent-1 --task 43 \
  --verbosity high \
  --template blocker \
  --message "Blocked by verify failure; requesting owner review."
```

When a non-default level is active (or explicitly overridden), CLI-emitted coordination text is prefixed with `[verbosity=<level>]`; blackboard notes also receive a `verbosity-<level>` tag for filtering.

## Trigger matrix

| Situation | Verbosity | Escalation threshold | Primary channel(s) | Minimum content |
|---|---|---|---|---|
| Routine progress with no blocker (start, reservation acquired/released, quick ACK) | **Low** | Default when no high trigger is active | mesh DM/broadcast (optional) | task ID + short status + next action |
| Cross-agent coordination update (review ask, shared-path change, work-loop status handoff prep) | **Medium** | Use when coordination spans agents but no high trigger is active | blackboard (`--template status`) + optional mesh ping | status, scope, verification snapshot, next owner/action |
| Failed verification escalation | **High** | Any failed verification command, or same failure repeats twice within 30 minutes | blackboard (`--template blocker`) + direct mesh ask | failed command + exit status, affected scope, mitigation attempt, requested owner/action |
| Reservation conflict escalation | **High** | Required path blocked >15 minutes, or 2 unanswered mesh pings within 10 minutes | blackboard (`--template blocker`) + direct mesh ask | blocker owner/path/age, requested release or handoff, fallback + timeout |
| Blocker-driven handoff escalation | **High** | Agent must pause because blocker/dependency cannot be resolved in current session | `tak handoff` + blackboard (`--template handoff`) | blocker reason, attempted mitigations, waiting owner, linked blocker note |
| Cross-task schema/interface change escalation | **High** | Change impacts shared contract used by >=2 tasks/features | blackboard (`--template status`) + direct mesh ping to affected owners | impact map (tasks + files), compatibility risk, verification sequence, explicit acknowledgement ask |

If unsure, choose **medium**. De-escalate from **high** after one full update cycle with no active high trigger.

## Objective high-verbosity escalation triggers (default thresholds)

Use these defaults unless your team has stricter repo-local policy:

| Trigger | Threshold | Required high-verbosity action |
|---|---|---|
| Verification failure on a contract or required local check | Immediate (first failure) | Post blocker/status note with failing command, result, and owner for next action |
| Reservation conflict blocking progress | Unresolved for **10+ minutes** after first direct ping | Post blocker note with owner/path/reason and explicit unblock ask |
| Task blocked with no progress | Blocked for **20+ minutes** | Post blocker note and include fallback/handoff path |
| Handoff due to blocker or unresolved risk | Immediate at handoff time | Use `tak handoff --summary` plus blackboard handoff/blocker note |
| Cross-task schema/API/contract change | Immediate before or at first rollout commit | Post high-verbosity status note listing impacted tasks/files and migration/rollback plan |
| Sensitive-text incident risk (token/secret detected in coordination text) | Immediate when warning appears | Redact text, repost corrected note, and notify affected owners with high-signal summary |

## Concrete examples

### Low verbosity (routine operational ping)

```bash
tak mesh send --from agent-1 --to agent-2 \
  --message "Task #43 started; editing docs/how/coordination-verbosity.md only."
```

Use low verbosity for quick state visibility. Keep it to 1–2 lines.

### Medium verbosity (durable shared status)

```bash
tak blackboard post --from agent-1 --template status --task 43 \
  --message "Drafted trigger matrix + examples; request docs review for wording."
```

Follow-up without repeating unchanged context:

```bash
tak blackboard post --from agent-1 --template status --task 43 \
  --since-note 120 --message "Applied feedback; clarified failed-verify trigger."
```

Or explicitly mark no material change:

```bash
tak blackboard post --from agent-1 --template status --task 43 \
  --since-note 120 --no-change-since \
  --message "Waiting on reviewer confirmation."
```

### High verbosity (blocker or handoff escalation)

```bash
tak blackboard post --from agent-1 --template blocker --task 43 \
  --message "Blocked >15m by docs/how reservation conflict; need owner release or handoff plan."

# Pair durable note with a direct ask
tak mesh send --from agent-1 --to agent-2 \
  --message "See B121 for blocker details on task #43; can you release docs/how or propose handoff?"

# If you must pause, include high-signal summary in lifecycle handoff
tak handoff 43 --summary "Blocked by docs/how reservation conflict; examples drafted, waiting for release/handoff."
```

For deep technical details (logs, repro steps, command output), store long-form content in task context/history and reference it from mesh/blackboard updates.

## Edge cases and failure behavior

- `tak blackboard post --no-change-since` requires `--since-note`; otherwise command fails.
- `tak blackboard post --task <id>` validates task existence; invalid IDs fail early.
- Sensitive-text detection emits warnings to stderr but does not block posting.
- Use delta follow-ups to avoid copy/paste repetition across multiple high-verbosity notes.

## Code pointers

- `src/commands/blackboard.rs`
  - template rendering (`render_template`)
  - delta metadata (`apply_delta_metadata`)
  - sensitive-text warnings (`detect_sensitive_text_warnings`)
- `src/commands/mesh.rs` (send/broadcast/inbox/reserve/release flow)
- `src/commands/lifecycle.rs` (`handoff` state transition + history side effects)
- `src/main.rs` (CLI `--verbosity` flags, default resolution, verbosity label/tag helpers)
- `src/store/sidecars.rs` (context/history storage)

## Test pointers

- `tests/integration.rs`
  - `test_blackboard_post_close_reopen`
  - `test_blackboard_post_invalid_task_link_fails`
  - `test_blackboard_post_blocker_template_formats_message_and_tags`
- `src/commands/blackboard.rs`
  - `apply_delta_metadata_with_change_placeholder`
  - `apply_delta_metadata_with_no_change_marker`
  - `template_includes_redaction_guidance`
  - `sensitive_detection_flags_common_markers`
  - `sensitive_detection_flags_jwt_like_values`
- `src/main.rs`
  - `parse_blackboard_post_verbosity_flag`
  - `parse_mesh_send_verbosity_flag`
  - `parse_handoff_verbosity_flag`
  - `parse_work_start_with_verbosity`
  - `apply_coordination_verbosity_label_*`
