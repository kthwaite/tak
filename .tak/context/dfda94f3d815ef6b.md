Implemented extraction of reusable internals into `tak-core` with minimal semantic changes:

- Copied former root library modules from `src/` into `crates/tak-core/src/` (commands/store/model/metrics/error/output/etc).
- Updated `crates/tak-core/Cargo.toml` with full dependency set needed by extracted modules.
- Adjusted embedded asset include paths in `crates/tak-core/src/commands/setup.rs` to account for new crate depth (`../../../../...`).
- Converted root `src/lib.rs` into a thin re-export shim (`pub use tak_core::*;`).
- Added root dependency on `tak-core` in `Cargo.toml`.
- Removed now-redundant copied module files from root `src/` so functional internals are owned by `tak-core`.

Validation run:
- `cargo test -p tak-core --no-run`
- `cargo test -p tak --no-run`
- `cargo test -p tak-cli --no-run`
- `cargo test --workspace --no-run`

Result: all no-run compile checks pass after extraction.