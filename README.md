# tak

Git-native **stigmergic** task manager for agentic workflows.

Tak is built around a **bottom-up, society-of-agents** model: specialized workers coordinate through shared artifacts (tasks, dependencies, reservations, and coordination notes) instead of waiting on a central planner. Tasks live as JSON files in `.tak/tasks/`, committed alongside your code. A gitignored SQLite index provides fast queries, while mesh + blackboard runtime data supports live multi-agent coordination.

## Install

```bash
cargo install --path .
```

## Quick Start

```bash
# Initialize in a git repository
tak init

# Create tasks
tak create "Set up database" --kind task
tak create "Refine epic scope" --kind meta
tak create "Write API endpoints" --depends-on 0000000000000001
tak create "Add tests" --depends-on 0000000000000002

# See the dependency tree
tak tree --format pretty

# Preview available work
tak next

# Claim and complete a task (atomic: finds + starts in one step)
tak claim --assignee agent-1
tak finish 0000000000000001

# Reassign a stale in-progress task safely
tak takeover 0000000000000001 --assignee agent-2 --inactive-secs 1800

# Check what's unblocked
tak list --available

# Or run the CLI-native work loop
# (default action is start/resume)
tak work --assignee agent-1
tak work status --assignee agent-1
tak work done --assignee agent-1 --pause
tak work stop --assignee agent-1
```

**Task ID input forms:** every `<task-id>` argument accepts a full canonical 16-char hex ID, a unique hex prefix (case-insensitive), or a legacy decimal ID. Resolution is exact-match first, then unique prefix; ambiguous prefixes error.

Examples:
- `tak show ef94`
- `tak depend b48b --on ef94`

### `tak work done` closeout helper

Use `tak work done` when you want one command to close the current unit and clean up loop state.

```bash
# Finish current task + release your reservations, keep loop active for next claim
tak work done --assignee agent-1

# Finish + release + pause loop (no auto-claim until you restart)
tak work done --assignee agent-1 --pause
```

Troubleshooting:

- If no current task is attached, `tak work done` is idempotent and reports a `no_current_task` transition.
- If loop state points at a stale/non-owned task, it reports `detached_without_finish`, clears the stale pointer, and still releases reservations.
- If reservation release fails, JSON output includes `done.reservation_release.error` for follow-up.

### `tak takeover` stale-owner transfer helper

Use `tak takeover` when a task is still `in_progress` but the current owner appears stale.

```bash
# Only take over when owner inactivity meets threshold
tak takeover <task-id> --assignee agent-2 --inactive-secs 1800

# Emergency override (bypass inactivity guardrail)
tak takeover <task-id> --assignee agent-2 --force
```

Guardrails and rollback:

- Default safety path requires stale ownership evidence (`inactive-secs` or default mesh registration TTL).
- Success output includes `previous_owner`, `decision`, and resulting assignment/state in json/pretty/minimal modes.
- If you took ownership too early, hand it back immediately:
  - `tak handoff <task-id> --summary "takeover reverted; owner still active"`
  - or `tak unassign <task-id>` and coordinate via blackboard.

## Stigmergic Coordination Model

Tak is optimized for **coordination through shared state** in a Minsky-style "society of mind": many narrow specialists, one shared memory.

- **Tasks + dependencies** (`.tak/tasks/*.json`) are the durable plan and execution truth.
- **Mesh runtime** (`tak mesh ...`) handles live presence, short-lived signals, and file-path reservations.
- **Blackboard notes** (`tak blackboard ...`) capture durable team-level context, blockers, and handoffs.
- **Task sidecars** (`tak context`, `tak log`) preserve local implementation context and lifecycle history.

Bottom-up principles:
- **Local decisions first:** agents choose and execute the next viable move from current context.
- **Global order emerges:** priority + dependencies + availability produce system-level flow.
- **Coordination is environmental:** progress signals live in artifacts, not in a central conductor.

In practice: claim work, reserve paths, post meaningful updates in shared channels, run learnings closeout, then finish/handoff and release reservations.

## Commands

Task ID arguments across commands accept canonical hex, unique hex prefixes, and legacy decimal IDs.

