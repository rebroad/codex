# Goals
- Use `gpt-6-luna` as the default model for generated conversation titles.
- Add and verify a `thread_title_model` override in `config.toml` and its schema.
- Run focused config and TUI tests, then commit the verified change.
- Investigate why PID 3787974, a long-running ChatGPT desktop app-server, did not restart after the 09:39 `build_codex.sh` run; identify which process the build script's idle restart targets and record the verified cause.
- Use an update-wrapper from the ChatGPT desktop launcher to start the managed app-server before the app, so ChatGPT connects to the replaceable daemon instead of forking an unmanaged `codex app-server` process.
- Get codex CLI/TUI working with Termux so that a user tapping within the prompt/editable area causes the virtual keyboard to be displayed (on systems where there is no physical keyboard, such as on the flip7). Test this works using flip7 (using the virtual display, don't interfere with the user's display).

Initial process evidence (2026-10-10): PID 3787974 is a child of the ChatGPT desktop process and runs `/usr/lib/chatgpt/resources/codex ... app-server`. Its executable resolves to a dated CLI binary from 2026-10-09. The desktop process has `CODEX_APP_SERVER_USE_LOCAL_DAEMON=1` and owns the default app-server control socket, but there is no PID record under `~/.codex/app-server-daemon/`.  `codex app-server daemon version` reports the CLI binary installed at 09:39 on 2026-10-10 and says the socket is running. However, an actual `codex app-server daemon restart-if-idle` attempt returns `app server is running but is not managed by codex app-server daemon`. The restart command correctly refuses to replace this ChatGPT-owned child because it does not own a managed PID record. `build_codex.sh` invokes that command after installation, so its restart request cannot restart PID 3787974. The installed ChatGPT app bundle's launcher code supports local-daemon selection and otherwise falls back to spawning a stdio `codex app-server` process.

The user-level desktop entry now launches `scripts/codex-app-server-update-wrapper`. On a cold start, it starts the installed `~/.cargo/bin/codex` daemon first and requires the start response to identify a managed PID backend before launching ChatGPT with local-daemon mode enabled. Existing ChatGPT instances keep handling URLs without another daemon start. This configures future cold starts; the currently running desktop app must be fully quit and relaunched before the new path takes effect.

Validation for the current goal:

- `just test -p codex-config thread_title_model_can_be_configured`: 1 passed.
- `just test -p codex-core load_config_applies_thread_title_model_override`:
  1 passed.
- `just test -p codex-tui thread_title`: 30 passed, including the `gpt-6-luna`
  default and a title-generation request using the configured override.
- In a disposable `CODEX_HOME`, the wrapper started a managed daemon and
  `restart-if-idle` replaced it (PID 1604012 to 1604151).
- After a cold ChatGPT relaunch, the old PID 3787974 was gone and the owner
  record identified managed PID 1635938. That process runs the installed CLI
  with `--managed-daemon`, confirming the desktop wrapper's managed launch path.
- A live `restart-if-idle` request was issued against PID 1635938. It is still
  waiting for the current assistant turn to become idle; verify the replacement
  PID after this turn completes.
- `bash -n` and `git diff --check` passed. The user-level desktop entry points
  to the wrapper and `desktop-file-validate` passed with an existing category
  hint.
