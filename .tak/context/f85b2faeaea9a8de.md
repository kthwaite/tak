Feature complete:
- 123711174b6a4404: added parity matrix tests (`crates/tak-cli/tests/parity_smoke.rs`) and wrapper guardrails (`crates/tak-cli/tests/guardrails.rs`).
- 5966f17b820fa3ca: updated README + CLAUDE + docs/how with workspace architecture guidance and targeted verification commands.
- ae43452fb05a9135: added lightweight rollout notes (`docs/plans/2026-02-11-crate-split-rollout-notes.md`).

Validation references:
- `cargo test -p tak-cli --tests`
- `cargo test -p tak --no-run`
- `cargo test --workspace --no-run`