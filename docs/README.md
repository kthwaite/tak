# Tak Documentation

This directory contains both historical design material and current behavior docs.

## Doc map

- [`how/`](./how/): **Current behavior docs** (“how things work”) for contributors/operators.
- [`rfcs/`](./rfcs/): design proposals and rationale.
- [`plans/`](./plans/): implementation plans/checklists from delivery work.
- [`benchmarks/`](./benchmarks/): performance analysis and harness details.
- Top-level deep dives (for example `hash-id-collision-rationale.md`, `stigmergic-coordination-design.md`).

## Reading order for contributors

1. Start in [`how/`](./how/) for current behavior.
2. Use RFCs/plans only when you need history, alternatives, or design rationale.
3. Cross-check behavior claims against source paths under `src/` when updating docs.

## Adding a new "how it works" page

1. Add/update content under `docs/how/`.
2. Link it from `docs/how/README.md`.
3. Prefer concrete references:
   - command entry points in `src/commands/`
   - core model/state code in `src/model.rs` and related modules
   - tests that validate behavior
4. Keep historical implementation notes in `docs/plans/` and keep `docs/how/` focused on current behavior.
