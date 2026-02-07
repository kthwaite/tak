# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # Build debug
cargo build --release          # Build release
cargo test                     # Run all 41 tests (27 unit + 14 integration)
cargo test model::tests        # Run unit tests in a specific module
cargo test integration         # Run only integration tests (tests/integration.rs)
cargo test test_name           # Run a single test by name
cargo clippy                   # Lint
cargo fmt --check              # Check formatting
```

No external SQLite needed — rusqlite bundles it via the `bundled` feature.

## Architecture

### Hybrid Storage Model

Tasks are JSON files in `.tak/tasks/` (the git-committed source of truth). A gitignored SQLite index (`index.db`) is derived from these files for fast queries. The index auto-rebuilds when missing (e.g., fresh clone) via `Repo::open()`.

### Two Edge Types

- **parent-child** — structural hierarchy (epics contain tasks)
- **depends-on** — scheduling graph (task B blocks on task A)

"Blocked" status is never stored; it's derived at query time from the dependency graph.

### Source Layout

- **`src/model.rs`** — `Task`, `Status` (pending/in_progress/done/cancelled), `Kind` (epic/task/bug)
- **`src/error.rs`** — `TakError` enum via thiserror; `Result<T>` alias used everywhere
- **`src/output.rs`** — `Format` enum (Json/Pretty/Minimal); `print_task(s)` functions
- **`src/store/files.rs`** — `FileStore`: CRUD on `.tak/tasks/*.json`, atomic ID allocation via counter.json + fs2 lock
- **`src/store/index.rs`** — `Index`: SQLite with WAL mode, FK-enabled. Cycle detection via recursive CTEs. Two-pass rebuild to handle forward references.
- **`src/store/repo.rs`** — `Repo`: wraps FileStore + Index. Walks up from CWD to find `.tak/`. Auto-rebuilds index on open if missing or stale (file fingerprint mismatch).
- **`src/commands/`** — One file per command group. All take `&Path` (repo root) and return `Result<()>`.
- **`src/main.rs`** — Clap derive CLI with 19 subcommands and global `--format`/`--pretty` flags. Uses `ValueEnum` for `Format`, `Kind`, `Status`; `conflicts_with` for `--available`/`--blocked`.

### CLI Commands

19 subcommands. `--format json` (default), `--format pretty`, `--format minimal`.

| Command | Purpose |
|---------|---------|
| `init` | Initialize `.tak/` directory |
| `create TITLE` | Create task (`--kind`, `--parent`, `--depends-on`, `-d`, `--tag`) |
| `delete ID` | Delete a task (`--force` to cascade: orphans children, removes deps) |
| `show ID` | Display a single task |
| `list` | Query tasks (`--status`, `--kind`, `--tag`, `--assignee`, `--available`, `--blocked`, `--children-of`) |
| `edit ID` | Update fields (`--title`, `-d`, `--kind`, `--tag`) |
| `start ID` | Pending → in_progress (`--assignee`) |
| `finish ID` | In_progress → done |
| `cancel ID` | Pending/in_progress → cancelled |
| `claim` | Atomic next+start with file lock (`--assignee`, `--tag`) |
| `reopen ID` | Done/cancelled → pending (clears assignee) |
| `unassign ID` | Clear assignee without changing status |
| `depend ID --on IDS` | Add dependency edges (comma-separated) |
| `undepend ID --on IDS` | Remove dependency edges |
| `reparent ID --to ID` | Change parent |
| `orphan ID` | Remove parent |
| `tree [ID]` | Display parent-child hierarchy |
| `next` | Show next available task (`--assignee`) |
| `reindex` | Rebuild SQLite index from files |

Errors are structured JSON on stderr when `--format json`: `{"error":"<code>","message":"<text>"}`.

### Key Patterns

- Status transitions are validated by a hard state machine in `lifecycle.rs`; `start` also rejects blocked tasks
- Cycle detection (both dependency and parent) uses SQL `WITH RECURSIVE` CTEs — check before adding edges
- `FileStore::create()` validates parent/dependency existence before writing
- `Task::normalize()` trims whitespace, drops empty tags, then deduplicates and sorts `depends_on`/`tags` before every file write
- Mutation commands use validate-then-commit: all validation before any file/index write
- `claim` serializes concurrent access via an exclusive file lock (`claim.lock`); lock acquisition retries with exponential backoff
- `Index::upsert()` is transactional (delete old deps/tags, insert new); uses `INSERT OR IGNORE` for resilience against duplicates
- `Index::rebuild()` uses two-pass insertion: first insert tasks without parent_id, then update parent_id (handles forward references and FK constraints); uses `INSERT OR IGNORE` for deps/tags
- `delete` validates referential integrity (children + dependents); `--force` cascades (orphans children, removes incoming deps)
- Sequential integer IDs via `counter.json` with OS-level file locking (fs2); lock file kept permanently
- Stale index detection via file fingerprint: `Repo::open()` compares task ID + size + nanosecond mtime against stored metadata, auto-rebuilds on mismatch
- Tree command pre-loads all tasks into a HashMap — no per-node file I/O or SQL queries

### On-Disk Layout

```
.tak/
  config.json          # {"version": 1}
  counter.json         # {"next_id": N}  (fs2-locked during allocation)
  counter.lock         # Persistent lock file for ID allocation (gitignored)
  claim.lock           # Persistent lock file for atomic claim (gitignored)
  tasks/*.json         # One file per task (committed to git)
  index.db             # SQLite index (gitignored, rebuilt on demand)
```

## Claude Code Plugin

Three skills in `skills/`: **task-management** (CLI reference), **epic-planning** (structured decomposition), **task-execution** (agent claim-work-finish loop). A `SessionStart` hook in `hooks/` auto-runs `tak reindex` to refresh the index after git operations.
