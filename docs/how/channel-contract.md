# Channel Contract: Mesh vs Blackboard vs Task Context/History

This page documents **current behavior** for where coordination information should live so agents avoid duplicate/noisy updates.

## What this contract does

It defines channel roles across three surfaces:

1. `tak mesh` (fast operational signal)
2. `tak blackboard` (durable shared team state)
3. task-local sidecars (`tak context`, `tak log`) for deep implementation detail

Use this contract to decide **where to post what**, especially during blockers/handoffs.

## Source of truth and invariants

- Task lifecycle truth stays in `.tak/tasks/*.json` (status/assignee/dependencies).
- Coordination channels do **not** replace task truth; they provide shared execution context.
- Mesh is ephemeral/runtime-oriented (`.tak/runtime/mesh/`, gitignored).
- Blackboard is durable shared coordination state (`.tak/runtime/blackboard/`, gitignored runtime but durable per repo run context).
- Context/history sidecars are task-local, git-committed (`.tak/context/`, `.tak/history/`).

## Channel role matrix

| Channel | Primary role | Message style | Retention | Put this here | Avoid this here |
|---|---|---|---|---|---|
| Mesh (`tak mesh send/broadcast/inbox`) | Immediate operational ask/ack | Short, action-oriented | Ephemeral runtime | reservation release asks, quick ownership pings, immediate unblock requests | long narrative, repeated deep logs |
| Blackboard (`tak blackboard post/list/...`) | Durable cross-agent coordination record | Structured status/blocker/handoff note | Durable board history | blocker records, handoff records, shared next-owner/action decisions | full command logs, long investigative dead-ends |
| Task context/history (`tak context`, `tak log`) | Task-local technical detail + lifecycle trail | Detailed implementation notes | Git-tracked sidecars | repro details, failure output, dead ends, local debugging detail | team-wide ping/ask loops |

## Recommended flow

1. **Fast ping in mesh** when immediate coordination is needed.
2. **Durable note in blackboard** for team-visible blocker/handoff/status.
3. **Deep detail in context/history** for task-specific evidence.
4. Cross-reference IDs/messages in note text when useful.

## Concrete examples

### Mesh (short operational ping)

```bash
tak mesh send --from agent-1 --to agent-2 \
  --message "Task #49 blocked by src/store/mesh.rs reservation; can you release or handoff?"
```

### Blackboard (durable shared state)

```bash
tak blackboard post --from agent-1 --template blocker --task 49 \
  --message "Blocked >10m on reservation conflict; requesting owner release or explicit handoff."
```

Delta follow-up (avoid repeating unchanged context):

```bash
tak blackboard post --from agent-1 --template status --task 49 \
  --since-note 120 --no-change-since --message "Still waiting on owner response; fallback planned at 15:30Z."
```

### Task-local context/history (deep detail)

```bash
tak context 49 --set "Investigated overlap paths: src/store/mesh.rs + src/store/paths.rs; failing check reproduced with ..."
tak log 49
```

## Structured blackboard note shape (current)

Template-backed notes are serialized as line-based `key: value` fields. Required fields for medium/high-signal templates include:

- `template`
- `summary`
- `status`
- `scope`
- `owner`
- `verification`
- `blocker`
- `next`

`blocker` template additionally includes `requested_action`.

Current behavior includes non-blocking schema warnings when template-required fields still use placeholder/unset values.

## Edge cases and operator expectations

- `tak blackboard post --task <id>` validates task existence and fails on missing IDs.
- `--no-change-since` requires `--since-note`.
- Free-text mode remains valid: omit `--template` for unstructured notes.
- Sensitive-text detection in blackboard posts warns but does not block posting.

## Code pointers

- `src/commands/mesh.rs` — runtime messaging/reservations/feed handlers
- `src/commands/blackboard.rs` — template rendering, delta metadata, schema/sensitive warnings
- `src/commands/context.rs` — task context sidecar read/write
- `src/commands/log.rs` — task history log display
- `src/store/sidecars.rs` — context/history persistence

## Test pointers

- `tests/integration.rs`
  - `test_blackboard_post_close_reopen`
  - `test_blackboard_post_blocker_template_formats_message_and_tags`
  - `test_blackboard_post_plain_message_remains_free_text`
- `src/commands/blackboard.rs`
  - template serialization and schema-warning unit tests
