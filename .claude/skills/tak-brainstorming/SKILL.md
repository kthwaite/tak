---
name: tak-brainstorming
description: Use when shaping a new feature, behavior change, or design in a tak-managed repository before implementation begins.
---

# Tak Brainstorming

**REQUIRED BACKGROUND:** superpowers:brainstorming  
**REQUIRED SUB-SKILL:** tak-coordination (for task lifecycle + mesh etiquette)

## Overview

Turn rough ideas into validated designs while keeping tak state and coordination accurate from the start.

Announce at start: **"I'm using the tak-brainstorming skill to shape this design with tak coordination."**

## Step 0: Tak coordination preflight

1. Check active peers and inbox:
   - `tak mesh list`
   - `tak mesh inbox --name <agent>`
2. Select or create a design task:
   - Existing: `tak start <task-id> --assignee <agent>`
   - New: `tak create "<title>" --kind task --priority <priority> --tag design --tag planning`
3. If peers are active, post a heads-up note:
   - `tak blackboard post --from <agent> --task <task-id> --tag design --message "<scope + expected files>"`
4. Reserve paths before major edits:
   - `tak mesh reserve --name <agent> --path <path> --reason task-<task-id>`

## Brainstorming process

- Check current project context first (files, docs, recent commits, open tasks).
- Ask **one question per message** to refine purpose, constraints, and success criteria.
- Prefer multiple-choice questions when possible.
- Propose 2-3 approaches with trade-offs; lead with your recommendation and why.
- Keep YAGNI discipline.

## Presenting the design

When confident in scope:
- Present design in 200-300 word sections.
- After each section ask: **"Does this look right so far?"**
- Cover architecture, components, data flow, error handling, and testing.
- If feedback changes direction, loop back and clarify before continuing.

## Convert design into tak-ready execution input

After design approval:
1. Save design doc to `docs/plans/YYYY-MM-DD-<topic>-design.md`.
2. Break implementation into concrete tak tasks (usually children of the design task):
   - `tak create "<task title>" --parent <design-task-id> --priority <priority> --tag implementation`
3. Add dependency ordering:
   - `tak depend <task-id> --on <dependency-id>`
4. Post a blackboard handoff note with plan/doc path and task graph summary.

## Completion handoff

Ask: **"Ready to move into implementation planning?"**  
If yes, use `tak-writing-plans`.
