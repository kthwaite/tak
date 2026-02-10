# Meta Refinement Workflow (Proposal -> Executable Plan)

This page documents practical, current-state guidance for using `kind=meta` tasks to refine rough proposals into executable Tak work.

Related design/policy references:
- [`docs/rfcs/0003-idea-kind-and-pupal-phase.md`](../rfcs/0003-idea-kind-and-pupal-phase.md)
- Feature [#81] Define pupal-phase policy and operator guidance

## What this workflow does

Use a `meta` task when the primary output is **better task structure**, not direct product/code changes.

Typical outcomes of a meta loop:

- clarified objective and constraints,
- decomposed epic/feature/task graph,
- explicit dependencies and sequencing,
- clear handoff notes for the next executor.

## Source of truth and invariants

- `meta` is a first-class task kind in the same task model as `epic/feature/task/bug`.
- `meta` tasks use the same lifecycle transitions (`start`, `handoff`, `finish`, etc.) and sidecar history logging.
- Coordination channels (mesh/blackboard) are communication layers; final planning state must be persisted in task files.
- Claim/next behavior is unchanged: if a `meta` task is pending + unblocked, it can be claimed like other work.

## Scope boundary with RFC-0003 and #81 policy work

This page is **implementation guidance** for `kind=meta` operations (task #36 scope). It intentionally does **not** define final intake/promotion policy for idea-first workflows.

Policy decisions such as:
- when concepts must start as `kind=idea`,
- required promotion gates,
- defer/reject governance,

belong to RFC-0003 policy finalization under task #81.

## When to choose `meta` vs implementation kinds

Choose **`meta`** when most effort is:

- creating/updating/reparenting tasks,
- writing/refining contract/planning fields,
- sequencing work and identifying blockers,
- producing handoff-ready plan artifacts.

Choose **`task`/`feature`/`bug`** when the primary output is implementation or defect fix work.

## Proposal -> refinement loop (single-agent example)

### 1) Open a refinement task

```bash
tak create "Refine rollout plan for selective sync" \
  --kind meta \
  --tag planning --tag proposal \
  --objective "Convert proposal into executable work graph" \
  --criterion "Feature decomposition is complete" \
  --criterion "Each execution task has owner-ready scope"
```

### 2) Start and capture planning context

```bash
tak start <meta-id> --assignee planner-1
tak context <meta-id> --set "Open questions, assumptions, and candidate decomposition."
```

### 3) Materialize executable tasks

```bash
# Example: create feature + implementation tasks
tak create "Selective sync API" --kind feature --parent <epic-id>
tak create "Server endpoint for selective sync" --kind task --parent <feature-id>
tak create "Client integration for selective sync" --kind task --parent <feature-id>

# Add sequencing edges
tak depend <client-task-id> --on <server-task-id> --reason "API must land first"
```

### 4) Validate shape and readability

```bash
tak list --kind meta
tak tree <epic-id>
```

### 5) Finish the meta task

```bash
tak finish <meta-id>
```

## Proposal -> refinement loop (handoff example)

Use this when planning spans multiple sessions or owners.

### Planner A (in progress)

```bash
tak start <meta-id> --assignee planner-a
tak blackboard post --from planner-a --template status --task <meta-id> \
  --message "Draft decomposition complete; reviewing dependency order."
```

### Planner A hands off with explicit next action

```bash
tak handoff <meta-id> \
  --summary "Created feature + 4 child tasks; unresolved risk is migration fallback. Next: confirm rollback criteria and finalize dependency edges."
```

### Planner B resumes and closes

```bash
tak start <meta-id> --assignee planner-b
tak edit <task-id> --constraint "Must support rollback without downtime"
tak finish <meta-id>
```

## Coordination guidance for meta work

- Reserve paths before edits (docs/plans/task files) in multi-agent lanes.
- Post durable blackboard updates when the plan changes materially.
- Keep mesh pings brief; put long-form rationale in task context/history.
- Prefer `handoff` over silent abandonment when refinement is incomplete.

## Edge cases and operator tips

- If refinement stalls on external input, keep the `meta` task active and post a blocker note, or handoff with concrete asks.
- If decomposition reveals implementation already underway, reparent/link tasks instead of duplicating work.
- Keep acceptance criteria concrete enough that execution owners can validate done-ness without reopening planning.
- If no executable work is justified, cancel the meta task with a reason so the decision is auditable.

## Code pointers

- `src/model.rs` (`Kind::Meta` and task schema)
- `src/main.rs` (CLI parsing for `--kind meta`)
- `src/output.rs` (`meta` rendering in pretty/minimal output)
- `src/commands/lifecycle.rs` (start/handoff/finish behavior)

## Test pointers

- `tests/meta_output_integration.rs` (create/show/list/tree output coverage for `meta`)
- `tests/meta_kind_regression_integration.rs` (legacy kind-filter/default behavior remains stable)
- `tests/integration.rs` (lifecycle/dependency baselines shared by all kinds)
