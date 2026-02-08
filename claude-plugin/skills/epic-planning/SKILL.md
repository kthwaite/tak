---
name: tak-epic-planning
description: Use when the user wants to plan a feature, create an epic, break down a large task into subtasks, or design a work breakdown structure. Activates when the user says things like "plan this feature", "break this down", "create an epic", or "decompose this into tasks".
---

# Epic Planning with Tak

Guide the user through structured decomposition of a large feature or initiative into an epic with child tasks and dependency relationships.

## Planning Workflow

### Step 1: Create the Epic

```bash
tak create "Epic title" --kind epic -d "High-level description of the initiative"
```

### Step 2: Brainstorm and Propose Tasks

Before creating tasks, discuss the decomposition with the user:

1. Identify the major components or phases of the work
2. For each component, identify concrete deliverable tasks
3. Consider what can be parallelized vs what must be sequential
4. Identify external dependencies or blockers

Present the proposed task breakdown to the user for feedback before creating anything.

### Step 3: Create Child Tasks

For each agreed-upon task:

```bash
tak create "Task description" --parent <epic-id> --kind task
```

For subtasks of tasks:

```bash
tak create "Subtask description" --parent <task-id> --kind task
```

### Step 4: Establish Dependencies

Dependencies represent scheduling constraints — task B cannot start until task A is done:

```bash
tak depend <blocked-task> --on <blocking-task>
```

Common patterns:
- **Sequential chain**: A -> B -> C (each depends on the previous)
- **Fan-out**: A blocks B, C, D (shared prerequisite)
- **Fan-in**: B, C, D all block E (integration point)

### Step 5: Review the Plan

```bash
tak tree <epic-id> --pretty
```

This shows the full hierarchy with blocked status. Verify:
- All tasks have appropriate parents
- Dependencies form a DAG (no cycles)
- The first tasks to work on are unblocked
- Nothing critical is missing

### Step 6: Iterate

If the plan needs changes:
- Add missing tasks: `tak create ...`
- Remove incorrect dependencies: `tak undepend <id> --on <id>`
- Restructure hierarchy: `tak reparent <id> --to <new-parent>`
- Cancel unnecessary tasks: `tak cancel <id>`

## Decomposition Principles

- **Atomic tasks**: Each task should be completable in one focused session
- **Clear acceptance criteria**: Include enough description that another agent could implement it
- **Minimal dependencies**: Only add dependencies where there's a genuine ordering constraint
- **Balanced granularity**: Not too coarse (hard to track) or too fine (overhead exceeds value)

## Example

```bash
# Create the epic
tak create "User authentication system" --kind epic -d "Add login, registration, and session management"

# Create phase 1 tasks
tak create "Design auth database schema" --parent 1
tak create "Implement user registration API" --parent 1 --depends-on 2
tak create "Implement login API" --parent 1 --depends-on 2
tak create "Add session management middleware" --parent 1 --depends-on 3,4
tak create "Write integration tests" --parent 1 --depends-on 5

# Review
tak tree 1 --pretty
```

Output:
```
[1] User authentication system (epic, pending)
├── [2] Design auth database schema (task, pending)
├── [3] Implement user registration API (task, pending) [BLOCKED]
├── [4] Implement login API (task, pending) [BLOCKED]
├── [5] Add session management middleware (task, pending) [BLOCKED]
└── [6] Write integration tests (task, pending) [BLOCKED]
```

Tasks 2 is available to start immediately. Tasks 3 and 4 unblock after 2 is done. Task 5 unblocks after both 3 and 4 are done.
