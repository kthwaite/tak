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
tak create "Write API endpoints" --depends-on 1
tak create "Add tests" --depends-on 2

# See the dependency tree
tak tree --format pretty

# Preview available work
tak next

# Claim and complete a task (atomic: finds + starts in one step)
tak claim --assignee agent-1
tak finish 1

# Check what's unblocked
tak list --available
```

## Commands

| Command | Description |
|---------|-------------|
| `tak init` | Initialize `.tak/` in the current repository |
| `tak create <title>` | Create a task (`--kind`, `--parent`, `--depends-on`, `-d`, `--tag`) |
| `tak delete <id>` | Delete a task (`--force` to cascade and orphan children / remove deps) |
| `tak show <id>` | Show task details |
| `tak list` | List tasks (`--status`, `--kind`, `--tag`, `--assignee`, `--available`, `--blocked`, `--children-of`) |
| `tak edit <id>` | Edit task fields (`--title`, `-d`, `--kind`, `--tag`) |
| `tak start <id>` | Set status to in_progress (`--assignee`) |
| `tak finish <id>` | Set status to done |
| `tak cancel <id>` | Set status to cancelled |
| `tak claim` | Atomic next+start with file lock (`--assignee`, `--tag`) |
| `tak reopen <id>` | Done/cancelled back to pending (clears assignee) |
| `tak unassign <id>` | Clear assignee without changing status |
| `tak depend <id> --on <ids>` | Add dependency edges (with cycle detection) |
| `tak undepend <id> --on <ids>` | Remove dependency edges |
| `tak reparent <id> --to <id>` | Change a task's parent |
| `tak orphan <id>` | Remove a task's parent |
| `tak tree [<id>]` | Show task hierarchy |
| `tak next` | Show the next available task (preview only) |
| `tak reindex` | Rebuild SQLite index from task files |

All commands output JSON by default. Use `--format pretty` for human-readable output or `--format minimal` for tabular summaries.

## Data Model

**Task kinds:** `epic`, `task`, `bug`

**Statuses:** `pending` → `in_progress` → `done` / `cancelled`

**Two relationship types:**
- **Parent-child** (structural): epics contain tasks, tasks contain subtasks
- **Depends-on** (scheduling): task B cannot start until task A is done

**Blocked** is derived, never stored. A task is blocked when any of its dependencies are unfinished.

## Storage

```
.tak/
  config.json         # Repository configuration
  counter.json        # Next task ID (locked during creation)
  tasks/
    1.json            # One file per task (committed to git)
    2.json
  index.db            # SQLite index (gitignored, rebuilt on demand)
```

Task files are the source of truth. The SQLite index is derived and rebuilt automatically when missing (e.g., after a fresh clone).

## Multi-Agent Workflow

Use `tak claim` for atomic task acquisition. It holds an exclusive file lock while finding and starting the next available task, preventing two agents from claiming the same work.

```bash
# Agent 1: atomically claim work
tak claim --assignee agent-1           # → finds and starts task 3

# Agent 2: atomically claim different work
tak claim --assignee agent-2           # → finds and starts task 5 (3 is taken)

# Agent 1: finish and check for unblocked tasks
tak finish 3
tak list --available                   # tasks blocked by 3 are now available

# After pulling changes from other agents
git pull
tak reindex
tak list --available
```

Note: `tak next` + `tak start` is subject to TOCTOU races — another agent can claim the same task between the two commands. Prefer `tak claim` for concurrent workflows.

## Claude Code Integration

Tak ships as a Claude Code plugin. Enable it to get:

- **Task management skill**: Full CLI reference for creating, querying, and updating tasks
- **Epic planning skill**: Guided decomposition of features into task hierarchies
- **Task execution skill**: Agent workflow for claiming, executing, and completing tasks
- **Session lifecycle hooks**: Auto-reindex + auto-join mesh on session start, auto-leave mesh on stop

```bash
# Run from your git repo root
# Install SessionStart + Stop hooks into .claude/settings.local.json
tak setup

# Also write plugin files under .claude/plugins/tak
tak setup --plugin
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
