---
name: tak-task-execution
description: Use when an agent needs to find, claim, execute, and complete tasks from a tak task list. Activates when working through a backlog of tasks, when coordinating with other agents, or when the user says "work on the next task", "what should I do next", or "pick up a task".
---

# Task Execution with Tak

Systematic workflow for agents to find available work, claim it, execute it, and report completion. Designed for both single-agent and multi-agent scenarios.

**Critical:** update task state via `tak` commands only (`claim`, `start`, `edit`, `finish`, etc.). Never manually edit or append `.tak/tasks/*.json` (or other `.tak/*` data files).

## Single-Agent Workflow

### 1. Claim available work

```bash
tak claim --assignee <your-name>
```

This atomically finds the next available task (pending, unblocked, unassigned), sets it to `in_progress`, and assigns it to you. If no task is available, it returns an error.

For previewing work without claiming:
```bash
tak next                # preview the next available task
tak list --available    # all available tasks
tak tree --pretty       # full picture of what's left
```

### 2. Understand the task

Read the task details:
```bash
tak show <id>
```

Check if the task has a description with acceptance criteria. If not, check the parent task for context:
```bash
tak show <parent-id>
```

### 3. Execute the work

Do whatever the task requires — write code, fix bugs, create files, run tests.

### 4. Track discovered work immediately

If you discover a bug, follow-up, or side quest while executing:

```bash
# Create a new task/bug issue
# (use --kind bug when appropriate)
tak create "<title>" --kind task -d "<context discovered during execution>"

# Link the current task to the discovered work when there is a scheduling dependency
# (example: current task depends on discovered prerequisite)
tak depend <current-id> --on <new-id>
```

Capture discovered work in `tak` right away; do not leave it only in narrative text or TodoWrite.

### 5. Mark completion

```bash
tak finish <id>
```

### 6. Check for newly unblocked work

```bash
tak list --available
```

Finishing a task may unblock dependent tasks. Check what's now available and continue.

### 7. Repeat

Go back to step 1.

## Multi-Agent Workflow

When multiple agents work from the same task list:

### Use `claim` for atomic task acquisition

Always use `tak claim` instead of `tak next` + `tak start`:

```bash
tak claim --assignee agent-1
```

`tak claim` holds an exclusive file lock while finding and starting the task, preventing two agents from claiming the same work. The `tak next` + `tak start` pattern has a TOCTOU race — another agent can claim the task between the two commands.

### After pulling changes

If another agent has committed task state changes:

```bash
git pull
tak reindex
tak list --available
```

### Handling blocked work

If all available tasks are claimed by other agents, wait or look for other ways to help:

```bash
# See what's in progress
tak list --status in_progress

# See what's blocked and why
tak list --blocked
```

## Status Transitions

```
pending ──→ in_progress ──→ done
   │              │
   │              ├──→ cancelled
   │              │
   │              └──→ pending (un-start)
   │
   └──→ cancelled

done ──→ pending (reopen)
cancelled ──→ pending (reopen)
```

## Best Practices

- **One task at a time**: Finish the current task before starting the next
- **Commit frequently**: Commit after each task so other agents can pull your changes
- **Update status promptly**: Don't leave tasks in `in_progress` after you're done
- **Reindex after merge**: Always `tak reindex` after pulling or merging branches
- **Use descriptive assignee names**: Helps identify which agent is doing what
