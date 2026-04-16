In the `codex-rs` folder where the Rust code lives:

- Install required tooling before running project commands (for example `just`, `rg`, `cargo-insta`) if missing.
- Never add or modify code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR`.
  - `CODEX_SANDBOX_NETWORK_DISABLED=1` is set when using the shell tool; existing checks are intentional.
  - `CODEX_SANDBOX=seatbelt` is set for processes spawned under Seatbelt; some tests intentionally exit early for this.
- If you add compile-time file reads (for example `include_str!`, `include_bytes!`, `sqlx::migrate!`), update the crate `BUILD.bazel` `data` attributes so Bazel builds continue to work.
- If you change `ConfigToml` or nested config types, run `just write-config-schema` before tests to keep `codex-rs/core/config.schema.json` in sync.
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), run `just bazel-lock-update` at repo root, include `MODULE.bazel.lock` in the same change, then run `just bazel-lock-check`.
- For Cargo commands that support it, prefer `--locked` (for example `cargo build --locked`, `cargo clippy --locked`, `cargo test --locked`) to avoid incidental lockfile rewrites.
- Prefer end-to-end verification with `./scripts/rebuild_codex.sh` from the designated build tree over localized checks when validating final build/run readiness.
- `codex-rs/Cargo.lock` may already be dirty from normal local Cargo usage; do not treat that alone as a blocker.
