---
name: tak-executing-plans
description: Use when executing an approved implementation plan in a tak-managed repository with batch checkpoints and coordination discipline.
---

# Tak Executing Plans

**REQUIRED BACKGROUND:** superpowers:executing-plans  
**REQUIRED SUB-SKILL:** tak-coordination

Announce at start: **"I'm using the tak-executing-plans skill to implement this plan with tak lifecycle tracking."**

## Step 1: Load and critique the plan

1. Read the plan file completely.
2. Raise risks, gaps, or ambiguities before coding.
3. Confirm each referenced task exists and is ready:
   - `tak show <task-id>`

## Step 2: Coordination preflight (required)

- Check peers and messages:
  - `tak mesh list`
  - `tak mesh inbox --name <agent>`
- Start or claim the next task:
  - `tak start <task-id> --assignee <agent>`
  - or `tak claim --assignee <agent>`
- Reserve touched paths before edits:
  - `tak mesh reserve --name <agent> --path <path> --reason task-<task-id>`

## Step 3: Execute in batches (default 3 tasks)

For each task in the batch:
1. Ensure tak status is `in_progress`.
2. Follow plan steps exactly, including TDD and listed verifications.
3. Capture verification evidence (command + pass/fail outcome).
4. If blocked:
   - `tak blackboard post --from <agent> --task <task-id> --tag blocker --message "<blocker + ask>"`
   - `tak handoff <task-id> --summary "<blocker context + next action>"`
   - stop and ask for guidance.
5. If complete:
   - `tak finish <task-id>`

## Step 4: Report checkpoint

After each batch:
- summarize implemented work
- report verification output
- list task IDs done / in_progress / blocked
- say: **"Ready for feedback."**

## Step 5: Finalize cycle

After all planned tasks are complete:
1. Run full verification suite from the plan.
2. Complete learnings closeout (required):
   - `tak learn add ... --task <task-id>` or
   - `tak learn edit <learning-id> --add-task <task-id>`
   - commit `.tak/learnings/*.json` updates in the same cycle.
3. Release reservations:
   - `tak mesh release --name <agent> --all`
4. Close related blackboard notes.
5. Use superpowers:finishing-a-development-branch for final branch hygiene.

## Stop rules

Stop and ask immediately if instructions are unclear, verification fails repeatedly, or a dependency blocks progress. Do not guess and do not bypass tak state updates.
