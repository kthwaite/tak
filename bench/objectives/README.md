# Objective packs

Each objective pack is self-contained and includes:

- `objective.toml` — manifest/config
  - `[paths]` for prompt/template locations
  - `[worker]` for puppeteer↔puppet protocol (ready strategy, done tokens, prompt transport)
  - `[change]` for mid-run change timing
  - `[scoring]` weights/penalties
- `prompts/initial.txt` — initial worker prompt
- `prompts/change.txt` — mid-run requirement update prompt
- `template/` — files copied into fresh benchmark repo

## Current packs

- `markdown_parser_v1`
