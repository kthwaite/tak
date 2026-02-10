---
name: tak-coordination
description: Use when working in a tak-managed repository to pick tasks, coordinate with other agents via mesh, and manage blockers through blackboard notes.
allowed-tools: "read bash tak_cli"
---

# Tak Coordination

Use tak as the source of truth for task state and multi-agent coordination.

## Task ID input forms

Wherever a task ID is expected, tak accepts:
- full 16-char hex ID (canonical),
- unique hex prefix (case-insensitive), or
- legacy decimal ID.

Resolution is exact-match first, then unique prefix; ambiguous prefixes error and should be lengthened.

Examples: `tak show ef94`, `tak depend b48b --on ef94`.

## Core rules

1. **Use tak commands for task state** (`create`, `list`, `show`, `claim`, `start`, `handoff`, `finish`, `cancel`, `reopen`).
2. **Prefer `tak_cli`** for structured task/mesh/blackboard operations.
3. **Prioritise by urgency, then age** (critical/high first; oldest first within each priority).
4. **Coordinate on mesh** before overlapping work:
   - check active peers (`mesh list`),
   - check inbox (`mesh inbox`),
   - reserve paths (`mesh reserve`) before major edits,
   - release reservations when done (`mesh release`).
5. **Use blackboard** for shared context, blockers, and handoffs (`blackboard post/list/close`).

## Recommended flow

1. `/tak` (default source: ready tasks sorted urgent → oldest) or `/tak work` for loop mode
2. Inspect the selected task (`tak show <id>`)
3. Claim/start (`tak claim` or `tak start <id> --assignee <agent>`)
4. Reserve touched files (`tak mesh reserve --name <agent> --path <path> --reason task-<id>`)
5. Execute work and keep blackboard updated for cross-agent visibility
6. Finish/handoff and release reservations

> Note: In Claude Code (without the pi extension runtime), use the same `/tak work` phrasing to trigger the analogous conversational loop in the `tak-task-execution` skill.

## Mesh etiquette

- Do not silently take over work assigned to another agent.
- If a reservation conflict appears, coordinate first through mesh or blackboard.
- In `/tak work` mode, reserve paths before edits; the extension may block unreserved writes.
- Keep inbox and blackboard messages concise and actionable.
