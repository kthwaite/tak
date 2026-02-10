# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # Build debug
cargo build --release          # Build release
cargo test                     # Run all 144 tests (86 unit + 58 integration)
cargo test model::tests        # Run unit tests in a specific module
cargo test integration         # Run only integration tests (tests/integration.rs)
cargo test test_name           # Run a single test by name
cargo clippy                   # Lint
cargo fmt --check              # Check formatting
```

No external SQLite needed — rusqlite bundles it via the `bundled` feature.

## Isolated verification fallback

Default practice: run verification in the shared working tree.

When unrelated in-progress edits make shared-tree verification noisy, use a temporary detached `git worktree` for targeted verification and then remove it. Record the reason + commands in blackboard/task updates so evidence provenance is clear.

See `docs/how/isolated-verification.md` for the playbook and guardrails.

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
- **`src/task_id.rs`** — `TaskId` wrapper (canonical 16-hex IDs), CSPRNG generation API (`TaskId::generate`) with deterministic test hook (`generate_with`), CLI parsing for canonical/prefix/legacy numeric input, serde + rusqlite compatibility helpers
- **`src/store/files.rs`** — `FileStore`: CRUD on `.tak/tasks/*.json`; allocates random task IDs via CSPRNG with bounded collision retry under `task-id.lock`; writes canonical hash-style filenames while reading legacy numeric filenames for compatibility
- **`src/store/index.rs`** — `Index`: SQLite with WAL mode, FK-enabled. Cycle detection via recursive CTEs. Two-pass rebuild to handle forward references. FTS5 full-text search for learnings.
- **`src/store/learnings.rs`** — `LearningStore`: CRUD on `.tak/learnings/*.json`, atomic ID allocation via counter.json + fs2 lock; separate from task ID sequence
- **`src/store/sidecars.rs`** — `SidecarStore`: manages per-task context notes (`.tak/context/{task_id}.md`), structured history logs (`.tak/history/{task_id}.jsonl`), verification results (`.tak/verification_results/{task_id}.json`), and artifact directories (`.tak/artifacts/{task_id}/`); supports legacy numeric path migration; defines `HistoryEvent` (timestamp/event/agent/detail), `VerificationResult` (timestamp/results/passed), `CommandResult` (command/exit_code/stdout/stderr/passed)
- **`src/store/mesh.rs`** — `MeshStore`: manages `.tak/runtime/mesh/` — agent registry (join/leave/list), messaging (send/broadcast/inbox), file reservations (reserve/release), activity feed (append/read). Per-domain file locks via `lock.rs`. Auto-generates agent names when omitted.
- **`src/store/blackboard.rs`** — `BlackboardStore`: shared coordination notes under `.tak/runtime/blackboard/` (`post/list/show/close/reopen`) with tags, task links, and close metadata.
- **`src/store/therapist.rs`** — `TherapistStore`: append-only workflow observations under `.tak/therapist/observations.jsonl` (`offline`/`online` diagnosis artifacts).
- **`src/store/work.rs`** — `WorkStore`: per-agent CLI work-loop state under `.tak/runtime/work/` with lock-safe activate/status/deactivate/save flows plus strategy/verbosity metadata.
- **`src/store/repo.rs`** — `Repo`: wraps FileStore + Index + SidecarStore + LearningStore. Walks up from CWD to find `.tak/`. Auto-rebuilds index on open if missing or stale (file fingerprint mismatch). Also auto-rebuilds learnings index via separate fingerprint.
- **`src/commands/`** — One file per command group. Most take `&Path` (repo root) and return `Result<()>`. `doctor` doesn't require a repo; `setup` supports global mode anywhere but project-scoped setup requires a git repo root.
- **`src/commands/work.rs`** — CLI-native work-loop handlers (`start/resume`, `status`, `done`, `stop`) with reconciliation events (`continued`/`attached`/`claimed`/`done`/`no_work`/`limit_reached`) and format-specific output rendering.
- **`src/commands/mesh.rs`** — 9 mesh subcommand handlers: join, leave, list, send, broadcast, inbox, reserve, release, feed
- **`src/commands/blackboard.rs`** — 5 blackboard subcommand handlers: post, list, show, close, reopen
- **`src/commands/therapist.rs`** — therapist handlers: offline diagnosis, online RPC interview, and observation log listing
- **`src/commands/migrate_ids.rs`** — task ID migration workflow: preflight, apply rewrite, optional `--rekey-random` remapping to fresh random IDs, config-version bump, and audit-map generation
- **`src/main.rs`** — Clap derive CLI with global `--format`/`--pretty` flags. Uses `ValueEnum` for `Format`, `Kind`, `Status`; `conflicts_with` for `--available`/`--blocked`; resolves task-id args through canonical/prefix/legacy-compatible parsing before command execution.

### CLI Commands

Top-level commands plus grouped subcommands. `--format json` (default), `--format pretty`, `--format minimal`.

For task-taking commands, `TASK_ID` accepts canonical 16-hex IDs, unique hex prefixes, and legacy decimal IDs.

| Command | Purpose |
|---------|---------|
| `init` | Initialize `.tak/` directory (including `context/`, `history/`, `artifacts/`, `verification_results/`, `learnings/` dirs + `.gitignore`) |
| `create TITLE` | Create task (`--kind`, `--parent`, `--depends-on`, `-d`, `--tag`, `--objective`, `--verify`, `--constraint`, `--criterion`, `--priority`, `--estimate`, `--skill`, `--risk`) |
| `delete TASK_ID` | Delete a task (`--force` to cascade: orphans children, removes deps); also cleans up sidecar files |
| `show TASK_ID` | Display a single task |
| `list` | Query tasks (`--status`, `--kind`, `--tag`, `--assignee`, `--available`, `--blocked`, `--children-of`, `--priority`) |
| `edit TASK_ID` | Update fields (`--title`, `-d`, `--kind`, `--tag`, `--objective`, `--verify`, `--constraint`, `--criterion`, `--priority`, `--estimate`, `--skill`, `--risk`, `--pr`) |
| `start TASK_ID` | Pending -> in_progress (`--assignee`); auto-captures git branch + HEAD SHA; appends history log |
| `finish TASK_ID` | In_progress -> done; auto-captures end commit SHA + commit range; appends history log (epic close hygiene gate enforced for tak source changes) |
| `cancel TASK_ID` | Pending/in_progress -> cancelled (`--reason`); appends history log |
| `handoff TASK_ID` | In_progress -> pending, record summary (`--summary`, required); appends history log |
| `claim` | Atomic next+start with file lock (`--assignee`, `--tag`); appends history log |
| `work [start\|status\|done\|stop]` | CLI-native work-loop controller (`start/resume`, `status`, `done`, `stop`) with persisted per-agent loop state |
| `reopen TASK_ID` | Done/cancelled -> pending (clears assignee); appends history log |
| `unassign TASK_ID` | Clear assignee without changing status; appends history log |
| `depend TASK_ID --on TASK_IDS` | Add dependency edges (`--dep-type hard\|soft`, `--reason`) |
| `undepend TASK_ID --on TASK_IDS` | Remove dependency edges |
| `reparent TASK_ID --to TASK_ID` | Change parent |
| `orphan TASK_ID` | Remove parent |
| `tree [TASK_ID]` | Display parent-child hierarchy |
| `next` | Show next available task (`--assignee`) |
| `context TASK_ID` | Read/write context notes (`--set TEXT`, `--clear`) |
| `log TASK_ID` | Display structured JSONL task history log |
| `verify TASK_ID` | Run contract verification commands; stores result; exits 1 if any fail |
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
| `mesh inbox` | Read inbox messages (`--name`, `--ack`) |
| `mesh reserve` | Reserve file paths (`--name`, `--path`, `--reason`) |
| `mesh release` | Release reservations (`--name`, `--path`/`--all`) |
| `mesh feed` | Show activity feed (`--limit`) |
| `blackboard post` | Post a shared coordination note (`--from`, `--message`, `--tag`, `--task`) |
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
- `delete` cleans up all sidecar files (context + history + verification results + artifacts) after removing the task file and index entry
- `Learning` struct: id/title/description/category/tags/task_ids/timestamps; stored as `.tak/learnings/{id}.json` with separate counter.json ID sequence
- `LearningCategory` enum: insight/pitfall/pattern/tool/process (default: insight)
- `Task.learnings: Vec<u64>` — bidirectional link; `learn add --task` and `learn edit --add-task/--remove-task` maintain both sides
- SQLite `learnings` table + `learning_tags`/`learning_tasks` junction tables; `learnings_fts` FTS5 virtual table (content-synced via `content=learnings, content_rowid=numeric_id`)
- FTS5 content-sync requires reading old data before delete, then inserting delete command with actual values; `upsert_learning` handles this
- `suggest_learnings` sanitizes task title to alphanumeric tokens, joins with OR for FTS5 MATCH; returns results by rank
- Learning index has separate fingerprint (`learning_fingerprint` in metadata table); auto-rebuilt by `Repo::open()` when stale
- `learn remove` unlinks learning from all referenced tasks before deleting
- `MeshStore`: manages `.tak/runtime/mesh/` — agent registry, messaging, reservations, activity feed
- Mesh uses per-domain file locks (registry, inbox, reservations, feed) via existing `lock.rs`
- Registration stores session metadata (`session_id`, `cwd`, timestamps); names are auto-generated when omitted
- Reservation conflict is prefix-based: `src/store/` conflicts with `src/store/mesh.rs`
- Feed events are best-effort: failures never break primary operations
- All mesh runtime state is gitignored (ephemeral per-session data)
- `BlackboardStore` persists shared notes in `.tak/runtime/blackboard/notes.json` with lock-protected ID allocation and lifecycle transitions (`open`/`closed`)
- `TherapistStore` appends JSONL observations to `.tak/therapist/observations.jsonl`; `offline` analyzes mesh+blackboard signals, `online` resumes pi RPC sessions for interviews

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
  runtime/mesh/                  # Coordination runtime (gitignored)
    registry/*.json              # Per-agent presence records
    inbox/<agent>/*.json         # Queued messages per agent
    reservations.json            # File path reservations
    feed.jsonl                   # Append-only activity timeline
    locks/                       # Per-domain lock files
  runtime/blackboard/            # Shared coordination board (gitignored)
    notes.json                   # Open/closed note records
    counter.json                 # Blackboard note IDs
    locks/                       # Blackboard lock files
  therapist/                     # Workflow therapist observations (committed)
    log.jsonl                    # Append-only JSONL observations
  index.db                       # SQLite index (gitignored, rebuilt on demand)
```

## Claude Code Plugin

Three embedded Claude skills under `claude-plugin/skills/`: **task-management** (CLI + coordination reference), **epic-planning** (structured decomposition), and **task-execution** (CLI-first `tak work` / `tak work status` / `tak work done` / `tak work stop` flow, with `/tak work*` prompts as shorthand).

Lifecycle hooks remain lightweight and session-scoped: `tak reindex` + `tak mesh join` on `SessionStart`, and `tak mesh leave` on `Stop`.

`tak setup` installs hooks into Claude Code settings (project-local or global). Project-scoped setup must run from a git repo root. `tak setup --plugin` writes Claude plugin files to `.claude/plugins/tak`. `tak setup --skills` installs only the Claude skill files under `.claude/skills/` (or `~/.claude/skills/` with `--global`). `tak setup --pi` installs pi integration files (`extensions/tak.ts`, `skills/tak-coordination/SKILL.md`, and a managed `APPEND_SYSTEM.md` block) under `.pi/` (or `~/.pi/agent/` with `--global`). `tak doctor` validates installation health.
