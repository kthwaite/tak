---
name: tak-task-execution
description: 'Use when an agent needs to find, claim, execute, and complete tasks from a tak task list, including when the user asks for "/tak work", "/tak work status", or "/tak work stop" behavior.'
allowed-tools: "Read,Bash(tak:*)"
---

# Task Execution with Tak

Systematic workflow for agents to find available work, claim it, execute it, and report completion in tak-managed repositories.

**Critical:** update task state via `tak` commands only (`claim`, `start`, `handoff`, `finish`, `cancel`, etc.). Never manually edit `.tak/*` data files.

## Task ID input forms

Wherever a command expects a task ID, you can pass:
- canonical 16-char hex ID,
- unique hex prefix (case-insensitive), or
- legacy decimal ID.

Resolution is exact-match first (canonical or legacy), then unique prefix; ambiguous prefixes return an error, so use a longer prefix.

Examples: `tak show ef94`, `tak finish ef94`.

## Primary path: `tak work` (event-driven)

`tak work` is a reconciliation engine. Each invocation checks durable state, reconciles against reality, and returns a single JSON response with an event telling you what happened.

```bash
tak work [--assignee <agent>] [--tag <tag>] [--limit <n>] [--verify isolated|local] [--strategy priority_then_age|epic_closeout] [--verbosity low|medium|high]
tak work status [--assignee <agent>]
tak work stop [--assignee <agent>]
```

When the user requests `/tak work`, interpret it as:

```text
/tak work [tag:<tag>] [limit:<n>] [verify:isolated|local] [strategy:<strategy>] [auto|cue:auto|cue:editor]
```

- Default (`cue:editor`) behavior: run `tak work` as normal reconciliation/claim flow.
- `auto` / `cue:auto` behavior: **do not auto-claim** a new task. Instead cue the current epic and candidate leaf tasks so the agent/user chooses explicitly.

### `/tak work auto` (epic-cue mode, no auto-claim)

When user intent is `/tak work auto` (or `cue:auto`), use this flow:

1. Resolve identity (`--assignee` or `TAK_AGENT`) and check current ownership:
   ```bash
   tak work status --assignee <agent>
   ```
2. If `current_task` exists, continue that task (normal cue/resume).
3. If no current task, build cue context without claiming:
   ```bash
   tak list
   tak list --available [--tag <tag>]
   tak list --status pending --kind epic [--tag <tag>]
   ```
4. From available tasks, prefer **pending leaves with valid ancestors** (no open children; no done/cancelled/missing/cyclic ancestors).
5. Pick the “current epic” as oldest pending epic (prefer one with ready leaf candidates), then present candidate leaf tasks and ask for explicit selection.
6. Start selected task explicitly:
   ```bash
   tak start <task-id> --assignee <agent>
   ```

Use standard `tak work` claiming mode only when cue mode is editor/default or when user explicitly asks to claim now.

### JSON output contract

Every `tak work` call returns:

```json
{
  "event": "<event>",
  "agent": "<agent-name>",
  "ephemeral_identity": true,
  "loop": {
    "active": true,
    "current_task_id": "ef9433bbf813d4de",
    "tag": "cli",
    "remaining": 2,
    "processed": 1,
    "verify_mode": "isolated",
    "claim_strategy": "priority_then_age",
    "coordination_verbosity": "medium",
    "started_at": "...",
    "updated_at": "..."
  },
  "current_task": { /* full task object or null */ }
}
```

`ephemeral_identity` appears only when `true` — it means no `--assignee` or `TAK_AGENT` was set, so a random name was generated. Loop state won't persist across invocations. Fix by setting `TAK_AGENT` or passing `--assignee`.

### Event interpretation

| Event | Meaning | Your action |
|-------|---------|-------------|
| `continued` | Stored current task is still in_progress and owned by you | Resume working on `current_task` |
| `attached` | Found an existing in_progress task you own (not in loop state) | Resume working on `current_task` |
| `claimed` | Atomically claimed and started a new task for you | Read `current_task` and begin work |
| `no_work` | No available tasks matching your filters | Stop loop; loop is auto-deactivated |
| `limit_reached` | Processed count hit the `--limit` ceiling | Stop loop; loop is auto-deactivated |

### What standard `tak work` (claiming mode) handles automatically

- Agent identity resolution (`--assignee` > `TAK_AGENT` env > generated fallback)
- Detecting and resuming your current in-progress task
- Atomic claiming of next available task
- Limit tracking and auto-deactivation
- Reservation cleanup on task transitions and stop

### What you still do manually after `tak work` returns a task

1. **Load additional execution context** (the `current_task` payload has the task, but not sidecar data):
   ```bash
   tak context <id>
   tak blackboard list --status open --task <id>
   ```

2. **Coordinate before major edits**:
   ```bash
   tak mesh reserve --name <agent> --path <path> --reason task-<id>
   ```

3. **Execute the task** — do the actual work.

4. **Signal completion or transition**:
   - Done:
     ```bash
     tak finish <id>
     ```
   - Blocked and stepping away:
     ```bash
     tak handoff <id> --summary "<what is done, what is blocked, and exact next step>"
     tak blackboard post --from <agent> --template handoff --task <id> --message "<handoff target + first action>"
     ```
   - Abandon:
     ```bash
     tak cancel <id> --reason "<why>"
     ```

