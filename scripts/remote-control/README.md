# Remote-control helpers

These wrappers start the managed Codex app-server and print the short-lived
manual pairing code used by the ChatGPT/Codex mobile app's Remote connection.

They work on Unix-like systems including Linux and Termux, and require a Codex
build with the `remote-control` command enabled.

## Install

From the repository root:

```sh
install -Dm755 scripts/remote-control/codex-remote-start "$HOME/bin/codex-remote-start"
install -Dm755 scripts/remote-control/codex-pairing-code "$HOME/bin/codex-pairing-code"
```

Then run:

```sh
codex login
codex-pairing-code
```

Enter the displayed code in the ChatGPT/Codex app under Remote → Add or pair
device. The phone and app must use the same ChatGPT account and workspace, and
the code expires quickly. The Codex account must be enrolled for remote
control before a pairing code can be created.

## Configuration

The helpers use `codex` from `PATH` by default. Override it with `CODEX_BIN`.
They use `$HOME/.codex` by default and honor `CODEX_HOME`.
When using a Codex binary that is not installed through the normal Codex
installer, the helpers create the daemon’s expected managed-path symlink to
the `codex` found on `PATH`. Existing managed installations are left alone.

Use `codex-pairing-code --debug-log` when diagnosing daemon startup. This may
replace an existing managed daemon so that remote-control logs are enabled.
