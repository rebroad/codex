# Remote-control helpers

These wrappers start the Codex app-server and print the short-lived
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

The helpers use `codex` from `PATH`. They use `$HOME/.codex` by default and
honor `CODEX_HOME`. The daemon uses the `codex` executable selected by the
current command and keeps its lifecycle state under `CODEX_HOME`.

Use `codex-pairing-code --debug-log` when diagnosing daemon startup. This may
replace an existing managed daemon so that remote-control logs are enabled.

Use `codex-pairing-code -p PROFILE` (or `--profile PROFILE`) to pair through a
named Codex profile. The profile gets its own app-server socket and daemon
state under `CODEX_HOME`, so it can run alongside the default profile without
requiring a separate `CODEX_HOME`.

## Traffic capture

The app-server can capture remote-control WebSocket traffic as JSON Lines for
protocol debugging. Add the following to `~/.codex/config.toml`:

```toml
[remote_control]
traffic_log = "/var/tmp/codex-remote-control-$$.jsonl"
traffic_log_redaction = "secrets"
```

`traffic_log` is a filename template. Each `$$` in the path expands to the
app-server process ID, so the example produces a file such as
`codex-remote-control-2286612.jsonl`. The PID can appear anywhere in the path;
the app-server does not add a timestamp or extension. If `$$` is omitted, the
path is used exactly as written, and a second process cannot overwrite an
existing capture file. The parent directory is created automatically. Omit
this setting to disable capture.

`traffic_log_redaction` controls what is written to the capture:

- `"secrets"` (default) redacts secret-like fields such as authorization
  headers, access or refresh tokens, cookies, passwords, and bearer values while
  preserving normal request and response content.
- `"content"` also redacts common payload fields such as `command`, `input`,
  `output`, `prompt`, `text`, and `content`.
- `"disabled"` writes the original wire JSON without parsing and
  re-serializing it. This preserves formatting useful for protocol analysis
  and has lower capture overhead, but can expose credentials and user data.

Capture output is flushed periodically and when the remote-control connection
worker stops, so the most recent records may not be visible immediately while
the process is running. Restart the app-server after changing these settings.

Treat capture files as sensitive whenever `traffic_log_redaction =
"disabled"`, and remove them after the diagnostic run.
