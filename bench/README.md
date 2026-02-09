# takbench (draft)

`takbench` is a tmux-based benchmark harness for evaluating both code outcomes and `tak` workflow behavior.

## Quick start (uv)

```bash
# Install/sync Python deps (pytest)
uv sync --project bench

# Example invocation (adjust worker command + hidden tests)
uv run --project bench python bench/takbench.py \
  --objective bench/objectives/markdown_parser_v1 \
  --worker-cmd "your-agent-cli" \
  --hidden-test-cmd "python -m pytest -q /abs/path/to/hidden_tests/test_hidden_markdown.py"
```

## Key requirements

- `tmux` must be installed
- `tak` must be on PATH
- `uv` should be installed for dependency/runtime management
- worker command must launch an interactive agent in the tmux pane

By default, `takbench` enforces uv runtime usage. To bypass (not recommended), pass `--allow-non-uv`.

## Batch execution

Run repeated trials and aggregate diagnostics:

```bash
uv run --project bench python bench/takbench_batch.py \
  --objective bench/objectives/markdown_parser_v1 \
  --worker-cmd "your-agent-cli" \
  --hidden-test-cmd "python -m pytest -q /abs/path/to/hidden_tests/test_hidden_markdown.py" \
  --count 10
```

Batch outputs are written under:

- `bench/runs/batches/<batch_id>/summary.json`
- `bench/runs/batches/<batch_id>/summary.md`
- `bench/runs/batches/<batch_id>/runs.jsonl`

## Output

Per-run artifacts are written to `bench/runs/<run_id>/` including:

- full pane log (`logs/worker.log`)
- injected command log (`logs/commands.jsonl`)
- tmux metadata (`tmux_meta.json`)
- scored report (`report.json`)
