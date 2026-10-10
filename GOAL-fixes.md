# Goals

## Completed

- [x] Use `gpt-6-luna` as the default model for generated conversation titles.
- [x] Add the `thread_title_model` config override and schema entry.
- [x] Verify the config, core, and TUI title-generation tests.
- [x] Diagnose the ChatGPT app-server restart issue: the build script's
  `restart-if-idle` command only replaces a daemon managed by Codex. The app
  had launched an unmanaged child process, so the restart was correctly
  refused.
- [x] Add a ChatGPT desktop launcher wrapper that starts a managed app-server
  before launching the app. Verify a cold launch uses that managed daemon and
  verify daemon replacement in a disposable `CODEX_HOME`.
- [x] Implement Termux soft-keyboard requests when editable TUI input receives
  focus, including the tmux passthrough form.

## Remaining

- [ ] Verify on Flip7, using its virtual display without disturbing the user's
  display, that tapping the prompt/editable area opens the virtual keyboard.
  The device was not connected during this review, so this device-level check
  remains unverified.
