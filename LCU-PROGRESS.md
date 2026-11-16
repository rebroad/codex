# Linux Computer Use Progress

This file records completed implementation and verification work for
`GOAL-LCU.md`. The goal file intentionally contains only remaining work and
completion percentages.

## Completed work

- Fixed built-in Computer Use routing on 2026-10-29. Added a `[mcp_servers.cua_repl]`
  config entry that duplicates the working `linux_cua` launcher and session
  environment, while retaining the bundled plugin's browser provider settings.
  The override enables both `browser,computer` surfaces and trusts both the
  browser desktop service and Sky. After `codex app-server reload`, the
  built-in `mcp__cua_repl__js` tool reset successfully; `cua.getState()` returned
  106 apps and two browser tabs, and `cua.listWindows()` returned 29 windows.
  This is a host config integration: the bundled plugin cache remains
  browser-only, and no user-wide `AGENTS.md` guidance is needed.
- Resolved a native-surface routing failure on 2026-10-29. The generic
  `mcp__cua_repl` integration exposed browser tabs only in that session; the
  separately configured `mcp__linux_cua` service was the native Linux path but
  lacked `DISPLAY` and `XAUTHORITY` in its MCP environment. Added the verified
  active X11 values (`:0` and `/home/rebroad/.Xauthority`) to the
  `linux_cua` server configuration and reloaded app-server. A fresh
  `mcp__linux_cua` runtime now discovers 106 desktop apps and 10 windows.
  Native Linux tasks must use `mcp__linux_cua`; the generic CUA server remains
  browser-only.
- The configured startup banner supplies `cua.listWindows` from the Linux
  computer backend when the installed CUA package omits that optional wrapper.
  The MCP server is discoverable in the session's tool inventory as
  `mcp__linux_cua__js`. It is separate from the browser-only
  `mcp__cua_repl`, so the latter cannot be transparently redirected to it.
- Regression test with a fresh agent on 2026-10-29: invoking the built-in
  Computer Use skill initialized the bundled browser-only runtime, returned
  Chrome tabs, and had no native `listWindows`; its summarized tool inventory
  separately showed three Linux CUA MCP functions. An explicit config-level
  `cua_repl` alias exposed the expected tool name but its process could not
  connect to X11 (`Connection refused`), so that alias was removed. The
  separately configured `linux_cua` MCP still initializes on the host and
  reports 106 apps and 29 windows. A later config alias copied from the working
  Linux CUA server was verified through the built-in `cua_repl` name as recorded
  above; the earlier failed alias's exact cause was not established.

- Confirmed that the installed `@oai/sky` package includes a Linux backend and
  binary for app discovery, launching, windows, screenshots, mouse, keyboard,
  X11, and accessibility.
- Confirmed that `@oai/cua-repl` supports the `computer` surface and registers
  the trusted `@oai/sky/service` when configured with
  `CUA_REPL_ENABLED_SURFACES=computer` or `browser,computer`.
- Proved the Linux CUA API contract in an isolated MCP process:
  `cua.listApps`, `cua.computer.launch_app`, and
  `cua.getApp({ windowId })` are available when the computer surface is
  enabled.
- Performed an authorized host-context probe against the active X11 session:
  application discovery returned desktop entries including `steam.desktop`,
  and window enumeration returned usable IDs.
- Launched `xfce4-about.desktop`, bound its window, and read a 6,215-character
  AT-SPI accessibility tree.
- Exercised Mousepad input with `typeText`, `Ctrl+A`, and `Backspace`; the
  temporary probe left no Mousepad process running.
- Added a local `[mcp_servers.linux_cua]` entry to
  `/home/rebroad/.codex/config.toml`, preserving the existing browser plugin.
  The entry launches the installed CUA REPL with the Linux computer surface and
  trusted Sky service.
- Verified the downstream `codex app-server reload` command against the live
  control socket. It returned `Reloaded mutable app-server configuration.`
- Verified host process state after reload: app-server-owned CUA processes are
  running with `CUA_REPL_ENABLED_SURFACES=computer` and
  `NODE_REPL_TRUSTED_SERVICES={"sky":"@oai/sky/service"}`. Existing
  browser-only CUA processes remain separate.
