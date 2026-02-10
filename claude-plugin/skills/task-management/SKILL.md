---
name: tak-task-management
description: Use when managing tasks, querying task status, updating lifecycle, coordinating via mesh/blackboard, or recording learnings in repositories that use tak.
allowed-tools: "Read,Bash(tak:*)"
---

# Tak Task Management

You have access to `tak`, a git-native task manager CLI. Tasks are stored as JSON files in `.tak/tasks/`, indexed in SQLite for fast querying, and coordinated through mesh + blackboard runtime state.

## Critical rule: use CLI commands, never manual `.tak/*` edits

For task/learning/context/history/coordination changes:

- ✅ Use `tak` commands (`create`, `edit`, `start`, `finish`, `blackboard post`, etc.)
- ❌ Do **not** hand-edit `.tak/tasks/*.json`, `.tak/learnings/*.json`, `.tak/context/*`, `.tak/history/*`, `.tak/runtime/*`, `.tak/counter.json`, or `.tak/index.db`

If repo state looks stale after branch changes, run:

```bash
tak reindex
```

## `tak` vs TodoWrite

| Use tak | Use TodoWrite |
|---|---|
| Persistent project work items shared across agents | Ephemeral one-conversation checklist |
| Dependencies, lifecycle state, assignees, history | Personal scratch execution steps |
| Anything teammates/agents should see later | Temporary note-taking |

## Session default in tak repos

1. Check ready work: `tak list --available` (or `tak next`)
2. Inspect: `tak show <id>`
3. Transition with lifecycle commands (`claim/start/handoff/finish/cancel/reopen`)
4. Coordinate with mesh + blackboard when other agents are active
5. Reindex after pull/merge/switch: `tak reindex`

> If the user asks for `/tak work` behavior, use the **tak-task-execution** skill flow.

## Isolated verification fallback (use sparingly)

Default is to verify in the shared working tree.

Use a temporary detached `git worktree` only when shared-tree verification is failing for unrelated in-progress edits and you need targeted evidence for your lane.

Required steps:
1. Post/update blackboard note with reason and planned verification command(s).
2. Run targeted tests in isolated worktree.
3. Report that evidence source explicitly in task notes/handoff.
4. Remove the temporary worktree after verification.

Reference: `docs/how/isolated-verification.md`.

## Quick reference

All commands output JSON by default. Add `--pretty` for readable output.

### Create tasks

```bash
tak create "Fix login bug" --kind bug
tak create "Implement OAuth" --kind feature -d "Google OAuth2 flow" --tag auth,backend
tak create "Write unit tests" --parent 3
tak create "Deploy staging" --depends-on 4,5
```

With contract + planning fields:

```bash
tak create "Refactor auth module" \
  --objective "Consolidate auth logic" \
  --verify "cargo test" --verify "cargo clippy" \
  --constraint "No unsafe code" \
  --criterion "No public API changes" \
  --priority high --estimate m --risk medium --skill rust
```

### Query tasks

```bash
tak show 12
tak list
tak list --available
tak list --blocked
tak list --status in_progress
tak list --kind feature
tak list --tag backend
tak list --assignee agent-1
tak list --priority critical
tak list --children-of 5
tak tree
tak tree 5 --pretty
```

### Update tasks

```bash
tak edit 12 --title "New title" -d "Updated description" --kind task --tag auth,api
tak edit 12 --objective "Updated objective" --verify "cargo test"
tak edit 12 --priority critical --estimate s --risk high --skill rust --skill sql
tak edit 12 --pr "https://github.com/org/repo/pull/42"
```

Lifecycle:

```bash
tak claim --assignee agent-1
tak start 12 --assignee agent-1
tak handoff 12 --summary "Implemented parser; blocked on schema migration"
tak finish 12
tak cancel 12 --reason "Superseded by new approach"
tak reopen 12
tak unassign 12
```

### Dependencies and hierarchy

```bash
tak depend 12 --on 3,4 --dep-type hard --reason "Needs schema + API"
tak undepend 12 --on 3
tak reparent 12 --to 2
tak orphan 12
```

### Context, history, verification

```bash
tak context 12 --set "Careful: migration must be backwards-compatible"
tak context 12
tak context 12 --clear
tak log 12
tak verify 12
```

### Learnings

```bash
tak learn add "Avoid N+1 query" --category pattern -d "Batch preload relationships" --tag db,perf --task 12
tak learn list --tag perf
tak learn suggest 12
```

### Mesh coordination

```bash
tak mesh join --name agent-1
tak mesh list
tak mesh inbox --name agent-1 --ack
tak mesh send --from agent-1 --to agent-2 --message "Can you release src/store?"
tak mesh reserve --name agent-1 --path src/store --reason task-12
tak mesh release --name agent-1 --path src/store
```

### Blackboard coordination

```bash
tak blackboard post --from agent-1 --template blocker --message "Blocked by migration lock" --task 12 --tag db
tak blackboard post --from agent-1 --template handoff --message "Handing off parser follow-up" --task 12
tak blackboard post --from agent-1 --template status --message "Verification pass complete" --task 12
tak blackboard list --status open --task 12
tak blackboard show 7
tak blackboard close 7 --by agent-2 --reason "Lock released"
tak blackboard reopen 7 --by agent-1
```

### Workflow therapist

```bash
tak therapist offline --by agent-1 --limit 200
tak therapist online --by agent-1
tak therapist log --limit 20
```

## Data model quick notes

### Task kinds

- `epic` — large initiative with children
- `feature` — user-facing capability or architectural slice
- `task` — standard work unit
- `bug` — defect fix

### Statuses

- `pending`
- `in_progress`
- `done`
- `cancelled`

Blocked state is derived from dependencies (not persisted as status).

### Sidecar/runtime files (managed by CLI)

- `.tak/context/{id}.md` — free-form task context notes
- `.tak/history/{id}.jsonl` — lifecycle event history
- `.tak/verification_results/{id}.json` — verification output
- `.tak/artifacts/{id}/` — task artifacts
- `.tak/runtime/mesh/*` — mesh state (registry/inbox/reservations/feed)
- `.tak/runtime/blackboard/*` — blackboard notes + counter
- `.tak/therapist/observations.jsonl` — append-only therapist observations

## Practical workflow

1. `tak list --available`
2. `tak claim --assignee <name>`
3. `tak show <id>` + `tak context <id>`
4. Reserve touched paths before major edits
5. Execute + verify (`tak verify <id>` when contract has verification)
6. `tak finish <id>` (or `tak handoff` / `tak cancel` with reasons)
7. Re-check unblocked work: `tak list --available`
