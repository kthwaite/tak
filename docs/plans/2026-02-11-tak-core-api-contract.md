# Draft API contract: `tak-core` vs `tak-cli`

Date: 2026-02-11  
Status: Draft (internal-only contract, no semver guarantee yet)  
Task: `828814dbd2d24181`

## 1) Decision baseline (locked at kickoff)

This contract follows the agreed constraints:

1. If API pressure grows, we may add a future middle seam (possible `tak-app`) as a non-blocking follow-up.
2. Output policy lives in core.
3. CLI-centric features live in `tak-cli`; functional behavior lives in `tak-core`.
4. Errors: rich `thiserror` types in `tak-core`, `anyhow` wrapping in `tak-cli`.
5. Tests: most unit/integration in core, CLI keeps smoke/contract tests.
6. Rollout posture: fast-moving, low ceremony.
7. setup/doctor/plugin path behavior unchanged in initial split.
8. API is internal-only first (no public stability commitment yet).

## 2) Crate responsibilities

### `crates/tak-core`

Owns functional behavior and execution policy:
- domain model + IDs + error taxonomy
- persistence/runtime backends (store/index/sidecars/coordination/work/learnings)
- command execution logic for functional commands
- output rendering policy (json/pretty/minimal)
- metrics derivation/aggregation/query validation

### `crates/tak-cli`

Owns process and CLI interface concerns:
- clap parser + subcommand UX/help text
- CLI arg normalization into core command requests
- stdout/stderr writing and process exit semantics
- `anyhow` wrapping at the app edge
- CLI-centric surfaces (interactive shell UX wrappers), while delegating data/behavior to core services

## 3) Proposed core command facade

Internal core entrypoint (shape draft):

```rust
pub struct ExecuteRequest {
    pub cwd: std::path::PathBuf,
    pub format: OutputFormat,
    pub command: CoreCommand,
}

pub struct ExecuteResponse {
    pub stdout: String,
}

pub fn execute(request: ExecuteRequest) -> Result<ExecuteResponse, TakError>;
```

### Notes
- `OutputFormat` stays core-owned.
- Core returns rendered output (`stdout`) so output policy remains centralized.
- `tak-cli` prints response and maps errors/exit codes.
- Internally, command handlers can continue to evolve from `println!` to structured return + renderer; no external guarantee yet.

## 4) Core command namespace (initial)

`CoreCommand` should include functional command families:
- task lifecycle + graph edits (`create/edit/list/show/start/finish/...`)
- claim/next/work engine behavior
- learnings
- mesh/blackboard runtime operations
- import/migrate/reindex/wait/verify/context/log
- metrics query commands (non-interactive)

CLI-centric commands may remain outside this enum initially if needed (e.g., interactive TUI shell wiring), but should consume core query/mutation services.

## 5) Error boundary contract

### In `tak-core`
- Continue using `TakError` (`thiserror`) as the canonical functional error type.
- Preserve machine codes via `TakError::code()`.

### In `tak-cli`
- Convert core failures to `anyhow::Error` at the process boundary.
- Keep existing JSON error envelope behavior (`{"error":"<code>","message":"<text>"}`) in CLI output mode handling.
- Exit code policy remains CLI-owned.

## 6) Layering and forbidden dependency directions

Inside `tak-core`:

- **Domain layer** (`model`, `task_id`, error enums)  
  must not depend on clap/crossterm/ratatui.

- **Infra layer** (`store`, `git`, `agent`, repo/open logic)  
  may depend on domain; must not depend on CLI parser types.

- **App/use-case layer** (`commands`, orchestration facade)  
  depends on domain+infra; no direct clap parser dependency.

- **Presentation layer** (`output` renderers for json/pretty/minimal)  
  depends on app/domain types; still core-owned per decision.

`tak-cli` rules:
- may depend on `tak-core` facade and command/request/result types.
- must not reach into core storage internals directly (`store::*`, `index`, file schemas).

## 7) Dependency partitioning guidance

### Keep in core
Functional deps and shared internals:
- `chrono`, `fs2`, `getrandom`, `git2`, `rusqlite`, `serde`, `serde_json`, `serde_yaml`, `thiserror`, `uuid`

### Keep in cli (or isolate behind CLI-only modules)
CLI/interface deps:
- `clap`
- interactive terminal stack (`crossterm`, `ratatui`)

`colored` can remain where rendering lives; with output policy in core, this likely remains in `tak-core` unless we later split renderer crates.

## 8) Test split contract

### Core tests (majority)
- command behavior and lifecycle semantics
- storage/index/runtime invariants
- JSON output contracts for functional commands
- metrics derivation/aggregation correctness

### CLI tests (targeted)
- clap parse behavior and help/flag compatibility
- process error envelope and exit semantics
- thin smoke tests proving parser -> core facade wiring

## 9) Migration contract (fast path)

Because rollout is intentionally fast:
- allow short break windows during large moves,
- prefer broad mechanical moves + immediate compile fixes over long compatibility shims,
- keep rollback simple (small commit slices where possible, but avoid over-engineering migration scaffolding).

setup/doctor/plugin path behavior remains unchanged during initial split.

## 10) Explicit non-goals for this draft

- No stable public SDK for external consumers.
- No semver guarantees for `tak-core` API yet.
- No mandatory third crate now.

Optional future follow-up (`05d66b0ae43f7d1e`): evaluate `tak-app` seam if command facade grows too broad or mixed.
