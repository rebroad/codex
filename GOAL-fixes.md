# Completed

- `codex app-server reload` now allows the config reload RPC up to 60 seconds,
  while other control-socket requests retain their shorter timeout.
- Side forks preserve the parent's developer instructions. The side boundary
  rules remain after inherited history, preserving the prompt-cache prefix.
- Build synchronization uses `cpto --no-lngit`; a real sync probe confirmed
  that a Git-ignored file in `codex.build` survives synchronization.
- Cargo lock waiting now cleans orphaned Git lock holders as well as orphaned
  Codex sandbox holders. The test run observed and cleaned an orphaned Git PID.

Validation:

- `just test -p codex-app-server-daemon`: 71 passed.
- Focused side-conversation TUI tests: 2 passed.
- Full `just test -p codex-tui`: failed in 16 other tests (snapshot, reconnect,
  startup, and subprocess cases); the focused side-conversation tests passed.
