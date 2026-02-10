---
name: tak-epic-planning
description: Use when the user wants to plan a feature, create an epic, break down large work into child tasks, or design dependency structure in tak.
allowed-tools: "Read,Bash(tak:*)"
---

# Epic Planning with Tak

Guide users through structured decomposition of large initiatives into an epic with child tasks and clear scheduling dependencies.

**Critical:** create/update plan artifacts with `tak` commands only. Never hand-edit `.tak/*` data files.

## Task ID input forms

Wherever a command expects a task ID, you can pass:
- canonical 16-char hex ID,
- unique hex prefix (case-insensitive), or
- legacy decimal ID.

Resolution is exact-match first (canonical or legacy), then unique prefix; ambiguous prefixes return an error, so use a longer prefix.

Examples: `tak show ef94`, `tak depend b48b --on ef94`.

## Planning workflow

### 1) Create the epic

```bash
tak create "Epic title" --kind epic -d "High-level initiative description"
```

### 2) Propose decomposition before creating everything

Discuss and validate:

1. Major phases/components
2. Concrete deliverables per phase
3. Parallelizable vs strictly sequential work
4. External blockers and skill/risk hotspots

Then create agreed tasks.

### 3) Create child tasks

```bash
tak create "Phase 1: schema migration" --parent <epic-id> --kind feature --priority high
tak create "Implement migration" --parent <task-id> --kind task
```

Use kinds intentionally:

- `feature` for coherent capability slices
- `task` for implementation units
- `bug` for defect-fix tasks inside the epic

### 4) Add dependencies (scheduling graph)

```bash
tak depend <blocked-task> --on <blocking-task>
```

Patterns:

- Chain: A → B → C
- Fan-out: A blocks B/C/D
- Fan-in: B/C/D all block E

Only add dependencies when ordering is required.

### 5) Review structure and readiness

```bash
tak tree <epic-id> --pretty
tak list --children-of <epic-id>
tak list --available
```

Validate:

- Parent/child hierarchy is correct
- Dependency graph is acyclic and minimal
- First actionable tasks are unblocked
- Priorities/estimates are sane

### 6) Add coordination scaffolding for multi-agent plans

For cross-team or multi-agent epics, add a blackboard note for shared plan state:

```bash
tak blackboard post \
  --from <agent> \
  --message "Epic <id> planning baseline agreed; use linked tasks for execution updates" \
  --task <epic-id> \
  --tag planning,coordination
```

Update/close as plan stabilizes.

## Decomposition principles

- **Atomic enough to finish** in one focused session when possible
- **Explicit done criteria** (description + contract fields)
- **Minimal dependency edges** (avoid accidental over-blocking)
- **Balanced granularity** (not giant blobs, not hyper-fragmented)
- **Traceable coordination** (blackboard notes for blockers/handoffs)

## Example

```bash
# Epic
tak create "User authentication system" --kind epic -d "Registration, login, sessions"

# Children
tak create "Design auth schema" --parent <epic-id-or-prefix> --kind feature --priority high
tak create "Registration API" --parent <epic-id-or-prefix> --kind task --depends-on <schema-task-id-or-prefix>
tak create "Login API" --parent <epic-id-or-prefix> --kind task --depends-on <schema-task-id-or-prefix>
tak create "Session middleware" --parent <epic-id-or-prefix> --kind feature --depends-on <registration-id-or-prefix>,<login-id-or-prefix>
tak create "Auth integration tests" --parent <epic-id-or-prefix> --kind task --depends-on <session-id-or-prefix>

# Review
tak tree <epic-id-or-prefix> --pretty
```
