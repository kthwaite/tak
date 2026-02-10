# tak pi integration

Pi extension + skill bundle for tak.

## What it adds

- `/tak` command with task picking and filtering
  - default source: `ready`
  - ordering: **urgent first, oldest first**
- `/tak work` loop mode
  - auto-claims next task, tracks current task, and auto-claims again after finish/handoff/cancel
  - optional filters: `tag:<tag>`, `limit:<n>`, `verify:isolated|local`, `cue:auto|editor` (or shorthand `auto`)
- `tak_cli` tool for structured tak command execution
- Mesh + blackboard aware workflows (`/tak mesh`, `/tak inbox`, `/tak blackboard`)
- Automatic session behavior in tak repos:
  - `tak reindex` on session start
  - `tak mesh` stale self-registration cleanup on session start (reload safety)
  - `tak mesh join --session-id <pi-session-id>` on session start (best effort)
  - `tak mesh leave` on session shutdown (best effort)
- Per-turn system prompt injection reinforcing active tak usage + agent coordination
- Coordination runtime data is sourced through CLI snapshots backed by `.tak/runtime/coordination.db` (no direct runtime file reads)
- Write/edit reservation guard: blocks edits to files reserved by other mesh agents
- Work-mode strict reservation guard: blocks write/edit unless your agent reserved the path
- Work-mode verify guard (`verify:isolated`): blocks local build/test/check on overlapping reservation scope
- Fail-safe behavior: if reservation snapshots cannot be loaded, guarded write/verify paths block with remediation guidance

Legacy `.tak/runtime/mesh/` and `.tak/runtime/blackboard/` directories are no longer the source of truth after CoordinationDb migration; use `tak doctor --fix` to clean stale runtime folders.

## Install (project-local)

Preferred (from tak repo root):

```bash
tak setup --pi
```

Manual package install:

```bash
pi install ./pi-plugin -l
```

Then start pi in the repo and use:

```text
/tak
```

## `/tak` quick examples

```text
/tak
/tak blocked
/tak mine
/tak priority:critical
/tak blackboard tag:handoff
/tak inbox
/tak inbox ack
/tak claim
/tak work
/tak work auto
/tak work tag:backend limit:2 verify:isolated cue:auto
/tak work status
/tak work stop
/tak mesh
/tak 42
```
