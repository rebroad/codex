In the `codex-rs` folder where the Rust code lives:

- Install required tooling before running project commands (for example `just`, `rg`, `cargo-insta`) if missing.
- Never add or modify code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR`.
  - `CODEX_SANDBOX_NETWORK_DISABLED=1` is set when using the shell tool; existing checks are intentional.
  - `CODEX_SANDBOX=seatbelt` is set for processes spawned under Seatbelt; some tests intentionally exit early for this.
- If you add compile-time file reads (for example `include_str!`, `include_bytes!`, `sqlx::migrate!`), update the crate `BUILD.bazel` `data` attributes so Bazel builds continue to work.
- Prefer private modules and explicitly exported public crate API.
- If you change `ConfigToml` or nested config types, run `just write-config-schema` before tests to keep `codex-rs/core/config.schema.json` in sync.
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), run `just bazel-lock-update` at repo root, include `MODULE.bazel.lock` in the same change, then run `just bazel-lock-check`.
- Escalated shell access may be needed for cargo (due to sccache).
- Prefer end-to-end verification with `./scripts/rebuild_codex.sh` from the designated build tree over localized checks when validating final build/run readiness.
- `codex-rs/Cargo.lock` may already be dirty from normal local Cargo usage; do not treat that alone as a blocker.
- When comparing local work against upstream, `git-catchup --print-upstream-equivalent` can print the upstream-equivalent commit hash to diff against directly.

## graphify

This project keeps its knowledge graph under codex.build/codex-rs/graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, invoke the `skill` tool with `skill: "graphify"` before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when codex.build/codex-rs/graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If codex.build/codex-rs/graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read codex.build/codex-rs/graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
