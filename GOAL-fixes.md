# Goals

- Use `gpt-6-luna` as the default model for generated conversation titles.
- Add and verify a `thread_title_model` override in `config.toml` and its schema.
- Run focused config and TUI tests, then commit the verified change.
- Investigate why PID 3787974, a long-running ChatGPT desktop app-server, did not
  restart after the 09:39 `build_codex.sh` run; identify which process the build
  script's idle restart targets and record the verified cause.
- Determine and implement a supported launch path so the ChatGPT app's forked
  `codex` process is a stable update wrapper capable of replacing the actual
  app-server process when requested.

Initial process evidence (2026-10-10): PID 3787974 is a child of the ChatGPT
desktop process and runs `/usr/lib/chatgpt/resources/codex ... app-server`. Its
executable resolves to a dated CLI binary from 2026-10-09. The desktop process
has `CODEX_APP_SERVER_USE_LOCAL_DAEMON=1` and owns the default app-server
control socket, but there is no PID record under `~/.codex/app-server-daemon/`.
`codex app-server daemon version` reports the CLI binary installed at 09:39 on
2026-10-10 and says the socket is running. However, an actual
`codex app-server daemon restart-if-idle` attempt returns
`app server is running but is not managed by codex app-server daemon`. The
restart command correctly refuses to replace this ChatGPT-owned child because
it does not own a managed PID record. `build_codex.sh` invokes that command
after installation, so its restart request cannot restart PID 3787974. The
installed ChatGPT app bundle's launcher code supports local-daemon selection
and otherwise falls back to spawning a stdio `codex app-server` process.
