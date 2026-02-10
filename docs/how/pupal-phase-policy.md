# Idea Intake and Pupal-Phase Policy

This page defines the current operator policy for idea-first planning and promotion.

Related references:
- [`docs/rfcs/0003-idea-kind-and-pupal-phase.md`](../rfcs/0003-idea-kind-and-pupal-phase.md)
- [`meta-refinement-workflow.md`](./meta-refinement-workflow.md)

## What this policy does

It standardizes how teams move work from concept -> refinement -> execution:

1. capture new concepts as `kind=idea`,
2. refine via `kind=meta` tasks,
3. promote only when execution gates are met,
4. explicitly defer or reject when promotion is not warranted.

## Source of truth and invariants

- `idea` and `meta` are first-class `Kind` values in the same task schema.
- Refinement state must be persisted in task files (`.tak/tasks/*.json`), not only mesh/blackboard chat.
- `tak next` and `tak claim` use index availability and **exclude `kind=idea` by default**.
- Direct execution of an idea requires explicit operator action (for example, `tak start <idea-id>`), not passive claim flow.

## Policy rules

### 1) Intake: default to `kind=idea` for non-executable concepts

Create as `idea` when any of these are true:
- scope/objective is not stable,
- decomposition is missing,
- ownership or sequencing is unclear,
- key risks/blockers are still unknown.

If work is already execution-ready, creating `feature/task/bug` directly is allowed.

### 2) Refinement: use `kind=meta` for planning work

Open a `meta` task when the primary output is improved structure (decomposition, dependencies, constraints, acceptance criteria), not implementation artifacts.

### 3) Promotion gate: all must be true before creating execution tasks

- objective/scope are stable enough to execute,
- acceptance criteria or equivalent decomposition exists,
- major risks and blockers are identified,
- clear next owner/action is defined.

### 4) Outcome is mandatory: promote, defer, or reject

Every active idea should end each refinement pass in one explicit state:

- **Promote:** create execution tasks (`epic/feature/task/bug`) and link back to origin idea/refinement context.
- **Defer:** keep idea pending with explicit revisit trigger/date in context.
- **Reject:** cancel idea with a reason so the decision is auditable.

### 5) Traceability minimum (until structured fields are expanded)

At minimum, keep origin links through:
- task references (`--parent`, `depend`, related IDs in descriptions/context),
- blackboard notes tied to idea/meta IDs,
- handoff summaries that mention promotion/defer/reject rationale.

## Operator workflow (recommended)

### A) Intake

```bash
tak create "Investigate selective sync strategy" --kind idea --tag proposal
```

### B) Open refinement loop

```bash
tak create "Refine selective sync proposal" --kind meta --tag planning --depends-on <idea-id>
tak start <meta-id> --assignee planner-1
```

### C) Decide outcome

**Promote:**

```bash
tak create "Selective sync rollout" --kind epic
# create feature/task children and dependency edges
```

**Defer:**

```bash
tak context <idea-id> --set "Deferred until Q2 load-test results; revisit by 2026-04-15"
```

**Reject:**

```bash
tak cancel <idea-id> --reason "Rejected: complexity outweighs expected benefit"
```

## Enforcement points in code

- Availability filtering (claim/next source): `src/store/index.rs` (`Index::available`)
- Claim command behavior: `src/commands/claim.rs`
- Next command behavior: `src/commands/next.rs`

## Test pointers

- `src/store/index.rs` tests:
  - `available_excludes_idea_tasks_by_default`
  - `available_with_assignee_still_excludes_idea_tasks`
- `src/commands/claim.rs` tests:
  - `claim_next_skips_idea_tasks_by_default`
  - `claim_next_returns_none_when_only_idea_tasks_are_pending`
- Existing integration suites validating kind parsing/output remain relevant for idea/meta visibility.
