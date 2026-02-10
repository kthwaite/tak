# How Tak Works (Current Behavior)

Use this section for contributor-facing docs that describe how Tak behaves **today**.

## Scope

Include:
- command behavior and side effects
- data flow and persistence model
- invariants and safety checks
- common edge cases and operator expectations

Exclude:
- implementation planning checklists (put those in `docs/plans/`)
- speculative proposals (put those in `docs/rfcs/`)

## Suggested page template

Each page should include:

1. **What this subsystem does**
2. **Source of truth + invariants**
3. **Execution flow** (happy path)
4. **Edge cases / idempotency / failure behavior**
5. **Code pointers** (`src/...`)
6. **Test pointers**

## Seed pages

- [`setup-and-integrations.md`](./setup-and-integrations.md) — `tak setup` behavior (hooks, plugin, skills, `--pi`)
- [`storage-and-index.md`](./storage-and-index.md) — JSON file store + SQLite index model and rebuild semantics
- [`task-lifecycle.md`](./task-lifecycle.md) — lifecycle state machine and command side effects
