# Storage and Index Model

This page describes Tak’s current persistence model and index rebuild behavior.

## What this subsystem does

Tak uses a hybrid model:

- **File store** (`.tak/tasks/*.json`) is the durable source of truth for tasks.
- **SQLite index** (`.tak/index.db`) is a derived query index for fast listing/filtering/cycle checks.

Related stores:

- sidecars under `.tak/context/`, `.tak/history/`, `.tak/verification_results/`, `.tak/artifacts/`
- learnings under `.tak/learnings/*.json`

## Source of truth and invariants

- Task and learning JSON files are authoritative.
- The SQLite DB is disposable/rebuildable.
- `Repo::open()` ensures index freshness (missing/stale schema/fingerprint mismatch => rebuild).
- Index schema enforces FK checks and uses TEXT task IDs in SQL tables.

## File store behavior (`src/store/files.rs`)

### Task file naming

- Canonical filename is derived from `TaskId::from(task.id)` and written as `<16-hex>.json`.
- Reads/writes support a legacy numeric fallback (`<id>.json`) for compatibility.

### ID allocation

- New IDs are allocated as `max(existing_ids) + 1` under `.tak/counter.lock`.
- Allocation no longer depends on `counter.json` contents.

### Validation and writes

- `create()` validates parent/dependency existence before writing.
- `write()` updates an existing canonical or legacy path; if none exists, it writes canonical.
- `list_all()` sorts tasks by `created_at`, then `id`.

### Fingerprint

`fingerprint()` computes a metadata fingerprint from task files:

- filename
- size
- nanosecond mtime

This catches adds/deletes/in-place edits without reading full file contents.

## SQLite index behavior (`src/store/index.rs`)

### Open/setup

- `Index::open()` enables `WAL` mode and `foreign_keys=ON`.
- Tables include tasks, dependencies, tags, skills, learnings, metadata, and learnings FTS.

### Rebuild strategy

`rebuild(tasks)` is transactional and two-pass:

1. Insert tasks with `parent_id = NULL`
2. Update `parent_id`, insert dependencies/tags/skills

This avoids FK failures when parents/dependencies appear later in file order.

### Upsert strategy

`upsert(task)` is transactional:

- `INSERT OR REPLACE` task row
- clear/reinsert dependencies
- clear/reinsert tags
- clear/reinsert skills

### Query-time graph logic

- `available()` computes blocked-ness dynamically (dependency statuses), with priority-aware ordering.
- `would_cycle()` and `would_parent_cycle()` use recursive CTEs.

## Repo open + stale detection (`src/store/repo.rs`)

`Repo::open()` performs freshness checks in this order:

1. Open file store/index
2. If index missing, mark for rebuild
3. If index schema is not TEXT-ID compatible, recreate DB and rebuild
4. Compare stored fingerprint vs current file fingerprint
5. Rebuild index when needed and persist fresh fingerprint
6. Rebuild learning index when learning fingerprint changes

So a fresh clone or deleted/stale `index.db` self-heals on open.

## Operator-facing implications

- Manual `tak reindex` is available, but normal command usage auto-heals stale index state.
- Editing task JSON outside CLI is supported, but index refresh occurs on next repo open/check flow.
- Index corruption/loss is recoverable as long as `.tak/tasks/*.json` files remain intact.

## Code pointers

- `src/store/files.rs`
- `src/store/index.rs`
- `src/store/repo.rs`

## Test pointers

Representative tests in `src/store/index.rs` and `src/store/files.rs`:

- `stale_index_detected_after_file_change`
- `stale_index_detected_after_in_place_edit`
- `rebuild_with_forward_pointing_deps`
- `rebuild_with_forward_pointing_parent`
- `rebuild_tolerates_duplicate_deps_and_tags`
- `read_supports_legacy_numeric_filename_fallback`
- `fingerprint_includes_non_numeric_filenames`
