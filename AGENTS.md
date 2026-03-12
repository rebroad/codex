Do all code edits in `~/src/codex`
Use `~/src/codex.other` for compilations, using `cpto ~/src/codex ~/src/codex.other` to copy the files over first.
Use `/var/tmp` NOT `/tmp` as /tmp has insufficient free space.

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
- Avoid bool or ambiguous `Option` parameters that force callers to write hard-to-read code such as `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other idiomatic Rust API shapes when they keep the callsite self-documenting.
- When you cannot make that API change and still need a small positional-literal callsite in Rust, follow the `argument_comment_lint` convention:
  - Use an exact `/*param_name*/` comment before opaque literal arguments such as `None`, booleans, and numeric literals when passing them by position.
  - Do not add these comments for string or char literals unless the comment adds real clarity; those literals are intentionally exempt from the lint.
  - If you add one of these comments, the parameter name must exactly match the callee signature.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- When writing tests, prefer comparing whole objects with `assert_eq!` when practical instead of field-by-field assertions.
- Do not create small helper methods referenced only once.
- Avoid large modules; prefer new modules once a file grows beyond ~800 LoC (exclude tests).
- When adding/changing API surface, update relevant docs in `docs/`.
- If you add compile-time file reads (for example `include_str!`, `include_bytes!`, `sqlx::migrate!`), update the crate `BUILD.bazel` data attributes so Bazel builds keep working.
- If you change `ConfigToml` or nested config types, ask the user to run (before the tests) `just write-config-schema` (updates `codex-rs/core/config.schema.json`).
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), run `just bazel-lock-update` (repo root) and include `MODULE.bazel.lock` in the same change; then run `just bazel-lock-check`.
- For commands that require escalated permissions, run them directly with escalation; do not ask for
  manual approval in chat (the UI will handle approval prompts).
- For file edits, prefer `apply_patch` first (including files outside workspace/writable roots). If a path
  needs elevated access, request escalation and proceed after approval instead of switching away from
  `apply_patch`.
- Rust commands can wait on lockfiles and appear stuck; be patient and avoid killing them early.
- `codex-rs/Cargo.lock` may already be dirty from normal local Cargo usage. Treat a pre-existing or incidental `Cargo.lock` modification as expected and do not stop work solely for that reason.
- If you did not intentionally change dependencies, do not include `codex-rs/Cargo.lock` in commits.
- For Cargo commands that support it, prefer `--locked` to avoid incidental lockfile rewrites (for example `cargo check --locked`, `cargo build --locked`, `cargo clippy --locked`, `cargo test --locked`).
- Any `git` write operation from Codex (`git add`, `git commit`, `git tag`, `git push`) should be run in an escalated shell.

Testing/formatting:

1. Ask the user to run the crate-specific tests for the project you changed (example: `cargo test -p codex-tui`).
2. If changes touched common, core, or protocol, ask the user before running the full test suite.
3. For large Rust changes, ask user to run `just fix -p <project>` before finalizing.
4. TUI UI changes must update `insta` snapshots (per project-specific workflow).
5. Never hand-edit `.snap` files to satisfy snapshot tests. Run the relevant tests first; if they fail with `.snap.new` output, regenerate/accept snapshots via `cargo insta test -p <crate>` and `cargo insta review` (or `cargo insta accept` when appropriate), then re-run the same tests.
6. DO NOT RUN tests yourself as they create too much output. Tell the user which tests to run, including `just argument-comment-lint` to ensure codebase is clean of comment lint errors.

Before finalizing a large change to `codex-rs`, run `just fix -p <project>` (in `codex-rs` directory) to fix any linter issues in the code. Prefer scoping with `-p` to avoid slow workspace‑wide Clippy builds; only run `just fix` without `-p` if you changed shared crates. Do not re-run tests after running `fix` or `fmt`.

Also run `just argument-comment-lint` to ensure the codebase is clean of comment lint errors.

## The `codex-core` crate

Over time, the `codex-core` crate (defined in `codex-rs/core/`) has become bloated because it is the largest crate, so it is often easier to add something new to `codex-core` rather than refactor out the library code you need so your new code neither takes a dependency on, nor contributes to the size of, `codex-core`.

To that end: **resist adding code to codex-core**!

Particularly when introducing a new concept/feature/API, before adding to `codex-core`, consider whether:

- There is an existing crate other than `codex-core` that is an appropriate place for your new code to live.
- It is time to introduce a new crate to the Cargo workspace for your new functionality. Refactor existing code as necessary to make this happen.

Likewise, when reviewing code, do not hesitate to push back on PRs that would unnecessarily add code to `codex-core`.

## TUI style conventions

See `codex-rs/tui/styles.md`.

TUI and app-server specifics:

- When a change lands in `codex-rs/tui` and `codex-rs/tui_app_server` has a parallel implementation of the same behavior, reflect the change in `codex-rs/tui_app_server` too unless there is a documented reason not to.
- For TUI styling/wrapping, follow file-local conventions and existing helpers.
- Avoid growing high-touch TUI files; extract new functionality instead.
- App-server work is v2-only; keep wire names camelCase (except config RPCs), and keep Rust/TS renames aligned.