| Command | Description |
|---------|-------------|
| `tak init` | Initialize `.tak/` in the current repository |
| `tak create <title>` | Create a task (`--kind`, `--parent`, `--depends-on`, contract + planning flags) |
| `tak import <source>` | Import strict YAML v2 plans (`epic` + `features` + `tasks`; `--dry-run` previews without writing) |
| `tak show <task-id>` / `tak list` / `tak tree [task-id]` | Query tasks and hierarchy |
| `tak edit <task-id>` | Update task metadata (`--title`, `--kind`, tags, contract/planning, `--pr`) |
| `tak claim` / `tak start <task-id>` | Start work (atomic claim preferred in multi-agent mode) |
| `tak takeover <task-id>` | Transfer stale in-progress ownership with guardrails (`--inactive-secs`, `--force`) |
| `tak work [start\|status\|stop\|done]` | CLI-native work loop controller (resume/claim, inspect state, stop loop, or finish+release with optional pause) |
| `tak finish <task-id>` / `tak handoff <task-id>` / `tak cancel <task-id>` | Close, hand off, or cancel execution |
| `tak reopen <task-id>` / `tak unassign <task-id>` | Reopen or clear assignment |
| `tak depend` / `tak undepend` / `tak reparent` / `tak orphan` | Manage dependency + parent-child edges (including bulk reparent via `tak reparent <id1,id2,...> --to <parent-id>`) |
| `tak wait` | Deterministically wait for reservation/path or dependency blockers to clear (`--path` or `--on-task`, optional `--timeout`) |
| `tak context <task-id>` / `tak log <task-id>` / `tak verify <task-id>` | Task sidecars: notes, history, verification |
| `tak learn <subcommand>` | Manage learnings + suggestions |
| `tak mesh <subcommand>` | Agent presence + coordination ops (`join/list/send/inbox`, lease upkeep `heartbeat/cleanup`, reservation diagnostics `blockers/reservations`, `reserve/release/feed`) |
| `tak blackboard <subcommand>` | Shared coordination notes (`post/list/show/close/reopen`), including structured `post --template blocker\|handoff\|status`, delta refs (`--since-note`, `--no-change-since`), and sensitive-text lint warnings |
| `tak therapist <subcommand>` | Workflow diagnosis (`offline`, `online`, `log`) |
| `tak metrics <burndown\|completion-time\|tui>` | Metrics trends + dashboard (`--from/--to`, `--bucket day\|week`, filters; completion-time `--metric lead\|cycle`; `--include-cancelled` for burndown/TUI only) |
| `tak tui` | Interactive cross-domain explorer for tasks/learnings/blackboard/mesh/feed with in-UI search + detail panes (`--focus`, `--query`) |
| `tak migrate-ids [--dry-run\|--apply]` | Task ID migration workflow (preflight/apply rewrite + audit map + config bump) |
| `tak delete <task-id>` | Delete task (`--force` to cascade orphan/removal behavior) |
| `tak reindex` | Rebuild SQLite index from task files |
| `tak setup` / `tak doctor` | Install/check integrations and environment health |

### Import v2 (strict YAML plan materializer)

`tak import` now accepts one canonical YAML schema: top-level `epic`, nested `features`, and nested `tasks`.

> Breaking change: legacy import payloads (for example top-level `tasks:` list/JSON wrappers from older versions) are no longer accepted.

```yaml
epic: Agentic Chat
description: Claude Code for writers
tags: [agentic-chat]
priority: high

features:
  - &infra
    alias: infra
    title: Tool Infrastructure
    priority: high
    tasks:
      - &schemas
        alias: schemas
        title: Define tool schemas in server/src/tools/
        tags: [backend]
        estimate: m

      - title: Add agentMode flag to ChatCompletionRequest
        depends_on: [*schemas]

  - &read
    alias: read
    title: Read Tools
    depends_on: [*infra]
    tasks:
      - title: Wire up get_outline and read_scene handlers

  - title: Write Tools and Approval Gates
    depends_on: [*read]
    tasks:
      - &approval
        alias: approval
        title: Build approval gate UI component
        estimate: l

      - title: Wire up edit_scene tool with diff view
        depends_on: [*approval, *schemas]
```

Symbolic dependency references are document-scoped and resolved during import (no hex ID juggling needed).

```bash
# Validate + preview hierarchy/dependencies/metadata without writes
tak import plan.yaml --dry-run

# Materialize the full plan in one command
tak import plan.yaml
```

See [`docs/how/import-v2.md`](docs/how/import-v2.md) for schema details and migration notes.

### Bulk reparent example

```bash
# Bundle multiple tasks under one parent in a single command
# (all-or-nothing validation: if any ID is invalid or cyclic, nothing is changed)
tak reparent 00000000000000a1,00000000000000a2,00000000000000a3 --to 00000000000000ff

# Optional quiet mode for scripts
# (suppresses mutation output)
tak reparent 00a1,00a2 --to 00ff --quiet
```

### Metrics quick examples

```bash
# Burndown trend by week (last month window can be overridden)
tak metrics burndown --bucket week --from 2026-01-01 --to 2026-02-10 --tag metrics

# Completion-time trend (cycle time by default; use lead for end-to-end)
tak metrics completion-time --metric lead --bucket week --kind task

# Human-readable summaries
tak --format pretty metrics burndown --bucket day
tak --format minimal metrics completion-time --metric cycle

# Interactive dashboard
tak metrics tui --bucket day --metric cycle

# Cross-domain explorer (tasks + learnings + coordination runtime)
tak tui --focus tasks --query blocker
```

