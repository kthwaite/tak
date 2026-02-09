---
name: tak-task-execution
description: 'Use when an agent needs to find, claim, execute, and complete tasks from a tak task list, including when the user asks for "/tak work", "/tak work status", or "/tak work stop" behavior.'
allowed-tools: "Read,Bash(tak:*)"
---

# Task Execution with Tak

Systematic workflow for agents to find available work, claim it, execute it, and report completion in tak-managed repositories.

**Critical:** update task state via `tak` commands only (`claim`, `start`, `handoff`, `finish`, `cancel`, etc.). Never manually edit `.tak/*` data files.

## Claude `/tak work` loop (pi-parity mode)

Claude Code does not provide the same extension runtime hooks/guards as pi. Emulate the same workflow behavior conversationally.

When the user requests `/tak work`, interpret it as:

```text
/tak work [tag:<tag>] [limit:<n>] [verify:isolated|local]
```

Support these control variants:

```text
/tak work status
/tak work stop
```

### Loop algorithm

1. **Ensure agent identity**
   - If needed, join mesh and capture identity:
     ```bash
     tak mesh join --format minimal
     ```
   - Reuse that name for `claim`, `start`, reservation, and coordination commands.

2. **Attach or claim**
   - First check if you already own in-progress work:
     ```bash
     tak list --status in_progress --assignee <agent>
     ```
   - If none, claim atomically:
     ```bash
     tak claim --assignee <agent>
     tak claim --assignee <agent> --tag <tag>   # when tag filter requested
     ```

3. **Load execution context**
   ```bash
   tak show <id>
   tak context <id>
   tak blackboard list --status open --task <id>
   ```

4. **Coordinate before major edits**
   ```bash
   tak mesh reserve --name <agent> --path <path> --reason task-<id>
   ```

5. **Run the task to completion or handoff**
   - Done:
     ```bash
     tak finish <id>
     ```
   - Blocked but resumable:
     ```bash
     tak handoff <id> --summary "<what is done, what is blocked, and exact next step>"
     tak blackboard post --from <agent> --message "<blocker + unblock request>" --task <id> --tag blocker,coordination
     ```
   - Abandon/cancel:
     ```bash
     tak cancel <id> --reason "<why>"
     ```

6. **Release reservations when leaving the task**
   ```bash
   tak mesh release --name <agent> --all
   ```

7. **Continue loop until stop/limit/no work**
   - If a `limit:<n>` was requested, stop after `n` completed/handed-off/cancelled tasks.
   - Otherwise auto-claim next available task and repeat.

### `/tak work status`

Report:

```bash
tak list --available
tak list --blocked
tak list --status in_progress --assignee <agent>
tak blackboard list --status open --limit 10
```

Include whether a loop is active, current task (if any), and remaining limit (if set).

### `/tak work stop`

Stop the loop intent and clean up:

```bash
tak mesh release --name <agent> --all
```

If there is an in-progress task, leave it in a truthful lifecycle state (`in_progress` if still active, or handoff/cancel if stepping away).

## Verify mode semantics

When `/tak work verify:isolated` (default):

- Be conservative with local build/test/check commands while peers are active on mesh and may hold overlapping reservations.
- Coordinate first via mesh/blackboard if verification likely touches shared paths.

When `/tak work verify:local`:

- Run normal local verification for the task.

## Standard (non-loop) workflow

1. Claim available work:
   ```bash
   tak claim --assignee <your-name>
   ```
2. Understand the task:
   ```bash
   tak show <id>
   ```
3. Execute.
4. Capture discovered follow-up work immediately:
   ```bash
   tak create "<title>" --kind task -d "<context>"
   tak depend <current-id> --on <new-id>   # when scheduling dependency exists
   ```
5. Finish or hand off:
   ```bash
   tak finish <id>
   # or
   tak handoff <id> --summary "..."
   ```

## Multi-agent requirements

- Prefer `tak claim` over `tak next` + `tak start` to avoid TOCTOU races.
- Reindex after pull/merge/branch switch:
  ```bash
  tak reindex
  ```
- Do not silently take over tasks assigned to another agent.
- Use mesh + blackboard when blocked on reservations or cross-agent dependencies.

## Status transitions

```
pending ──→ in_progress ──→ done
   │              │
   │              ├──→ cancelled
   │              └──→ pending (handoff)
   │
   └──→ cancelled

done/cancelled ──→ pending (reopen)
```

## Best practices

- Keep one active task at a time unless explicitly parallelizing.
- Use handoff summaries that are actionable for the next agent.
- Keep reservation scope narrow (file/dir level, not repo-wide).
- Keep blackboard notes concise: blocker, owner/path, required action, next check time.
