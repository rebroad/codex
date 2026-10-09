RESOLVED: `DOWNSTREAM.md` explains that `760ede5a9f` adds `rollout_uncompressed_size` and does not remove `read_rollout_lines`; the later change scopes `read_rollout_lines` to tests because production uses the incremental `open_rollout_line_reader` API. The production size API remains exported and used by rollout recording and thread-store code.

IN PROGRESS: Prevent project `AGENTS.md` instructions from entering model prompts used to generate conversation-title summaries. The title-generation session now skips loading AGENTS.md context; verify this with the focused and full core test suites.

IN PROGRESS: Default Rust builds and tests to `CARGO_INCREMENTAL=0` with sccache enabled through `scripts/codex_cargo_env.sh`; verify cache statistics show the real builds/tests are using sccache.

TODO: Find where Codex selects `gpt-5.6-terra` and update that use to a more recent supported model, either `gpt-6-luna` or `gpt-6.1-sol`.

TODO: `codex app-server reload` isn't working (it fails with `Error: timed out waiting for config/reload response` and `Caused by: deadline has elapsed`). Fix this.

