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
codex app-server daemon restart-if-idle
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
starts app-server as a pidfile-backed detached process. It does not fetch or
watch for updates.

## Installation and update cases

The daemon launches the executable used by the current CLI command. The local
build/release workflow explicitly requests a graceful restart after it updates
the `codex` symlink under `$HOME/.cargo/bin`.

| Situation | What starts | Does this daemon fetch new binaries? | Does a running app-server eventually move to a newer binary on its own? |
| --- | --- | --- | --- |
| `start` is used | The current CLI executable starts app-server | No | No. |
| `bootstrap` is used | The current CLI executable starts app-server | No | No. |
| A newer version is installed into Cargo bin | The build workflow requests `restart-if-idle` | Installation is performed by the local build/release workflow | Yes, after active turns finish. |

### Cargo-bin installs

For installs created by the local build/release workflow:

- lifecycle commands use the executable from the current CLI invocation
- `bootstrap` is supported
- updates are installed as versioned binaries and selected by the `codex` symlink

### Out-of-band updates

This daemon does not watch executable files for replacement. Tools that update
the Cargo-bin `codex` symlink should request `restart-if-idle` explicitly.

## Lifecycle semantics

`start` is idempotent and returns after app-server is ready to answer the normal
JSON-RPC initialize handshake on the Unix control socket.

`restart` stops any pid-managed daemon and starts it again using the current
CLI executable.

`restart-if-idle` sends the app-server's graceful shutdown signal and waits for
active assistant turns to finish before starting the current CLI executable.
It does not force-kill the app-server.

`enable-remote-control` and `disable-remote-control` persist the launch setting
for future starts. If a pid-managed app-server is already running, they restart it
so the new setting takes effect immediately.

Top-level `codex remote-control` bootstraps with `--remote-control` when needed;
otherwise it enables remote control and starts the daemon using the current CLI
executable.

`stop` sends a graceful termination request first, then sends a second
termination signal after the grace window if the process is still alive.

All mutating lifecycle commands are serialized per `CODEX_HOME`, so a concurrent
`start`, `restart`, `enable-remote-control`, `disable-remote-control`, `stop`,
or `bootstrap` does not race another in-flight lifecycle operation.

## State

The daemon stores its local state under `CODEX_HOME/app-server-daemon/`:

- `settings.json` for persisted launch settings
- `app-server.pid` for the app-server process record
- `daemon.lock` for daemon-wide lifecycle serialization