5. **Run learnings closeout and commit it in-cycle**:
   ```bash
   tak learn add "<title>" --category <insight|pitfall|pattern|tool|process> -d "<what changed + why>" --task <id>
   # or update an existing learning link
   tak learn edit <learning-id> --add-task <id>
   ```
   Ensure `.tak/learnings/*.json` (plus linked task updates) is committed in this same implementation cycle.

6. **Release reservations**:
   ```bash
   tak mesh release --name <agent> --all
   ```

7. **Iterate**:
   - Claiming mode: call `tak work` again; it will detect previous completion, advance counters, and claim the next task.
   - Auto cue mode: call `/tak work auto` again to refresh epic context and pick the next leaf explicitly.

### Blocker cooperation

When blocked on a reservation overlap:

```bash
# 1) Diagnose owner/path/reason/age
tak mesh blockers --path <path-you-need>

# 2) Immediate operational ask (mesh)
tak mesh send --from <agent> --to <owner> --message "Task #<id> blocked on <path>; request release or handoff."

# 3) Durable blocker record (blackboard)
tak blackboard post --from <agent> --template blocker --verbosity high --task <id> \
  --message "Blocked by <owner> on <path> (reason=<reason>, age=<age>s); requested <exact action>; next check in 120s."

# 4) Deterministic wait window
tak wait --path <blocking-path> --timeout 120
```

When blocked on an unfinished dependency:

```bash
tak blackboard post --from <agent> --template blocker --verbosity high --task <id> \
  --message "Blocked on dependency <dep-id>; requested owner update; next check in 120s."
tak wait --on-task <blocking-task-id> --timeout 120
```

If a wait timeout expires, post a delta follow-up (avoid repeating unchanged context):

```bash
tak blackboard post --from <agent> --template status --task <id> \
  --since-note <note-id> --no-change-since \
  --message "Still blocked after 120s; re-pinged owner and retrying wait window."
```

### `/tak work status`

```bash
tak work status [--assignee <agent>]
```

Returns a `"status"` event with the current loop state and task. Supplement with:

```bash
tak list --available
tak list --blocked
tak blackboard list --status open --limit 10
tak mesh blockers
```

### `/tak work stop`

```bash
tak work stop [--assignee <agent>]
```

Deactivates the loop and releases reservations. If there is an in-progress task, leave it in a truthful lifecycle state (`in_progress` if still active, or handoff/cancel if stepping away). Prefer a `--template handoff` blackboard note when pausing blocked work.

## Verify mode semantics

When `--verify isolated` (default), use **path-scoped** gating:

1. Derive verify scope (`V`) from your owned reservations.
2. Collect foreign reservations (`F`) from other active agents.
3. Decision model:
   - `V` empty + `F` empty: allow.
   - `V` empty + `F` non-empty: block with guidance to reserve scope or switch to local mode.
   - overlap(`V`,`F`): block.
   - no-overlap(`V`,`F`): allow.

If blocked:

```bash
tak mesh blockers --path <scope-or-blocking-path>
tak mesh send --from <agent> --to <owner> --message "Verify blocked for task #<id> on <path>; request release/window or handoff."
tak blackboard post --from <agent> --template blocker --verbosity high --task <id> \
  --message "Blocked verify in isolated mode by <owner>/<path> (reason=<reason>, age=<age>s); requested <action>; waiting 120s."
tak wait --path <blocking-path> --timeout 120
```

When `--verify local`: run normal local verification (no reservation-based blocking).

## Fallback: manual orchestration

When `tak work` is not available or doesn't fit, drive the loop manually:

1. **Ensure agent identity**:
   ```bash
   tak mesh join --format minimal
   ```

2. **Attach or claim**:
   ```bash
   tak list --status in_progress --assignee <agent>
   # If none:
   tak claim --assignee <agent> [--tag <tag>]
   ```

3. **Load context, coordinate, execute, finish/handoff** — same as steps 1-5 above.

4. **Release reservations**:
   ```bash
   tak mesh release --name <agent> --all
   ```

5. **Repeat** until no work or limit reached.

## Standard (non-loop) workflow

1. Claim: `tak claim --assignee <your-name>`
2. Understand: `tak show <id>`
3. Execute.
4. Capture follow-up work: `tak create "<title>" --kind task -d "<context>"`
5. Finish or hand off: `tak finish <id>` / `tak handoff <id> --summary "..."`

## Multi-agent requirements

- Prefer `tak claim` over `tak next` + `tak start` to avoid TOCTOU races.
- Reindex after pull/merge/branch switch: `tak reindex`
- Do not silently take over tasks assigned to another agent.
- Use mesh + blackboard when blocked on reservations or cross-agent dependencies.
- Run `tak mesh blockers` before posting blocker notes so owner/path/reason/age are concrete.
- Use deterministic waits (`tak wait --path ...` / `tak wait --on-task ...`) instead of manual polling.
- Prefer structured templates (`blocker`, `handoff`, `status`) plus delta follow-ups (`--since-note`, `--no-change-since`) for durable coordination notes.

## Status transitions

```
pending ──→ in_progress ──→ done
   │              │
   │              ├──→ cancelled
   │              └──→ pending (handoff)
   │
   └──→ cancelled

done/cancelled ──→ pending (reopen)
```

## Best practices

- Keep one active task at a time unless explicitly parallelizing.
- Use handoff summaries that are actionable for the next agent.
- Always run a learnings closeout before ending a cycle, and commit `.tak/learnings/*.json` updates in that same cycle.
- Keep reservation scope narrow (file/dir level, not repo-wide).
- Keep blackboard notes concise: blocker owner/path, requested action, and next check time.
