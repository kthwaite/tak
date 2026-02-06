---
name: tak-task-execution
description: Use when an agent needs to find, claim, execute, and complete tasks from a tak task list. Activates when working through a backlog of tasks, when coordinating with other agents, or when the user says "work on the next task", "what should I do next", or "pick up a task".
---

# Task Execution with Tak

Systematic workflow for agents to find available work, claim it, execute it, and report completion. Designed for both single-agent and multi-agent scenarios.

## Single-Agent Workflow

### 1. Find available work

```bash
tak next
```

This returns the next available task (pending, unblocked, unassigned). If no task is available, it returns `null`.

For more context:
```bash
tak list --available    # all available tasks
tak tree --pretty       # full picture of what's left
```

### 2. Claim the task

```bash
tak start <id> --assignee <your-name>
```

This sets the task to `in_progress` and records who is working on it.

### 3. Understand the task

Read the task details:
```bash
tak show <id>
```

Check if the task has a description with acceptance criteria. If not, check the parent task for context:
```bash
tak show <parent-id>
```

### 4. Execute the work

Do whatever the task requires — write code, fix bugs, create files, run tests.

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

### Claiming prevents conflicts

Always use `--assignee` when starting a task:

```bash
tak start <id> --assignee agent-1
```

Other agents running `tak next` will not see tasks that are already `in_progress`.

### Check before assuming

Always query `tak next` or `tak list --available` before starting work. Don't assume a task is still available — another agent may have claimed it.

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
