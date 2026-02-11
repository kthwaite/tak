# Lightweight rollout notes: crate split (`tak-core` + `tak-cli`)

Date: 2026-02-11  
Mode: fast-moving / low-ceremony rollout

## Current state

- Workspace scaffold exists (`crates/tak-core`, `crates/tak-cli`).
- Core runtime modules were extracted to `tak-core`.
- `tak-cli` now runs through `tak_core::cli::run_cli()` as a thin wrapper.
- Guardrail + smoke tests exist under `crates/tak-cli/tests/`.

## Expected break windows

During ongoing extraction slices, short-lived break windows are expected in:

- module path churn (`src/...` -> `crates/tak-core/src/...`),
- include-str asset paths in moved modules,
- crate-local dependency manifests,
- tests that assume a single-package layout.

Default handling: prefer quick compile restoration over compatibility shims.

## Roll-forward strategy (preferred)

1. Land mechanical move slices quickly.
2. Immediately run:
   - `cargo test -p tak-core --no-run`
   - `cargo test -p tak-cli --tests`
   - `cargo test -p tak --no-run`
   - `cargo test --workspace --no-run`
3. If broken, patch in place and continue forward.

## Quick rollback handle

If a slice is too disruptive:

- revert the latest migration commit(s),
- restore known-good compile state using `cargo test --workspace --no-run`,
- re-land with smaller path-scoped slices.

No heavyweight phased release branch is required for this iteration.

## Coordination notes for multi-agent lanes

- Broadcast before broad path moves (`src/`, `crates/tak-core/`, `crates/tak-cli/`).
- Reserve path prefixes for high-churn edits.
- Post blackboard status notes after each major extraction/dispatch/docs milestone.
- Keep task context updated with exact verification commands used.
