# codex-app-server-daemon

> `codex-app-server-daemon` is experimental and its lifecycle contract may
> change while the remote-management flow is still being developed.

`codex-app-server-daemon` backs the machine-readable `codex app-server`
lifecycle commands used by remote clients such as the desktop and mobile apps.
It is intended for Codex instances launched over SSH, including fresh developer
machines that should expose app-server with `remote_control` enabled.

## Platform support

The current daemon implementation is Unix-only. It uses pidfile-backed
daemonization plus Unix process and file-locking primitives, and does not yet
support Windows lifecycle management.

## Commands

```sh
codex app-server daemon start
codex app-server daemon restart
codex app-server daemon enable-remote-control
codex app-server daemon disable-remote-control
codex app-server daemon stop
codex app-server daemon version
codex app-server daemon bootstrap --remote-control
```

On success, every command writes exactly one JSON object to stdout. Consumers
should parse that JSON rather than relying on human-readable text. Lifecycle
responses report the resolved backend, socket path, local CLI version, and
running app-server version when applicable.

## Bootstrap flow

For a new remote machine, build or install Codex into Cargo's binary directory:

```sh
$HOME/.cargo/bin/codex app-server daemon bootstrap --remote-control
```

`bootstrap` records daemon settings under `CODEX_HOME/app-server-daemon/` and
starts app-server as a pidfile-backed detached process. The updater is started
separately when explicitly requested.

## Installation and update cases

The daemon launches the executable used by the current CLI command. Its updater
tracks the `codex` symlink under `$HOME/.cargo/bin`, which is updated by the
local build/release workflow.

| Situation | What starts | Does this daemon fetch new binaries? | Does a running app-server eventually move to a newer binary on its own? |
| --- | --- | --- | --- |
| `start` is used | The current CLI executable starts app-server | No | No. |
| `bootstrap` is used | The current CLI executable starts app-server | No | No. The updater requires explicit opt-in. |
| A newer version is installed into Cargo bin | The updater detects the new target and restarts app-server with it | Installation is performed by the local build/release workflow | Yes. |

### Cargo-bin installs

For installs created by the local build/release workflow:

- lifecycle commands use the executable from the current CLI invocation
- `bootstrap` is supported
- the explicitly started updater tracks `$HOME/.cargo/bin/codex`
- updates are installed as versioned binaries and selected by the `codex` symlink

### Out-of-band updates

This daemon does not watch arbitrary executable files for replacement. If some
other tool updates the Cargo-bin `codex` symlink:

- without `bootstrap`, a currently running app-server remains on the old
  executable image until an explicit `restart`
- with the explicitly started updater, it detects the changed target and
  restarts the running app-server

## Lifecycle semantics

`start` is idempotent and returns after app-server is ready to answer the normal
JSON-RPC initialize handshake on the Unix control socket.

`restart` stops any pid-managed daemon and starts it again using the current
CLI executable.

`enable-remote-control` and `disable-remote-control` persist the launch setting
for future starts. If a pid-managed app-server is already running, they restart it
so the new setting takes effect immediately.

Top-level `codex remote-control` bootstraps with `--remote-control` when the
updater loop is not running. Otherwise it enables remote control and starts the
daemon using the current CLI executable.

`stop` sends a graceful termination request first, then sends a second
termination signal after the grace window if the process is still alive.

All mutating lifecycle commands are serialized per `CODEX_HOME`, so a concurrent
`start`, `restart`, `enable-remote-control`, `disable-remote-control`, `stop`,
or `bootstrap` does not race another in-flight lifecycle operation.

## State

The daemon stores its local state under `CODEX_HOME/app-server-daemon/`:

- `settings.json` for persisted launch settings
- `app-server.pid` for the app-server process record
- `app-server-updater.pid` for stopping stale updater loops from older builds
- `daemon.lock` for daemon-wide lifecycle serialization
