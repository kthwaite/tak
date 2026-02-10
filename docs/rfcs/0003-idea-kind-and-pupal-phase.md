# RFC 0003: Idea Kind and Pupal-Phase Refinement Workflow

- **Status:** Draft
- **Date:** 2026-02-09
- **Author:** nimble-otter-e114
- **Related task:** Epic [#78] Idea intake and pupal-phase workflow
- **Related tasks:** [#79] Draft RFC for idea kind and pupal-phase lifecycle, [#36] Add 'meta' task type for planning/refinement workflows, [#81] Define pupal-phase policy and operator guidance
- **Related docs:**
  - [`docs/rfcs/0002-verbose-team-communication.md`](./0002-verbose-team-communication.md)
  - [`docs/how/meta-refinement-workflow.md`](../how/meta-refinement-workflow.md)
  - [`docs/how/pupal-phase-policy.md`](../how/pupal-phase-policy.md)

---

## Summary

This RFC proposes adding a first-class `idea` task kind and defining a formal **pupal phase** that uses `meta` tasks to refine ideas into executable work.

Target pipeline:

1. **Idea intake** (`kind=idea`)
2. **Pupal refinement** (`kind=meta` planning/refinement tasks)
3. **Promotion** into executable **epic/feature/task** graph

The key policy change is: **new concepts enter as ideas first**, then move through refinement before becoming normal execution work.

---

## Problem

Today, early concepts and implementation-ready work can be mixed together in the same kinds (`feature`/`task`/`epic`), which creates avoidable friction:

1. Backlogs include items that are not ready for execution.
2. Claim/next flows can pull under-specified planning items into active implementation lanes.
3. It is hard to audit how a vague idea became concrete work.
4. Teams lack a standard “proposal incubator” state in tak itself.

---

## Goals

1. Introduce `kind=idea` as canonical intake for new concepts.
2. Define a standard pupal-phase loop using `meta` tasks for refinement.
3. Add explicit promotion criteria from `idea` -> executable work.
4. Preserve traceability from execution tasks back to originating idea.
5. Prevent accidental execution of raw ideas by default claim/next behavior.

## Non-goals

1. Replacing existing lifecycle statuses (`pending/in_progress/done/cancelled`).
2. Forcing one single planning method for all teams.
3. Shipping a full strategic planning engine in this RFC.
4. Breaking compatibility for existing task files.

## Policy boundary: #36 implementation guidance vs #81 policy

To avoid doc drift and duplicate guidance:

- **#36 scope (implementation guidance):** `kind=meta` behavior and practical refinement operations (lifecycle/claim/dependency parity, operator examples). See [`docs/how/meta-refinement-workflow.md`](../how/meta-refinement-workflow.md).
- **#81 scope (policy):** pupal-phase intake/promotion/defer/reject policy rules, including operator expectations for when idea-first flow is required. See [`docs/how/pupal-phase-policy.md`](../how/pupal-phase-policy.md).

This RFC provides design rationale and rollout framing; the how-docs capture current operational guidance.

---

## Terminology

- **Idea:** low-commitment proposal record, intentionally not execution-ready.
- **Pupal phase:** refinement stage where idea scope is iterated and transformed into concrete work.
- **Meta task:** planning/refinement task that modifies or generates other tasks.
- **Promotion:** explicit conversion from idea/refinement output into executable epic/feature/task set.

---

## Proposed design

## 1) New kind: `idea`

Add `idea` to `Kind` and CLI value enums so users can create:

```bash
tak create "Potential migration to X" --kind idea
```

Expected semantics:

- `idea` is valid task data and appears in `show/list/tree`.
- `idea` can carry description, tags, contract, planning metadata.
- `idea` remains low-commitment until promoted.

## 2) Pupal phase via `meta`

Pupal phase is represented by one or more linked `meta` tasks (see related task #36 and [`docs/how/meta-refinement-workflow.md`](../how/meta-refinement-workflow.md) for current operator workflow).

Typical loop:

1. Create/update an `idea`.
2. Create `meta` refinement task linked to that idea.
3. Refine in iterations (scope, constraints, risks, decomposition).
4. Produce a promotion decision:
   - promote to executable work,
   - park/defer,
   - reject/cancel with rationale.

## 3) Promotion policy

Promotion from idea to execution should require explicit refinement output:

Minimum gate:

1. objective and scope are stable enough,
2. acceptance criteria or decomposition exists,
3. major blockers/risks are identified,
4. owner/next action is clear.

Promotion result should be one of:

- new `epic` + children,
- one or more `feature` tasks,
- one or more direct `task`/`bug` items.

## 4) Traceability model

Every promoted execution item should keep a trace back to the source idea and refinement steps.

Implementation status/options (phased):

- **Current baseline:** reserve structured traceability fields via task extensions (`origin_idea_id`, `refinement_task_ids`) and derive them from parent/dependency links during `tak create`.
- **Future hardening:** graduate these into fully typed first-class schema fields if/when stricter validation/migration is needed.

## 5) Scheduling behavior

To avoid accidental execution of raw ideas:

- `tak next` and `tak claim` exclude `kind=idea` by default via index availability filtering.
- Ideas remain visible through explicit filters (`tak list --kind idea`).
- Meta tasks remain claimable because they are active refinement work.

---

## CLI / UX implications

Potential command-surface additions (incremental):

1. `tak create --kind idea`
2. Optional helper command for promotion, e.g.:
   - `tak promote <idea-id> --to epic|feature|task`
3. Optional list source/filter shortcuts:
   - ideas backlog,
   - ready-for-execution backlog (excluding ideas)

This RFC does not require all helper commands in one release; policy and core kind support can land first.

---

## Rollout plan

### Phase A (policy + docs)

- Publish this RFC.
- Document intake/pupal/promotion conventions.
- Encourage all new concept capture as `kind=idea`.

### Phase B (core kind support)

- Add `idea` to model, CLI enums, index, output, and tests.
- Ensure JSON round-trip and query support are stable.

### Phase C (promotion ergonomics + traceability)

- Add promotion helper(s) and explicit trace links.
- Update claim/next defaults to exclude ideas.
- Add docs/examples for idea -> meta -> execution workflows.

---

## Backward compatibility

- Existing tasks remain valid.
- Existing workflows continue to work without using `idea`.
- Teams can adopt idea/pupal policy incrementally.

---

## Success criteria

1. New concepts are consistently captured as ideas first.
2. Fewer under-specified items enter execution claim flow.
3. Promotion decisions are explicit and auditable.
4. Execution tasks can be traced back to original ideas/refinement history.

---

## Open questions

1. Should promotion create new tasks only, or optionally mutate an idea into a feature/task?
2. Should idea items ever be startable directly, or always require promotion?
3. When should traceability extension fields be promoted to strict first-class schema with migrations?
4. Should there be policy checks/lints that warn when execution tasks are created without idea/refinement linkage?

---

## Initial recommendation

Adopt the model in this order:

1. Ship `idea` kind.
2. Finalize `meta` kind + pupal policy.
3. Add promotion/linking ergonomics.

This delivers immediate intake clarity while keeping implementation risk low.
