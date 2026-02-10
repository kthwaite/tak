# Pi Extension JSON Contract for CoordinationDb Parity

This page defines the JSON payload contract consumed by `pi-plugin/extensions/tak.ts` for `/tak` coordination features.

## Goal

Keep the pi extension aligned with current CLI output when coordination data comes from `.tak/runtime/coordination.db`.

The extension should parse these payloads via coercers and avoid raw field assumptions.

## Command payload mapping

| Command | Current JSON fields (CoordinationDb-backed CLI) | Extension adapter rule | Legacy fallback (temporary) |
|---|---|---|---|
| `tak mesh list` | `name`, `generation`, `session_id`, `cwd`, `pid`, `host`, `status`, `started_at`, `updated_at`, `metadata` | Parse `name/session_id/cwd/status/pid`; ignore extra fields safely | none needed |
| `tak mesh inbox --name <agent>` | `id`, `from_agent`, `to_agent`, `text`, `reply_to`, `created_at`, `read_at`, `acked_at` | Map sender from `from_agent`, timestamp from `created_at` | `from`, `to`, `timestamp` |
| `tak mesh feed` | `id`, `agent`, `event_type`, `target`, `preview`, `detail`, `created_at` | Render type from `event_type`, timestamp from `created_at` | `type`, `ts` |
| `tak mesh blockers` | `owner`, `path`, `reason?`, `age_secs` | Keep as-is (`owner/path/reason/age_secs`) | none needed |
| `tak mesh reservations` *(new for parity guards)* | `agent`, `path`, `reason?`, `created_at`, `expires_at`, `age_secs` | Use this as guard input for write/verify overlap checks | file read of `.tak/runtime/mesh/reservations.json` (to be removed) |
| `tak blackboard list/show/post` | `id`, `from_agent`, `message`, `status`, `tags`, `task_ids`, `created_at`, `updated_at`, `closed_by`, `closed_reason`, `closed_at` | Map note author from `from_agent`; render `updated_at` | `author` |
| `tak work start/status/stop` | top-level `event`, `agent`, `loop`, `current_task`, `reservations`, `blockers`, `suggested_action`, optional `done`; loop includes `active`, `current_task_id`, `tag`, `remaining`, `processed`, `verify_mode`, `claim_strategy`, etc. | Keep parsing `loop` fields with snake_case keys; canonicalize `current_task_id` through task-id coercion | none needed |

## Contract rules

1. **Prefer current field names first** (CoordinationDb shape), then fallback for legacy names when needed.
2. **Treat unknown fields as additive**, not breaking.
3. **Never parse coordination runtime files directly** for extension logic once parity tasks land.
4. **Fail safe for guards**: if reservation snapshot is unavailable during strict/isolated modes, block with actionable guidance.

## Code pointers

- Extension parser/adapter: `pi-plugin/extensions/tak.ts`
- Mesh command output: `src/commands/mesh.rs`
- Blackboard command output: `src/commands/blackboard.rs`
- Work-loop response output: `src/commands/work.rs`
- Coordination DB model types: `src/store/coordination_db.rs`

## Verification

- `cargo build`
