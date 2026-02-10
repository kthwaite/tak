# tak

Git-native task manager for agentic workflows.

Tasks live as JSON files in `.tak/tasks/`, committed alongside your code. A gitignored SQLite index provides fast queries. Designed for multi-agent AI coordination via CLI.

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
tak create "Write API endpoints" --depends-on 0000000000000001
tak create "Add tests" --depends-on 0000000000000002

# See the dependency tree
tak tree --format pretty

# Preview available work
tak next

# Claim and complete a task (atomic: finds + starts in one step)
tak claim --assignee agent-1
tak finish 0000000000000001

# Check what's unblocked
tak list --available
```

## Commands

| Command | Description |
|---------|-------------|
| `tak init` | Initialize `.tak/` in the current repository |
| `tak create <title>` | Create a task (`--kind`, `--parent`, `--depends-on`, contract + planning flags) |
| `tak show <task-id>` / `tak list` / `tak tree [task-id]` | Query tasks and hierarchy |
| `tak edit <task-id>` | Update task metadata (`--title`, `--kind`, tags, contract/planning, `--pr`) |
| `tak claim` / `tak start <task-id>` | Start work (atomic claim preferred in multi-agent mode) |
| `tak finish <task-id>` / `tak handoff <task-id>` / `tak cancel <task-id>` | Close, hand off, or cancel execution |
| `tak reopen <task-id>` / `tak unassign <task-id>` | Reopen or clear assignment |
| `tak depend` / `tak undepend` / `tak reparent` / `tak orphan` | Manage dependency + parent-child edges |
| `tak context <task-id>` / `tak log <task-id>` / `tak verify <task-id>` | Task sidecars: notes, history, verification |
| `tak learn <subcommand>` | Manage learnings + suggestions |
| `tak mesh <subcommand>` | Agent presence, messaging, reservations, activity feed |
| `tak blackboard <subcommand>` | Shared coordination notes (`post/list/show/close/reopen`) |
| `tak therapist <subcommand>` | Workflow diagnosis (`offline`, `online`, `log`) |
| `tak migrate-ids [--dry-run\|--apply]` | Preflight/apply legacy numeric → hash-style task filename migration |
| `tak delete <task-id>` | Delete task (`--force` to cascade orphan/removal behavior) |
| `tak reindex` | Rebuild SQLite index from task files |
| `tak setup` / `tak doctor` | Install/check integrations and environment health |

All commands output JSON by default. Use `--format pretty` for human-readable output or `--format minimal` for tabular summaries.

## Task IDs and Migration

Canonical task IDs are 16-character lowercase hex strings (for example `000000000000002a`).

When a command expects a task ID, tak resolves input as:
1. exact canonical hex or legacy decimal match,
2. otherwise a unique hex prefix match (case-insensitive).

If your repository still has legacy numeric filenames (`.tak/tasks/1.json`, etc.), run:

```bash
# Preview preflight checks (dry-run is default when --apply is omitted)
tak migrate-ids --dry-run --format pretty

# Apply migration once preflight is clean
tak migrate-ids --apply --format pretty

# Refresh derived index metadata
tak reindex
```

`tak migrate-ids --apply` rewrites task + sidecar filenames, updates learning task links, bumps `.tak/config.json` to version 3, and writes an audit map under `.tak/migrations/task-id-map-<timestamp>.json`.

## Data Model

**Task kinds:** `epic`, `feature`, `task`, `bug`

**Statuses:** `pending` → `in_progress` → `done` / `cancelled`

**Two relationship types:**
- **Parent-child** (structural): epics contain tasks, tasks contain subtasks
- **Depends-on** (scheduling): task B cannot start until task A is done

**Blocked** is derived, never stored. A task is blocked when any of its dependencies are unfinished.

## Storage

```
.tak/
  config.json                         # Repository configuration
  counter.json                        # Legacy counter file (optional in hash-id mode)
  tasks/*.json                        # Task source of truth (16-char lowercase hex filenames)
  context/{task_id}.md                # Task notes sidecars (committed)
  history/{task_id}.jsonl             # Lifecycle history sidecars (committed)
  verification_results/{task_id}.json # Verification outputs (gitignored)
  artifacts/{task_id}/                # Task artifacts (gitignored)
  learnings/*.json                    # Learning records (committed)
  migrations/task-id-map-*.json       # ID migration audit logs (written by migrate-ids --apply)
  runtime/mesh/*                      # Agent registry/inbox/reservations/feed (gitignored)
  runtime/blackboard/*                # Shared note board (gitignored)
  therapist/log.jsonl                 # Workflow observations (committed)
  index.db                            # SQLite index (gitignored, rebuilt on demand)
```

Task files are the source of truth. The SQLite index is derived and rebuilt automatically when missing (e.g., after a fresh clone).

## Multi-Agent Workflow

Use `tak claim` for atomic task acquisition. It holds an exclusive file lock while finding and starting the next available task, preventing two agents from claiming the same work.

```bash
# Agent 1: atomically claim work
tak claim --assignee agent-1           # → finds and starts task 0000000000000003

# Agent 2: atomically claim different work
tak claim --assignee agent-2           # → finds and starts task 0000000000000005 (0000000000000003 is taken)

# Agent 1: finish and check for unblocked tasks
tak finish 0000000000000003
tak list --available                   # tasks blocked by 0000000000000003 are now available

# After pulling changes from other agents
git pull
tak reindex
tak list --available
```

Note: `tak next` + `tak start` is subject to TOCTOU races — another agent can claim the same task between the two commands. Prefer `tak claim` for concurrent workflows.

## Claude Code Integration

Tak ships as a Claude Code plugin. Enable it to get:

- **Task management skill**: Full CLI + coordination reference (tasks, mesh, blackboard, learnings, therapist)
- **Epic planning skill**: Guided decomposition of features into task hierarchies and dependency graphs
- **Task execution skill**: Agent workflow for claiming/executing/completing tasks, including conversational `/tak work`, `/tak work status`, and `/tak work stop` loop semantics
- **Session lifecycle hooks**: Auto-reindex + auto-join mesh on session start, auto-leave mesh on stop

> Note: Claude implements `/tak work` via skill instructions (conversational loop), while pi additionally enforces loop guards through extension runtime hooks.

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
- Auto `tak reindex` + mesh join/leave lifecycle behavior
- System-prompt augmentation that enforces active tak usage and cross-agent coordination

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
