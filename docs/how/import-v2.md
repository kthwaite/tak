# Import v2: strict YAML plan materialization

`tak import` materializes a full planning document (epic + features + tasks + dependency graph + metadata) in one command.

## 1) What this subsystem does

- Parses one canonical YAML schema:
  - top-level `epic` (required)
  - `features` array (required, non-empty)
  - each feature has `tasks` (optional)
- Infers hierarchy from nesting:
  - epic -> feature -> task
- Resolves symbolic dependency references across the whole document.
- Supports `--dry-run` preview output (JSON/pretty/minimal).
- Applies imports with rollback compensation if any write step fails.

## 2) Source of truth + invariants

- Task files under `.tak/tasks/*.json` remain the source of truth.
- Import-v2 invariants:
  - unknown YAML fields are rejected (`deny_unknown_fields`),
  - legacy payload shapes are rejected (breaking change),
  - self-parent/self-dependency is rejected,
  - cycles across parent/dependency edges are rejected,
  - duplicate aliases are rejected,
  - ambiguous title-based symbolic refs are rejected.

## 3) Execution flow (happy path)

1. Parse YAML into strict v2 structs (`ImportPlanSpec`, `ImportFeatureSpec`, `ImportLeafTaskSpec`).
2. Normalize into flat internal specs with deterministic indexes.
3. Resolve references:
   - scalar refs: alias first, then existing task ID/prefix,
   - mapping refs: prefer `alias`, then `title`/`epic`.
4. Build deterministic creation order (topological sort over parent + local deps).
5. If `--dry-run`: render preview only.
6. If apply:
   - create task files,
   - apply traceability updates,
   - upsert index rows,
   - on failure, rollback created rows/files in reverse order.

## 4) Edge cases / idempotency / failure behavior

- Import is not idempotent by default (re-running creates new task IDs).
- If apply fails after partial writes, importer performs rollback cleanup:
  - removes created index entries,
  - removes created task files,
  - returns original error (or combined apply+rollback error if cleanup fails).
- YAML anchors/aliases work naturally as symbolic refs when they resolve to alias/title-bearing mappings.
- Title-based symbolic refs must be unique within the document.

## 5) Canonical schema example

```yaml
epic: Agentic Chat
description: Claude Code for writers
tags: [agentic-chat]
priority: high

features:
  - &infra
    alias: infra
    title: Tool Infrastructure
    tasks:
      - &schemas
        alias: schemas
        title: Define tool schemas
        estimate: m
      - title: Add agentMode flag
        depends_on: [*schemas]

  - &read
    alias: read
    title: Read Tools
    depends_on: [*infra]
    tasks:
      - title: Wire read handlers

  - title: Write Tools
    depends_on: [*read]
    tasks:
      - &approval
        alias: approval
        title: Build approval gate
      - title: Wire edit_scene diff view
        depends_on: [*approval, *schemas]
```

## 6) Breaking-change migration notes

Legacy import shapes were removed. In particular:

- old top-level `tasks:` list payloads are no longer accepted,
- old JSON wrapper/list formats are no longer accepted,
- old compatibility aliases (`key`, `ref`, `depends-on` variants, etc.) are no longer accepted.

Migration guidance:

1. Move to `epic` + `features` + `tasks` nesting.
2. Add explicit `alias` values where you need stable symbolic references.
3. Prefer `tak import plan.yaml --dry-run` to validate before apply.

## 7) Code pointers

- `src/commands/import.rs`
  - schema types + strict parse
  - flatten/resolve/order pipeline
  - dry-run preview rendering
  - rollback-backed apply

## 8) Test pointers

- Unit tests in `src/commands/import.rs`
  - strict schema validation
  - symbolic dependency resolution
  - metadata fidelity
  - rollback behavior
- Integration tests:
  - `tests/import_dry_run_output_integration.rs`