Option semantics:
- Default window is the last 30 days (`--to` defaults to today, `--from` defaults to 30 days earlier).
- `--include-cancelled` is supported for `metrics burndown` and `metrics tui`; `metrics completion-time --include-cancelled` is rejected with `metrics_invalid_query`.
- `--from` must be on or before `--to`; invalid combinations return structured `metrics_invalid_query` errors.

All commands output JSON by default. Use `--format pretty` for human-readable output or `--format minimal` for tabular summaries.

### Epic close hygiene gate (tak source repository)

When finishing an **epic** in the tak source repository and the epic's commit range changes tak functionality (`src/`, `pi-plugin/`, `claude-plugin/`, `Cargo.toml`/`Cargo.lock`), `tak finish` enforces a close-out gate:

- docs were updated in-range (`README.md`, `CLAUDE.md`, `docs/`, or skill docs),
- the running `tak` binary is built from current `HEAD`,
- project-local `.pi` integration is synced with `pi-plugin/`.

If blocked, follow the guided actions from the error (typically `cargo install --path .` and `tak setup --pi`) before retrying `tak finish`.

## Task IDs and Migration

Canonical task IDs are 16-character lowercase hex strings (for example `000000000000002a`).

New task IDs are allocated from OS-backed CSPRNG entropy (counterless allocation). On create, tak retries on filename collision under a task-ID allocation lock.

When a command expects a task ID, tak resolves input as:
1. exact canonical hex or legacy decimal match,
2. otherwise a unique hex prefix match (case-insensitive).

Prefix examples:
- `tak show ef94`
- `tak depend b48b --on ef94`

If your repository still has legacy numeric filenames (`.tak/tasks/1.json`, etc.), run:

```bash
# Preview preflight checks (dry-run is default when --apply is omitted)
tak migrate-ids --dry-run --format pretty

# Apply migration once preflight is clean
tak migrate-ids --apply --format pretty

# Optionally re-key already-canonical repositories to fresh random IDs
tak migrate-ids --apply --rekey-random --format pretty

# Refresh derived index metadata
tak reindex
```

`tak migrate-ids --apply` rewrites task + sidecar filenames, updates learning task links, bumps `.tak/config.json` to version 3, and writes an audit map under `.tak/migrations/task-id-map-<timestamp>.json`.

Adding `--rekey-random` remaps all task IDs (including already-canonical repositories) to fresh random IDs while preserving parent/dependency/learnings/sidecar references.

## Data Model

**Task kinds:** `epic`, `feature`, `task`, `bug`, `meta`

**Statuses:** `pending` → `in_progress` → `done` / `cancelled`

**Two relationship types:**
- **Parent-child** (structural): epics contain tasks, tasks contain subtasks
- **Depends-on** (scheduling): task B cannot start until task A is done

**Blocked** is derived, never stored. A task is blocked when any of its dependencies are unfinished.

## Storage

```
.tak/
  config.json                         # Repository configuration
  task-id.lock                        # Task ID allocation lock file (created on first task create)
  tasks/*.json                        # Task source of truth (16-char lowercase hex filenames)
  context/{task_id}.md                # Task notes sidecars (committed)
  history/{task_id}.jsonl             # Lifecycle history sidecars (committed)
  verification_results/{task_id}.json # Verification outputs (gitignored)
  artifacts/{task_id}/                # Task artifacts (gitignored)
  learnings/*.json                    # Learning records (committed)
  migrations/task-id-map-*.json       # ID migration audit logs (written by migrate-ids --apply)
  runtime/coordination.db             # Coordination runtime DB (WAL, gitignored)
  runtime/coordination.db-wal         # SQLite WAL sidecar (gitignored)
  runtime/coordination.db-shm         # SQLite shared-memory sidecar (gitignored)
  runtime/work/states/*.json          # Per-agent work-loop state (gitignored)
  therapist/observations.jsonl        # Workflow observations (committed)
  index.db                            # SQLite index (gitignored, rebuilt on demand)
```

Task files are the source of truth. The SQLite index is derived and rebuilt automatically when missing (e.g., after a fresh clone).

Coordination state is runtime-only and now backed by `.tak/runtime/coordination.db`. Legacy `.tak/runtime/mesh/` and `.tak/runtime/blackboard/` directories are inert after migration; `tak doctor --fix` can clean them up.

## Stigmergic Multi-Agent Workflow

The default loop is bottom-up and stigmergic: agents coordinate through shared task state, reservations, and notes instead of ad-hoc direct pings or top-down command routing.