- After the recent `accept-chatgpt-update.mjs` run, a fresh MCP process using
  the reloaded backend still exposed `cua.listApps` and
  `cua.computer.launch_app` and returned bounded Linux app inventory. The
  update changed `@oai/sky` from 0.7.6 to 0.7.8 and replaced its Linux binary;
  no Linux CUA regression was observed.
- The same fresh process bound an existing Xfce terminal window and captured a
  78,986-byte screenshot plus an X11 accessibility tree, completing the
  screenshot/state verification gate for the current host.
- Added `scripts/linux_cua_check.py`, a read-only diagnostic that reads the
  configured `linux_cua` MCP entry, performs the required CUA initialization,
  checks the API surface and inventory, and optionally verifies screenshot and
  accessibility capture. It returns a distinct configuration/display/MCP/window
  exit code and a bounded JSON result.
- Ran `python3 scripts/linux_cua_check.py --screenshot` successfully:
  106 apps, 10 windows, 983 screenshot bytes, and 99 accessibility bytes.
- Ran the same diagnostic without `DISPLAY` and `WAYLAND_DISPLAY`; it returned
  exit code 3 with `no DISPLAY or WAYLAND_DISPLAY is set; run from a graphical
  session`.
- Python compilation and Ruff checks pass for the diagnostic.
- Added three standard-library unit tests for missing configuration, invalid
  TOML, and missing graphical-session handling. All three pass, as do Ruff and
  `git diff --check`.
- Refined capability classification so backend messages mentioning X11,
  Wayland, AT-SPI, accessibility, D-Bus, or display/session setup return the
  dedicated capability exit code. An unavailable `DISPLAY=:99` was verified
  to return code 3 with the backend's actionable X11 guidance; the healthy
  `--screenshot` path still passes afterward.
- The diagnostic now reports `displayBackend: "x11"`, surfaces structured
  `cua.getState()` inventory errors, and explicitly rejects an unvalidated
  Wayland-only session with capability exit code 3. Both the healthy X11 path
  and the Wayland-only failure path were run successfully.
- Operating policy: use shell, filesystem, process, and service tools for normal
  Linux work. Invoke `linux_cua` only for GUI-only applications, visual state,
  accessibility workflows, or UI input. The backend remains configured so it
  can be used on demand; no Computer Use calls are needed for shell-first work.
- A session-bus failure probe was also run. Sky's X11 path remained usable and
  returned a valid accessibility tree, so D-Bus is not incorrectly treated as
  a mandatory prerequisite for this X11 backend.
- Final audit: the live diagnostic returned `status: "ok"` with 106 apps, 10
  windows, X11 state, screenshot bytes, and accessibility bytes; all five unit
  tests and Ruff passed. The Codex backend goal is complete for this host.
- Confirmed that `/mnt/kingston/@home/rebroad/src/chatgpt-window` is not the
  desktop runtime source; it is a small Chrome/ChatGPT bridge with no Electron
  feature gate or plugin reconciliation code.

## Boundaries and negative findings

- The installed ChatGPT Desktop production runtime still materializes the
  bundled plugin as browser-only because its Linux `computerUse` feature gate is
  disabled. The host's Codex config override enables both surfaces for the
  running app-server, but does not change Desktop's plugin packaging or make
  this setup portable to other Linux installations.
- The generated cache under `~/.codex/plugins/cache` is not an implementation
  target and was not modified.
- The Codex repository has no Electron Desktop source. The backend path is
  therefore the viable Codex-owned integration; changing Desktop production
  behavior requires its external source/build owner.
- The isolated sandbox cannot connect to the active `DISPLAY=:0.0`; host
  validation used an authorized host-context process instead.

## Evidence locations

- Codex goal and remaining work: `GOAL-LCU.md`
- Local app-server configuration: `/home/rebroad/.codex/config.toml`
- Installed CUA launcher: `/mnt/kingston/@home/rebroad/src/chatgpt-resources/cua_node`
- App-server stderr log: `/home/rebroad/.codex/app-server-daemon/app-server.stderr.log`
