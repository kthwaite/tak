# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # Build debug
cargo build --release          # Build release
cargo test                     # Run all ~410 tests (unit + integration)
cargo test model::tests        # Run unit tests in a specific module
cargo test --test integration  # Run the main integration test binary
cargo test --test work_done_integration  # Run a specific integration test binary
cargo test test_name           # Run a single test by name
cargo clippy                   # Lint
cargo fmt --check              # Check formatting
```

No external SQLite needed — rusqlite bundles it via the `bundled` feature.

## Isolated verification fallback

Default practice: run verification in the shared working tree.

When unrelated in-progress edits make shared-tree verification noisy, use a temporary detached `git worktree` for targeted verification and then remove it. Record the reason + commands in blackboard/task updates so evidence provenance is clear.

See `docs/how/isolated-verification.md` for the playbook and guardrails.

## Implementation-cycle learnings closeout (required)

Before ending a cycle (`finish`, `handoff`, or `cancel`), perform learnings closeout:

1. Capture reusable insight with `tak learn add ... --task <task-id>` (or update an existing learning with `tak learn edit ... --add-task <task-id>`).
2. Commit `.tak/learnings/*.json` changes (plus any linked task-file updates) in the same implementation cycle.
3. Do not defer learning commits to a later unrelated commit.

## Architecture

### Hybrid Storage Model

Tasks are JSON files in `.tak/tasks/` (the git-committed source of truth). A gitignored SQLite index (`index.db`) is derived from these files for fast queries. The index auto-rebuilds when missing (e.g., fresh clone) via `Repo::open()`.

### Two Edge Types

- **parent-child** — structural hierarchy (epics contain tasks)
- **depends-on** — scheduling graph (task B blocks on task A)

"Blocked" status is never stored; it's derived at query time from the dependency graph.

### Task ID Semantics

- Canonical task IDs are 16-character lowercase hex strings (`TaskId`, e.g. `000000000000002a`).
- CLI task-id arguments accept canonical hex, unique hex prefixes (case-insensitive), and legacy decimal IDs.
- Resolution order is exact match first (hex or legacy decimal), then unique prefix match.
- Exact legacy decimal matches win over prefix interpretation for digit-only input.

### Source Layout

- **`src/model.rs`** — `Task`, `Status` (pending/in_progress/done/cancelled), `Kind` (epic/feature/task/bug/meta), `Dependency` (id/dep_type/reason), `DepType` (hard/soft), `Contract` (objective/acceptance_criteria/verification/constraints), `Planning` (priority/estimate/required_skills/risk), `Execution` (attempt_count/last_error/handoff_summary/blocked_reason), `Priority` (critical/high/medium/low), `Estimate` (xs/s/m/l/xl), `Risk` (low/medium/high), `GitInfo` (branch/start_commit/end_commit/commits/pr), `Learning` (id/title/description/category/tags/task_ids/timestamps), `LearningCategory` (insight/pitfall/pattern/tool/process)
- **`src/git.rs`** — `current_head_info()` returns branch + SHA; `commits_since()` returns one-line summaries between two SHAs via git2 revwalk
- **`src/error.rs`** — `TakError` enum via thiserror; `Result<T>` alias used everywhere
- **`src/output.rs`** — `Format` enum (Json/Pretty/Minimal); `print_task(s)` functions
- **`src/metrics/model.rs`** — metrics query/filter domain model (`MetricsBucket`, `MetricsQuery`, `CompletionMetric`) and report payload schemas (`BurndownReport`, `CompletionTimeReport`)
- **`src/metrics/derive.rs`** — normalized lifecycle timeline derivation from task history sidecars, including completion episodes and data-quality accounting
- **`src/metrics/burndown.rs`** — burndown aggregation logic (remaining series, ideal line, scope added/removed overlays)
- **`src/metrics/aggregate.rs`** — completion-time aggregation (lead/cycle samples, bucket rollups, avg/p50/p90 summary)
- **`src/metrics/tui.rs`** — terminal dashboard runtime (`tak metrics tui`) with refresh loop, key controls, and panel rendering helpers
- **`src/agent.rs`** — `resolve_agent()` reads `TAK_AGENT` env var; auto-generated adjective-animal names when identity is unset
- **`src/build_info.rs`** — build-time git SHA stamp via `TAK_BUILD_GIT_SHA` (set by `build.rs`)
- **`src/json_ids.rs`** — canonical task-ID rendering/normalization helpers for JSON output (format, parse, rewrite)
- **`src/task_id.rs`** — `TaskId` wrapper (canonical 16-hex IDs), CSPRNG generation API (`TaskId::generate`) with deterministic test hook (`generate_with`), CLI parsing for canonical/prefix/legacy numeric input, serde + rusqlite compatibility helpers
- **`src/store/lock.rs`** — fs2-based exclusive file locking with exponential backoff (1ms→512ms); used by claim, task-ID allocation, learning counter
- **`src/store/files.rs`** — `FileStore`: CRUD on `.tak/tasks/*.json`; allocates random task IDs via CSPRNG with bounded collision retry under `task-id.lock`; writes canonical hash-style filenames while reading legacy numeric filenames for compatibility
- **`src/store/index.rs`** — `Index`: SQLite with WAL mode, FK-enabled. Cycle detection via recursive CTEs. Two-pass rebuild to handle forward references. FTS5 full-text search for learnings.
- **`src/store/learnings.rs`** — `LearningStore`: CRUD on `.tak/learnings/*.json`, atomic ID allocation via counter.json + fs2 lock; separate from task ID sequence
- **`src/store/sidecars.rs`** — `SidecarStore`: manages per-task context notes (`.tak/context/{task_id}.md`), structured history logs (`.tak/history/{task_id}.jsonl`), verification results (`.tak/verification_results/{task_id}.json`), and artifact directories (`.tak/artifacts/{task_id}/`); supports legacy numeric path migration; defines `HistoryEvent` (timestamp/event/agent/detail), `VerificationResult` (timestamp/results/passed), `CommandResult` (command/exit_code/stdout/stderr/passed)
- **`src/store/coordination_db.rs`** — `CoordinationDb`: SQLite runtime backing mesh + blackboard state under `.tak/runtime/coordination.db` (agents, messages, reservations, notes, feed events) with WAL + FK enforcement.
- **`src/store/coordination.rs`** — `CoordinationLinks` struct for cross-channel linkage (mesh message IDs, blackboard note refs, history event IDs); `derive_links_from_text()` extracts references from free-form text.
- **`src/store/migration.rs`** — atomic task-file rewriting for ID migration: reads tasks, remaps IDs, writes to staging dir, then swaps via rename (with rollback on failure).
- **`src/store/paths.rs`** — path normalization (relative to repo root), traversal-escape validation, and prefix-based conflict detection for reservations.
- **`src/store/therapist.rs`** — `TherapistStore`: append-only workflow observations under `.tak/therapist/observations.jsonl` (`offline`/`online` diagnosis artifacts).
- **`src/store/work.rs`** — `WorkStore`: per-agent CLI work-loop state under `.tak/runtime/work/` with lock-safe activate/status/deactivate/save flows plus strategy/verbosity metadata.
- **`src/store/repo.rs`** — `Repo`: wraps FileStore + Index + SidecarStore + LearningStore. Walks up from CWD to find `.tak/`. Auto-rebuilds index on open if missing or stale (file fingerprint mismatch). Also auto-rebuilds learnings index via separate fingerprint.
- **`src/commands/`** — One file per command group. Most take `&Path` (repo root) and return `Result<()>`. `doctor` doesn't require a repo; `setup` supports global mode anywhere but project-scoped setup requires a git repo root.
- **`src/commands/work.rs`** — CLI-native work-loop handlers (`start/resume`, `status`, `done`, `stop`) with reconciliation events (`continued`/`attached`/`claimed`/`done`/`no_work`/`limit_reached`) and format-specific output rendering.
- **`src/commands/takeover.rs`** — stale-owner reassignment command with inactivity/force guardrails, structured decision output, and lifecycle history logging.
- **`src/commands/import.rs`** — YAML/JSON task-plan import pipeline (`tak import`) with dry-run previews, alias/reference resolution, graph validation, and deterministic create ordering.
- **`src/commands/wait.rs`** — deterministic wait helpers (`tak wait`) for reservation-path and dependency-unblock readiness with timeout diagnostics.
- **`src/commands/metrics.rs`** — metrics handlers for `burndown`, `completion-time`, and `tui` including shared query validation and format-specific renderers.
- **`src/commands/tui.rs`** — top-level interactive explorer (`tak tui`) for tasks/learnings/blackboard/mesh/feed with searchable list + deep detail panes (including task sidecars).
- **`src/commands/mesh.rs`** — 13 mesh subcommand handlers: join, leave, list, send, broadcast, inbox, heartbeat, cleanup, blockers, reservations, reserve, release, feed
- **`src/commands/blackboard.rs`** — 5 blackboard subcommand handlers: post, list, show, close, reopen
- **`src/commands/therapist.rs`** — therapist handlers: offline diagnosis, online RPC interview, and observation log listing
- **`src/commands/migrate_ids.rs`** — task ID migration workflow: preflight, apply rewrite, optional `--rekey-random` remapping to fresh random IDs, config-version bump, and audit-map generation
- **`src/main.rs`** — Clap derive CLI with global `--format`/`--pretty` flags. Uses `ValueEnum` for `Format`, `Kind`, `Status`; `conflicts_with` for `--available`/`--blocked`; resolves task-id args through canonical/prefix/legacy-compatible parsing before command execution.

### CLI Commands

Top-level commands plus grouped subcommands. `--format json` (default), `--format pretty`, `--format minimal`.

For task-taking commands, `TASK_ID` accepts canonical 16-hex IDs, unique hex prefixes, and legacy decimal IDs.

| Command | Purpose |
|---------|---------|
| `init` | Initialize `.tak/` directory (`tasks/`, sidecars, `learnings/`, `therapist/`, runtime `coordination.db`, work-state dirs, `.gitignore`) |
| `create TITLE` | Create task (`--kind`, `--parent`, `--depends-on`, `-d`, `--tag`, `--objective`, `--verify`, `--constraint`, `--criterion`, `--priority`, `--estimate`, `--skill`, `--risk`) |
| `import SOURCE` | Import YAML/JSON task plans (`--dry-run` validates/prints plan without writing) |
| `delete TASK_ID` | Delete a task (`--force` to cascade: orphans children, removes deps); also cleans up sidecar files |
| `show TASK_ID` | Display a single task |
| `list` | Query tasks (`--status`, `--kind`, `--tag`, `--assignee`, `--available`, `--blocked`, `--children-of`, `--priority`) |
| `edit TASK_ID` | Update fields (`--title`, `-d`, `--kind`, `--tag`, `--objective`, `--verify`, `--constraint`, `--criterion`, `--priority`, `--estimate`, `--skill`, `--risk`, `--pr`) |
| `start TASK_ID` | Pending -> in_progress (`--assignee`); auto-captures git branch + HEAD SHA; appends history log |
| `finish TASK_ID` | In_progress -> done; auto-captures end commit SHA + commit range; appends history log (epic close hygiene gate enforced for tak source changes) |
| `cancel TASK_ID` | Pending/in_progress -> cancelled (`--reason`); appends history log |
| `handoff TASK_ID` | In_progress -> pending, record summary (`--summary`, required); appends history log |
| `claim` | Atomic next+start with file lock (`--assignee`, `--tag`); appends history log |
| `takeover TASK_ID` | Reassign stale in-progress ownership with guardrails (`--assignee`, `--inactive-secs`, `--force`); appends history log |
| `work [start\|status\|done\|stop]` | CLI-native work-loop controller (`start/resume`, `status`, `done`, `stop`) with persisted per-agent loop state |
| `reopen TASK_ID` | Done/cancelled -> pending (clears assignee); appends history log |
| `unassign TASK_ID` | Clear assignee without changing status; appends history log |
| `depend TASK_IDS --on TASK_IDS` | Add dependency edges in bulk (`--dep-type hard\|soft`, `--reason`, `--quiet`) |
| `undepend TASK_IDS --on TASK_IDS` | Remove dependency edges in bulk (`--quiet`) |
| `reparent TASK_IDS --to TASK_ID` | Change parent in bulk (`--quiet`; comma-separated IDs) |
| `orphan TASK_ID` | Remove parent (`--quiet`) |
| `tree [TASK_ID]` | Display parent-child hierarchy (`--pending`, `--sort id\|created\|priority\|estimate`) |
| `next` | Show next available task (`--assignee`) |
| `wait` | Block until a path reservation clears or a task unblocks (`--path`/`--on-task`, `--timeout`) |
| `context TASK_ID` | Read/write context notes (`--set TEXT`, `--clear`) |
| `log TASK_ID` | Display structured JSONL task history log |
| `verify TASK_ID` | Run contract verification commands; stores result; exits 1 if any fail |
| `metrics <burndown\|completion-time\|tui>` | Metrics trends + TUI dashboard (`--from`, `--to`, `--bucket day\|week`, filters; completion-time `--metric lead\|cycle`; `--include-cancelled` for burndown/TUI) |
| `tui` | Interactive cross-domain explorer for tasks/learnings/blackboard/mesh/feed (`--focus`, `--query`) |
| `learn add TITLE` | Record a learning (`--category`, `-d`, `--tag`, `--task`) |
| `learn list` | List learnings (`--category`, `--tag`, `--task`) |
| `learn show ID` | Display a single learning |
| `learn edit ID` | Update learning fields (`--title`, `-d`, `--category`, `--tag`, `--add-task`, `--remove-task`) |
| `learn remove ID` | Delete a learning (unlinks from tasks) |
| `learn suggest TASK_ID` | Suggest relevant learnings via FTS5 search on task title |
| `mesh join` | Register agent in coordination mesh (`--name` optional, `--session-id`) |
| `mesh leave` | Unregister from mesh (`--name` optional) |
| `mesh list` | List registered mesh agents |
| `mesh send` | Send direct message (`--from`, `--to`, `--message`) |
| `mesh broadcast` | Broadcast message to all agents (`--from`, `--message`) |
| `mesh inbox` | Read inbox messages (`--name`, `--ack`, `--ack-id`, `--ack-before`) |
| `mesh heartbeat` | Refresh registration/reservation lease liveness (`--name`, `--session-id`) |
| `mesh cleanup` | Remove stale mesh registrations/reservations (`--stale`, `--dry-run`, `--ttl-seconds`) |
| `mesh blockers` | Diagnose reservation blockers (`--path` repeatable) |
| `mesh reservations` | List active reservation snapshots (`--name`, `--path`) |
| `mesh reserve` | Reserve file paths (`--name`, `--path`, `--reason`) |
| `mesh release` | Release reservations (`--name`, `--path`/`--all`) |
| `mesh feed` | Show activity feed (`--limit`) |
| `blackboard post` | Post a shared coordination note (`--from`, `--message`, `--template`, `--since-note`, `--no-change-since`, `--tag`, `--task`) |
| `blackboard list` | List notes (`--status`, `--tag`, `--task`, `--limit`) |
| `blackboard show` | Show one note by ID |
| `blackboard close` | Close note (`--by`, `--reason`) |
| `blackboard reopen` | Re-open note (`--by`) |
| `therapist offline` | Diagnose workflow friction from mesh + blackboard and append an observation (`--by`, `--limit`) |
| `therapist online` | Resume a pi session via RPC for interview-style workflow diagnosis (`--session`, `--session-dir`, `--by`) |
| `therapist log` | Read therapist observation log (`--limit`) |
| `migrate-ids` | Preflight/apply task-ID migration (`--dry-run`, `--apply`, `--rekey-random`, `--force`); writes audit map + bumps config version |
| `reindex` | Rebuild SQLite index from files |
| `setup` | Install agent integrations (`--global`, `--check`, `--remove`, `--plugin`, `--skills`, `--pi`) |
| `doctor` | Validate installation health (`--fix`) |

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
- `delete` validates referential integrity (children + dependents); `--force` cascades (orphans children, removes incoming deps); cleans up sidecar files
- `reparent` supports bulk targets (`TASK_IDS --to TASK_ID`) and uses validate-then-commit semantics across the whole batch (no partial writes on invalid IDs or cycle checks).
- Task IDs are allocated via OS-backed CSPRNG (`TaskId::generate`) and stored as canonical 16-hex filenames; FileStore retries collisions up to a bounded attempt limit under `task-id.lock`
- Task-id CLI resolution is exact-first (canonical hex or legacy decimal), then unique hex-prefix fallback (case-insensitive)
- Stale index detection via file fingerprint: `Repo::open()` compares task filename + size + nanosecond mtime against stored metadata, auto-rebuilds on mismatch
- Tree command pre-loads all tasks into a HashMap — no per-node file I/O or SQL queries
- `setup` and `doctor` are dispatched before `find_repo_root()`; project-scoped `setup` validates that CWD is a git repo root
- `setup` embeds Claude plugin/skills + pi integration assets via `include_str!` at compile time; idempotent install/remove
- `doctor` runs grouped health checks (Core/Index/Data Integrity/Environment) with auto-fix support
- `Task` uses `#[serde(flatten)]` extensions map for forward-compatible JSON round-trips (unknown fields survive read→write)
- `depends_on: Vec<Dependency>` — each dep has `id`, optional `dep_type` (hard/soft), optional `reason`; `depend` updates metadata on existing deps
- `dependencies` SQLite table carries `dep_type TEXT` and `reason TEXT` columns
- `Task.contract: Contract` — optional executable spec with `objective`, `acceptance_criteria`, `verification` commands, `constraints`; omitted from JSON when empty
- `Task.planning: Planning` — optional triage metadata with `priority` (critical/high/medium/low), `estimate` (xs-xl), `risk` (low/medium/high), `required_skills`; omitted from JSON when empty
- Priority-ordered claiming: `available()`, `next`, and `claim` sort by `COALESCE(priority_rank, 4), id` — critical first, unprioritized last
- SQLite `tasks` table carries `priority INTEGER` (rank 0-3), `estimate TEXT`, and `attempt_count INTEGER` columns; `skills` junction table for required_skills
- `Task.git: GitInfo` — auto-populated provenance: `branch` + `start_commit` on `start`, `end_commit` + `commits` on `finish`, `pr` via `edit --pr`; omitted from JSON when empty
- `start` captures git info only on first start (idempotent on restart after reopen); `finish` collects commit summaries via `git::commits_since()` revwalk
- `finish` applies an epic close hygiene gate in the tak source repo when functionality changed: requires docs updates, binary-at-HEAD, and synced project `.pi` integration
- `src/git.rs` uses git2 to discover the repo, read HEAD, and walk revisions; all functions degrade gracefully outside a git repo
- `Task.execution: Execution` — runtime metadata with `attempt_count` (incremented on start/claim), `last_error` (set by cancel --reason), `handoff_summary` (set by handoff), `blocked_reason` (human context); omitted from JSON when empty
- `start` and `claim` both increment `execution.attempt_count` to track retry attempts
- `handoff` transitions in_progress -> pending, clears assignee, records `execution.handoff_summary`
- `cancel --reason` stores the reason in `execution.last_error`
- Sidecar files: `SidecarStore` manages per-task `context/{task_id}.md` (free-form notes, git-committed), `history/{task_id}.jsonl` (structured JSONL event log, git-committed), `verification_results/{task_id}.json` (gitignored), and `artifacts/{task_id}/` (gitignored)
- `migrate-ids` runs a preflight gate by default (dry-run), blocks when in-progress tasks exist unless `--force`, and on apply rewrites filenames + sidecars, bumps config version, and writes `.tak/migrations/task-id-map-*.json`; `--rekey-random` remaps canonical task IDs to fresh random IDs while preserving references
- Lifecycle commands (start, finish, cancel, handoff, reopen, unassign, claim) auto-append structured `HistoryEvent` entries to JSONL history logs; failures are best-effort (never fail the main command)
- `verify` command stores `VerificationResult` to `.tak/verification_results/{task_id}.json` after running contract verification commands
- `context` command reads/writes free-form context notes; `--set` overwrites, `--clear` deletes
- `log` command displays history log; JSON mode returns array of lines, pretty mode prints raw
- `verify` command runs `contract.verification` commands via `sh -c` from repo root; reports pass/fail per command; exits 1 if any fail
- Metrics queries default to a 30-day window (`--to` defaults today, `--from` defaults 30 days prior), reject inverted windows, and currently reject `metrics completion-time --include-cancelled`.
- `delete` cleans up all sidecar files (context + history + verification results + artifacts) after removing the task file and index entry
- `Learning` struct: id/title/description/category/tags/task_ids/timestamps; stored as `.tak/learnings/{id}.json` with separate counter.json ID sequence
- `LearningCategory` enum: insight/pitfall/pattern/tool/process (default: insight)
- `Task.learnings: Vec<u64>` — bidirectional link; `learn add --task` and `learn edit --add-task/--remove-task` maintain both sides
- SQLite `learnings` table + `learning_tags`/`learning_tasks` junction tables; `learnings_fts` FTS5 virtual table (content-synced via `content=learnings, content_rowid=numeric_id`)
- FTS5 content-sync requires reading old data before delete, then inserting delete command with actual values; `upsert_learning` handles this
- `suggest_learnings` sanitizes task title to alphanumeric tokens, joins with OR for FTS5 MATCH; returns results by rank
- Learning index has separate fingerprint (`learning_fingerprint` in metadata table); auto-rebuilt by `Repo::open()` when stale
- `learn remove` unlinks learning from all referenced tasks before deleting
- Workflow expectation: each implementation cycle closes with a learnings pass, and any `.tak/learnings/*.json` updates are committed in that same cycle.
- Coordination runtime state is centralized in `.tak/runtime/coordination.db` (`CoordinationDb`, WAL-backed) for mesh + blackboard domains.
- Mesh registrations carry lease metadata (`session_id`, `cwd`, timestamps); `mesh heartbeat` refreshes liveness and `mesh cleanup --stale` prunes expired rows.
- Reservation conflict is prefix-based on normalized paths: `src/store/` conflicts with `src/store/coordination_db.rs`.
- `mesh blockers` and `mesh reservations` expose blocker/snapshot diagnostics; feed append failures are best-effort (never break primary operations).
- Blackboard notes (status, tags, task links, open/closed lifecycle) are stored in coordination DB tables; runtime state remains gitignored.
- `TherapistStore` appends JSONL observations to `.tak/therapist/observations.jsonl`; `offline` analyzes mesh/blackboard signals, `online` resumes pi RPC sessions for interviews.

### On-Disk Layout

```
.tak/
  .gitignore                     # Ignores index.db, *.lock, artifacts/, verification_results/, runtime/
  config.json                    # {"version": 2} (migrate-ids --apply bumps to 3)
  task-id.lock                   # Persistent lock file for random task ID allocation (gitignored)
  claim.lock                     # Persistent lock file for atomic claim (gitignored)
  tasks/*.json                   # One file per task (canonical 16-char lowercase hex filenames)
  context/{task_id}.md           # Per-task context notes (committed to git)
  history/{task_id}.jsonl        # Per-task structured JSONL history logs (committed to git)
  verification_results/{task_id}.json # Per-task verification results (gitignored)
  artifacts/{task_id}/           # Per-task artifact directories (gitignored)
  learnings/*.json               # One file per learning (committed to git)
  learnings/counter.json         # {"next_id": N}  (fs2-locked during allocation)
  learning_counter.lock          # Persistent lock file for learning ID allocation (gitignored)
  migrations/
    task-id-map-*.json           # Audit map written by migrate-ids --apply
  runtime/coordination.db        # Mesh + blackboard runtime DB (gitignored)
  runtime/coordination.db-wal    # SQLite WAL sidecar (gitignored)
  runtime/coordination.db-shm    # SQLite shared-memory sidecar (gitignored)
  runtime/work/states/*.json     # Per-agent work-loop state (gitignored)
  therapist/                     # Workflow therapist observations (committed)
    observations.jsonl           # Append-only JSONL observations
  index.db                       # SQLite index (gitignored, rebuilt on demand)
```

## Claude Code Plugin

Three embedded Claude skills under `claude-plugin/skills/`: **task-management** (CLI + coordination reference), **epic-planning** (structured decomposition), and **task-execution** (CLI-first `tak work` / `tak work status` / `tak work done` / `tak work stop` flow, with `/tak work*` prompts as shorthand).

Lifecycle hooks remain lightweight and session-scoped: `tak reindex` + `tak mesh join` on `SessionStart`, and `tak mesh leave` on `Stop`.

`tak setup` installs hooks into Claude Code settings (project-local or global). Project-scoped setup must run from a git repo root. `tak setup --plugin` writes Claude plugin files to `.claude/plugins/tak`. `tak setup --skills` installs only the Claude skill files under `.claude/skills/` (or `~/.claude/skills/` with `--global`). `tak setup --pi` installs pi integration files (`extensions/tak.ts`, `skills/tak-coordination/SKILL.md`, and a managed `APPEND_SYSTEM.md` block) under `.pi/` (or `~/.pi/agent/` with `--global`). `tak doctor` validates installation health.