```bash
# 1) Claim next available work (ordered by priority, then age)
tak claim --assignee agent-1

# 2) Reserve the paths you will touch before major edits
tak mesh reserve --name agent-1 --path src/store/coordination_db.rs --reason task-0000000000000003

# 3) Publish durable team context / blockers
tak blackboard post --from agent-1 --template status --task 0000000000000003 --message "Reservation diagnostics in progress"

# 4) Keep task-local notes close to execution
tak context 0000000000000003 --set "Need overlap checks for parent/child paths"

# 5) Finish (or handoff), then release reservations
tak finish 0000000000000003
# tak handoff 0000000000000003 --summary "Parser done; tests remain"
tak mesh release --name agent-1 --path src/store/coordination_db.rs
```

### Implementation-cycle learnings closeout (required)

Before you end a cycle (`finish`, `handoff`, or `cancel`), do a learnings pass:

1. Capture reusable insight/pitfall/pattern with `tak learn add ... --task <id>` (or `tak learn edit ... --add-task <id>`).
2. Ensure `.tak/learnings/*.json` (and any linked task JSON updates) are committed in the same implementation cycle.
3. Do not defer learning commits to a later unrelated change.

Template shortcuts for consistent high-signal notes:

Structured template notes serialize as line-based `key: value` fields (`template`, `summary`, `status`, `scope`, `owner`, `verification`, `blocker`, `next`; blocker adds `requested_action`).

```bash
tak blackboard post --from agent-1 --template blocker --task 0000000000000003 --message "Blocked on reservation conflict"
tak blackboard post --from agent-1 --template handoff --task 0000000000000003 --message "Handing off after parser pass"
tak blackboard post --from agent-1 --template status --task 0000000000000003 --message "Verification pass complete"

# Delta-style follow-up to avoid repeating unchanged context
# (references B42 and marks no material changes)
tak blackboard post --from agent-1 --template status --task 0000000000000003 --since-note 42 --no-change-since --message "Waiting on owner confirmation"
```

Unstructured mode remains supported: omit `--template` to post plain free-text notes exactly as before.

`tak blackboard post` also emits non-blocking warnings when text looks like a secret/token. Redact sensitive values before posting (for example `sk-...abcd` -> `<redacted:...abcd>`).

`tak claim` is atomic and avoids TOCTOU races that can happen with `tak next` + `tak start`. For concurrent work, prefer claim-first execution plus explicit reservation management.

## Isolated Verification Fallback

Default practice is to run verification in the shared working tree.

If unrelated in-progress edits make shared-tree verification noisy, use a temporary detached `git worktree` for targeted verification, then clean it up. Always note this in blackboard/task updates so evidence provenance is explicit.

See: [`docs/how/isolated-verification.md`](./docs/how/isolated-verification.md).

## Claude Code Integration

Tak ships as a Claude Code plugin. Enable it to get:

- **Task management skill**: Full CLI + coordination reference (tasks, mesh, blackboard, learnings, therapist)
- **Epic planning skill**: Guided decomposition of features into task hierarchies and dependency graphs
- **Task execution skill**: Agent workflow for claiming/executing/completing tasks, centered on CLI-native `tak work`, `tak work status`, `tak work done`, and `tak work stop` (with `/tak work*` chat aliases)
- **Session lifecycle hooks**: Auto-reindex + auto-join mesh on session start, auto-leave mesh on stop

> Note: Prefer invoking `tak work` commands directly. Claude `/tak work*` prompts are shorthand that run the same CLI-first flow; pi additionally layers extension runtime guardrails.

```bash
# Run from your git repo root
# Install SessionStart + Stop hooks into .claude/settings.local.json
tak setup

# Also write plugin files under .claude/plugins/tak
tak setup --plugin

# Install only Claude skills under .claude/skills/
tak setup --skills
```

## Pi Integration

Tak also ships a pi package under [`pi-plugin/`](./pi-plugin):

- `/tak` task picker with filtering (`ready`, `blocked`, `all`, `mine`, `in_progress`) and default **urgent → oldest** ordering
- `tak_cli` tool for structured task/mesh/blackboard command execution
- Mesh + blackboard integration (`/tak mesh`, `/tak inbox`, `/tak blackboard`)
- Guard inputs are CLI-backed (`tak mesh reservations`, `tak mesh list`) rather than direct runtime file reads
- Auto `tak reindex` + mesh join/leave lifecycle behavior
- System-prompt augmentation that enforces active tak usage and cross-agent coordination
- Fail-safe guard behavior: when reservation snapshots are unavailable, guarded write/verify paths block with actionable remediation guidance

Install project-local pi integration from the repo root:

```bash
tak setup --pi
```

(Use `tak setup --global --pi` to install into `~/.pi/agent/` instead.)

You can also install the package manually:

```bash
pi install ./pi-plugin -l
```

Then run `pi` in this repository and use `/tak`.

## License

MIT
