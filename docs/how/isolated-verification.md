# Isolated Verification in Multi-Agent Lanes

Use this playbook when you need reliable test evidence but the shared working tree is temporarily noisy (for example: unrelated in-progress edits make `cargo test` fail before your target tests run).

Scoped verification now supports explicit scope paths:

```bash
tak verify <task-id> --path src/commands --path tests/verify_scope_integration.rs
```

When an explicit scope overlaps a peer reservation, `tak verify` fails with actionable diagnostics (owner/path/reason/age) and remediation hints such as:

- `tak mesh blockers --path <held-path>`
- `tak wait --path <held-path> --timeout 120`
- `tak mesh reserve --name <agent> --path <scope-path> --reason task-<task-id>`

Use these diagnostics first; fall back to isolated worktrees only when overlap/noise cannot be resolved quickly.

## When to use this

Default behavior is still: **verify in the main working tree**.

Use an isolated `git worktree` only when all are true:

1. You need verification evidence now for a narrow change.
2. The shared tree is failing for unrelated reasons.
3. You can keep the verification scope targeted and reproducible.

## Coordination requirements

Before running isolated verification:

- Post a short blackboard status/blocker note explaining why shared-tree verification is noisy.
- Keep file reservations focused on actual edited paths (not the temporary worktree directory).
- Record exact commands + outcomes in task context/log/handoff notes.

After verification:

- Mention that results came from an isolated worktree.
- Remove the temporary worktree.
- Close/update blocker notes once the lane is unblocked.

## Recommended flow

```bash
# 1) Create temporary detached worktree from current HEAD
WT_DIR=$(mktemp -d /tmp/tak-verify-XXXX)
git worktree add --detach "$WT_DIR" HEAD

# 2) Apply only your candidate diff for touched files
# (replace paths with your actual edited files)
git diff -- path/to/file1 path/to/file2 | (cd "$WT_DIR" && git apply -)

# 3) Run targeted verification in the isolated tree
(cd "$WT_DIR" && cargo test test_name --quiet)

# 4) Clean up
git worktree remove "$WT_DIR" --force
```

## Guardrails

- Keep this as a **verification fallback**, not a default workflow.
- Prefer targeted tests (or a focused command subset) over full-suite runs when isolating.
- Never treat isolated verification as a substitute for eventual shared-tree stability.
- Do not mutate `.tak/*` files directly; continue using `tak` CLI lifecycle/coordination commands.
