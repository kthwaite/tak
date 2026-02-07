---
name: tak-task-management
description: Use when managing tasks, tracking work items, querying task status, or coordinating work in a git repository that uses tak for task management. Activates when the user mentions tasks, issues, bugs, or work tracking, or when a .tak/ directory exists in the repository.
---

# Tak Task Management

You have access to `tak`, a git-native task manager CLI. Tasks are stored as JSON files in `.tak/tasks/` and indexed in SQLite for fast queries. Use `tak` to create, query, update, and track tasks.

## Quick Reference

All commands output JSON by default. Add `--pretty` for human-readable output.

### Creating tasks

```bash
# Create a simple task
tak create "Fix login bug" --kind bug

# Create with description and tags
tak create "Add OAuth support" --kind task -d "Implement Google OAuth2 flow" --tag auth,backend

# Create a child task under a parent
tak create "Write unit tests" --parent 3

# Create with dependencies
tak create "Deploy to staging" --depends-on 4,5
```

### Viewing tasks

```bash
# Show a specific task
tak show 1

# List all tasks
tak list

# List only available (unblocked, unassigned, pending) tasks
tak list --available

# List blocked tasks
tak list --blocked

# Filter by status, kind, tag, or assignee
tak list --status pending
tak list --kind bug
tak list --tag backend
tak list --assignee agent-1

# Show children of a task
tak list --children-of 1

# Show the task tree
tak tree
tak tree 1          # tree rooted at task 1
tak tree --pretty   # with box-drawing characters
```

### Updating tasks

```bash
# Edit fields
tak edit 1 --title "New title" -d "Updated description" --kind epic --tag new-tag

# Claim the next available task (atomic find+start, preferred for multi-agent)
tak claim --assignee agent-1

# Start a specific task
tak start 3 --assignee agent-1

# Mark as done
tak finish 3

# Cancel a task
tak cancel 5

# Reopen a done or cancelled task
tak reopen 3

# Clear assignee without changing status
tak unassign 3
```

### Deleting tasks

```bash
# Delete a leaf task (no children or dependents)
tak delete 5

# Force-delete a task with children/dependents (orphans children, removes deps)
tak delete 1 --force
```

### Managing dependencies

```bash
# Add dependency: task 4 depends on tasks 2 and 3
tak depend 4 --on 2,3

# Remove a dependency
tak undepend 4 --on 2

# Change parent
tak reparent 5 --to 1

# Remove parent (make root-level)
tak orphan 5
```

### Finding work

```bash
# Claim the next available task atomically
tak claim --assignee <your-name>

# Preview the next available task (without claiming)
tak next

# Rebuild the index (after git pull, merge, etc.)
tak reindex
```

## Task Kinds

- **epic**: A large initiative decomposed into child tasks
- **task**: A unit of work
- **bug**: A defect to fix

## Task Statuses

- **pending**: Not started
- **in_progress**: Being worked on (set via `tak start` or `tak claim`)
- **done**: Completed (set via `tak finish`)
- **cancelled**: Won't do (set via `tak cancel`)

A task is **blocked** (derived, not stored) when any of its dependencies are not done or cancelled. A task is **available** when it is pending, unblocked, and unassigned.

## JSON Output Parsing

All commands output single-line JSON by default. Parse with standard JSON tools:

```bash
# Get the ID of the next available task
tak next | jq '.id'

# List available task IDs
tak list --available | jq '.[].id'

# Check if a task is blocked (has unfinished deps)
tak list --blocked | jq '.[].id'
```

## Workflow

1. Check for available work: `tak list --available`
2. Claim a task: `tak claim --assignee <your-name>`
3. Do the work
4. Mark complete: `tak finish <id>`
5. Check what's unblocked: `tak list --available`
6. Repeat

## Important Notes

- Always run `tak reindex` after pulling changes or switching branches
- The `.tak/tasks/` directory should be committed to git
- The `.tak/index.db` file is gitignored (rebuilt on demand)
- Use `tak claim` instead of `tak next` + `tak start` to avoid TOCTOU races in multi-agent setups
