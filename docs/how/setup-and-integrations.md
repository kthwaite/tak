# Setup and Integrations (`tak setup`)

This page describes how Tak installs and manages Claude/pi integrations today.

## What this subsystem does

`tak setup` manages three integration surfaces:

1. **Claude hooks** (SessionStart/Stop commands in Claude settings)
2. **Claude assets** (`--plugin`, `--skills`)
3. **pi assets** (`--pi`)

It also supports status checks (`--check`) and removal (`--remove`).

## Source of truth and invariants

Implementation lives in `src/commands/setup.rs`.

Important invariants:

- Install/remove is **idempotent** (safe to rerun).
- Hook updates are additive and preserve unrelated existing hook entries.
- `--pi` manages a **marked block** inside `APPEND_SYSTEM.md`:
  - start marker: `<!-- tak:pi-system:start -->`
  - end marker: `<!-- tak:pi-system:end -->`
- Managed files are compared by trimmed content; mismatches are rewritten to embedded Tak assets.

## Install targets

### Hooks

- Project scope (default): `.claude/settings.local.json`
- Global scope (`--global`): `~/.claude/settings.json`

Hook commands inserted/maintained:

- `tak reindex 2>/dev/null || true`
- `tak mesh join --format minimal >/dev/null 2>/dev/null || true`
- `tak mesh leave --format minimal >/dev/null 2>/dev/null || true`

### Claude assets

- `--plugin` (project-local): `.claude/plugins/tak/...`
- `--skills`:
  - project: `.claude/skills/...`
  - global: `~/.claude/skills/...`

### pi assets (`--pi`)

- project: `.pi/`
- global: `~/.pi/agent/`

Files managed:

- `extensions/tak.ts`
- `skills/tak-coordination/SKILL.md`
- `APPEND_SYSTEM.md` (managed tak block upsert)

## Behavior when data already exists

For `tak setup --pi`:

- If target files already exist and match Tak content (trim-insensitive), they are left unchanged.
- If target files exist but differ, they are overwritten with Tak’s embedded versions.
- In `APPEND_SYSTEM.md`, only the tak marked block is inserted/replaced; content outside the block is preserved.
- If the tak block is already current, no change is made.

This is why rerunning `tak setup --pi` is safe, but edits inside Tak-managed files/blocks are not persistent unless you keep them in sync with Tak’s managed content.

## Hook interaction details

By default, `tak setup` manages hooks. One exception:

- `tak setup --skills` (skills-only mode) does **not** manage hooks.

`tak setup --pi` **does** manage hooks (unless combined in a mode that disables hook management).

## Check and remove semantics

### `tak setup --check`

For each selected integration surface, status is reported as:

- `installed`
- `outdated` (partial/mismatched files or missing managed prompt block)
- `not installed`

Exit status is non-zero if requested components are not fully installed.

### `tak setup --remove --pi`

- Removes managed pi files (`tak.ts`, coordination skill)
- Removes Tak’s marked block from `APPEND_SYSTEM.md`
- Deletes `APPEND_SYSTEM.md` only if empty after block removal
- Cleans up now-empty integration directories (best effort)

## Safety/customization boundaries

Safe:

- Custom content in `APPEND_SYSTEM.md` **outside** Tak markers
- Unrelated files under `.pi/`, `.claude/`, and existing non-Tak hooks

Not safe from overwrite:

- `.pi/extensions/tak.ts`
- `.pi/skills/tak-coordination/SKILL.md`
- Content inside the Tak-managed marker block in `APPEND_SYSTEM.md`

## Code pointers

- `src/commands/setup.rs`
  - `write_pi_files()`
  - `upsert_pi_append_system()` / `remove_pi_append_system()`
  - `check_pi_installed_at()`
  - `hooks_requested()`
  - `run_install()` / `run_remove()` / `run_check()`

## Test pointers

In `src/commands/setup.rs` tests:

- `marked_block_upsert_and_remove_are_idempotent`
- `check_pi_installed_at_reports_states`
- `hooks_requested_is_false_for_skills_only_mode`
