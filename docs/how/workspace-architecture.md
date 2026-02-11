# Workspace crate architecture

Tak is organized as a Cargo workspace with a core library + thin CLI wrapper.

## Crate boundaries

- `crates/tak-core/`
  - owns functional behavior (model, storage/index, lifecycle/commands, metrics, output policy)
  - exports shared facade helpers used by binaries
- `crates/tak-cli/`
  - binary wrapper
  - should remain thin: process entrypoint + exit behavior; delegates command behavior to `tak-core`
- root package (`tak`)
  - compatibility layer while migration lands
  - `src/lib.rs` re-exports `tak_core::*`

## Where to add code

- New feature behavior: `crates/tak-core/src/...`
- New CLI wrapper-only checks (shape/guardrails): `crates/tak-cli/tests/...`
- Root `src/` should generally be treated as compatibility glue, not primary feature implementation.

## Verification workflow in workspace mode

Use targeted verification during refactors:

```bash
# Core behavior + unit/integration tests
cargo test -p tak-core

# Wrapper smoke + guardrail tests
cargo test -p tak-cli --tests

# Ensure root package compatibility still compiles
cargo test -p tak --no-run

# Quick workspace-wide compile confidence
cargo test --workspace --no-run
```

## Guardrails

`crates/tak-cli/tests/guardrails.rs` enforces that `tak-cli` stays a thin wrapper (no direct `clap` dependency and no local command-dispatch logic in `main.rs`).

If you intentionally change wrapper responsibilities, update guardrails in the same PR and document rationale.
