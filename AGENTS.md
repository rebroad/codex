# Rust/codex-rs

In the codex-rs folder where the rust code lives:

- Crate names are prefixed with `codex-` (example: the `core` folder’s crate is `codex-core`).
- When using `format!` and you can inline variables into `{}`, always do that.
- Install any commands the repo relies on (for example `just`, `rg`, or `cargo-insta`) if they aren't already available before running instructions here.
- Never add or modify any code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR`.
  - `CODEX_SANDBOX_NETWORK_DISABLED=1` is set when using the shell tool; existing checks are intentional.
  - `CODEX_SANDBOX=seatbelt` is set for processes spawned under Seatbelt; some tests exit early for this.
- Always collapse if statements per https://rust-lang.github.io/rust-clippy/master/index.html#collapsible_if
- Always inline format! args when possible per https://rust-lang.github.io/rust-clippy/master/index.html#uninlined_format_args
- Use method references over closures when possible per https://rust-lang.github.io/rust-clippy/master/index.html#redundant_closure_for_method_calls
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Do not create small helper methods referenced only once.
- Avoid large modules; prefer new modules once a file grows beyond ~800 LoC (exclude tests).
- When adding/changing API surface, update relevant docs in `docs/`.
- If you change `ConfigToml` or nested config types, run `just write-config-schema` (updates `codex-rs/core/config.schema.json`).
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), run `just bazel-lock-update` (repo root) and include `MODULE.bazel.lock` in the same change; then run `just bazel-lock-check`.

Testing/formatting:

1. Run `just fmt` (in `codex-rs`) after Rust code changes; do not ask for approval.
2. Ask the user to run the crate-specific tests for the project you changed (example: `cargo test -p codex-tui`).
3. If changes touched common, core, or protocol, ask the user before running the full test suite.
4. For large Rust changes, run `just fix -p <project>` before finalizing (do not re-run tests after `fix` or `fmt`).
5. TUI UI changes must update `insta` snapshots (per project-specific workflow).

TUI and app-server specifics:

- For TUI styling/wrapping, follow file-local conventions and existing helpers.
- Avoid growing high-touch TUI files; extract new functionality instead.
- App-server work is v2-only; keep wire names camelCase (except config RPCs), and keep Rust/TS renames aligned.
