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
- [`coordination-verbosity.md`](./coordination-verbosity.md) — low/medium/high coordination trigger matrix and concrete message examples
- [`channel-contract.md`](./channel-contract.md) — normative channel-role contract for mesh/blackboard/context/history with concrete usage examples
- [`isolated-verification.md`](./isolated-verification.md) — when and how to use temporary git worktrees for targeted verification in noisy multi-agent lanes
- [`meta-refinement-workflow.md`](./meta-refinement-workflow.md) — practical proposal -> `meta` refinement loops with handoff and closeout examples
- [`pupal-phase-policy.md`](./pupal-phase-policy.md) — idea-first intake policy, promotion gates, and explicit defer/reject outcomes
