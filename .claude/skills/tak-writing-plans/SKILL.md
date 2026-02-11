---
name: tak-writing-plans
description: Use when converting an approved design into an executable, tak-tracked implementation plan before coding.
---

# Tak Writing Plans

**REQUIRED BACKGROUND:** superpowers:writing-plans  
**REQUIRED SUB-SKILL:** tak-coordination

Announce at start: **"I'm using the tak-writing-plans skill to produce a tak-tracked implementation plan."**

## Step 0: Coordination + planning task setup

1. Check mesh presence and inbox:
   - `tak mesh list`
   - `tak mesh inbox --name <agent>`
2. Start the planning task:
   - `tak start <task-id> --assignee <agent>`
3. Reserve planning artifacts before writing:
   - `tak mesh reserve --name <agent> --path docs/plans --reason task-<task-id>`
4. If no planning task exists, create one and link it to the relevant epic/feature before writing.

## Plan file

Save to `docs/plans/YYYY-MM-DD-<feature-name>.md`.

Every plan should start with:

```markdown
# [Feature Name] Implementation Plan

> **For Claude/pi:** REQUIRED SUB-SKILL: Use `tak-executing-plans` to implement this plan task-by-task.

**Tak Root Task:** `<task-id>`
**Goal:** [One sentence]
**Architecture:** [2-3 sentence approach]
**Tech Stack:** [Key tools/libraries]

---
```

## Build plan and tak graph together

For each component, include:
- exact file paths (create/modify/test)
- 2-5 minute TDD micro-steps
- exact verification commands with expected outcomes
- a commit step
- linked tak task ID

When IDs do not exist yet, create them during planning:
- `tak create "<title>" --parent <root-task-id> --priority <priority> --estimate <estimate>`
- `tak depend <task-id> --on <dependency-id>`

## Recommended per-task template

```markdown
### Task N: [Component Name]

**Tak Task ID:** `<task-id>`
**Files:**
- Create: `exact/path`
- Modify: `exact/path`
- Test: `tests/exact/path`

**Step 1:** Write failing test
**Step 2:** Run to confirm failure
**Step 3:** Implement minimal fix
**Step 4:** Run tests to confirm pass
**Step 5:** Commit

**Verification:** `exact command(s)`
**Lifecycle Notes:** start -> reserve paths -> finish/handoff
```

## Close planning cycle

1. Re-read plan for DRY, YAGNI, and TDD fidelity.
2. Post a blackboard note with doc path and linked task IDs.
3. Release reservations:
   - `tak mesh release --name <agent> --path docs/plans`
4. Offer execution mode:
   - in-session subagent flow, or
   - separate execution session with `tak-executing-plans`.
