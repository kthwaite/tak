# Crate split inventory: current module coupling and command entry points

Date: 2026-02-11  
Status: Inventory snapshot for task `8d63d042647bc914`

## 1) Current package topology

Tak is currently a **single Cargo package** (`name = "tak"`) that builds:
- a library (`src/lib.rs`), and
- the CLI binary (`src/main.rs`).

Everything is compiled under one dependency graph (`clap`, `ratatui`, `git2`, `rusqlite`, etc.), so CLI-only and core logic are not yet isolated.

## 2) Source distribution snapshot

Approximate Rust LOC by area:

- `src/main.rs`: **2799**
- `src/commands/*`: **17896**
- `src/store/*`: **7243**
- `src/metrics/*`: **2261**
- `src/model.rs`: **876**
- `src/output.rs`: **799**
- `src/task_id.rs`: **493**
- other support modules (`error`, `git`, `agent`, `json_ids`, etc.): remainder

Interpretation: the current architecture is command-centric, with a very large dispatch/parser layer in `main.rs` and large command modules that mix execution + rendering.

## 3) Module ownership inventory (current state)

### CLI/process layer
- `src/main.rs`
  - Clap parser + subcommand schema
  - task-id arg resolution helpers
  - dispatch routing to `commands::*`
  - top-level error->stderr rendering and process exit semantics

### Application/use-case layer
- `src/commands/*`
  - one module per command group
  - each module opens repo/store, performs validation + mutation/query, and usually prints output directly
  - heavy modules include `tui.rs`, `work.rs`, `import.rs`, `mesh.rs`, `setup.rs`

### Domain/data layer
- `src/model.rs`, `src/task_id.rs`, `src/error.rs`
  - task model, enums, planning/contract/execution types
  - canonical ID parsing + generation
  - rich error taxonomy (`TakError`)

### Persistence/runtime layer
- `src/store/*`
  - file store, index, sidecars, learnings, coordination db, migration, locks, work state
  - repo root discovery/open/reindex integration (`store/repo.rs`)

### Reporting/analytics
- `src/metrics/*`
  - derivation + aggregation + metrics TUI runtime

### Rendering/output
- `src/output.rs`
  - JSON/pretty/minimal rendering for tasks and task lists

### Misc infra
- `src/git.rs`, `src/agent.rs`, `src/build_info.rs`, `src/json_ids.rs`

## 4) Command entrypoint flow (current)

### 4.1 Pre-repo dispatch path
`main::run()` handles these before repository discovery:
- `init`
- `setup`
- `doctor`

### 4.2 Repo-backed dispatch path
For all other commands, `main::run()` currently does:
1. `find_repo_root()`
2. parse/resolve CLI task IDs (canonical/prefix/legacy)
3. apply some coordination verbosity wrapping logic
4. call a concrete `tak::commands::<module>::<fn>(..., format)`

### 4.3 Rendering and exit behavior
- Most `commands::*` functions perform rendering internally based on `output::Format`.
- `main()` only handles the outermost error rendering and exit code.

## 5) Coupling/leak hotspots relevant to crate split

### 5.1 Clap traits currently in non-CLI modules
`clap::ValueEnum` is derived in multiple core-facing modules, not only `main.rs`, including:
- `src/model.rs`
- `src/metrics/model.rs`
- `src/output.rs` (`Format`)
- `src/store/work.rs`
- `src/store/coordination_db.rs` (`BlackboardStatus`)
- `src/store/therapist.rs`
- `src/commands/tree.rs` (`TreeSort`)
- `src/commands/tui.rs` (`TuiSection`)
- `src/commands/blackboard.rs` (`BlackboardTemplate`)

This is the largest CLI coupling seam if we want `tak-core` to avoid clap-facing derives.

### 5.2 Execution + presentation are intertwined
Most command handlers take `format: Format` and print directly. This means:
- command behavior and output policy are coupled,
- there is no thin app-service return type boundary yet.

### 5.3 `main.rs` performs app policy beyond parsing
`main.rs` currently contains policy logic such as:
- task-id resolution helpers,
- verbosity label/tag application,
- per-command orchestration details.

This logic can migrate behind a core command facade to make `tak-cli` thinner.

### 5.4 Dependency graph is shared across CLI/core concerns
Current single-package deps include CLI-centric libraries (`clap`, `ratatui`, `crossterm`, `colored`) and functional libs (`rusqlite`, `git2`, `serde`, `thiserror`) together.

## 6) Extraction implications (for follow-up tasks)

Aligned to kickoff decisions:
- output policy stays in core,
- CLI-centric features stay in CLI,
- functional behavior moves to core.

Practical implication for the next step:
1. define a `tak-core` command facade that encapsulates repo/root/id resolution + command execution,
2. keep `tak-cli` focused on clap parsing + top-level process IO/exit mapping,
3. progressively remove clap derives from core-owned types (or isolate them behind adapter types in CLI) where needed.

This document is an inventory baseline for API-contract drafting (`828814dbd2d24181`).
