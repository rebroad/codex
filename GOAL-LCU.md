# Goal: Linux Computer Use

Enable and validate native Linux Computer Use in the Codex desktop runtime so
the Linux session can discover desktop applications, launch them, inspect their
windows, and perform approved UI interactions.

## Current state

- `~/src/codex` and `/mnt/kingston/@home/rebroad/src/codex` resolve to the same
  source checkout.
- The installed `@oai/sky` package already contains a Linux backend and binary
  with app discovery, app launching, window, screenshot, mouse, keyboard, X11,
  and accessibility support.
- The bundled `unified-computer-use` plugin currently exposes only the browser
  surface. Its runtime therefore omits the `cua.listApps` and
  `cua.computer.launch_app` interfaces even though the tool documentation can
  mention them.
- The generated plugin cache under `~/.codex/plugins/cache` is not an editing
  target; updates may replace it.

## Implementation evidence

- The installed `@oai/cua-repl` launcher accepts `CUA_REPL_ENABLED_SURFACES=
  browser,computer` and registers `@oai/sky/service` for the computer surface.
- An isolated MCP probe with those settings exposed `cua.listApps`,
  `cua.computer.launch_app`, and the Linux `cua.getApp({ windowId })` contract
  without the previous JavaScript API errors.
- The local bundled-plugin source manifest is configured for both surfaces and
  trusts the Sky service. The generated plugin cache still needs to be
  regenerated or reloaded by the desktop runtime.
- The isolated probe could not validate live windows because this sandbox cannot
  connect to the active `DISPLAY=:0.0`; that is an environment gate, not an API
  failure.
- An authorized host-context probe reached the X11 session and enumerated the
  desktop successfully, including `steam.desktop`, running applications, and
  window IDs.
- The same probe launched `xfce4-about.desktop`, bound its returned window,
  and read a 6,215-character AT-SPI accessibility tree.
- A Mousepad probe completed `typeText`, `Ctrl+A`, and `Backspace` through the
  Linux CUA surface. The temporary probe process left no Mousepad process
  running.

## Scope

1. Enable the Linux computer surface in the local bundled Computer Use runtime.
2. Register the trusted Linux Sky service and preserve the existing browser
   surface.
3. Add capability detection and clear failures for missing display, session,
   accessibility, or compositor permissions.
4. Validate the public CUA contract on Linux:
   - enumerate applications;
   - launch an application by its app identifier;
   - enumerate and bind its windows;
   - capture state or screenshots;
   - perform approved mouse, keyboard, and text actions.
5. Keep browser-only operation working when native Computer Use is unavailable.

The first implementation should use the existing Sky Linux backend. A new
standalone host bridge is out of scope unless the existing runtime cannot be
enabled or does not meet the required Linux session constraints.

## API details

The Linux surface is expected to use the following shape:

```js
await cua.listApps();
await cua.computer.launch_app({ app: "<app name or id>" });
await cua.getApp({ windowId: "<window id>" });
```

`launch_app` is a computer-surface operation, not a method on the bound app
object. Linux app bindings use a window identifier returned by app or window
enumeration.

## Safety and platform constraints

- Keep the computer surface capability-gated and subject to existing approval
  and guardian policy checks.
- Do not expose arbitrary shell execution as a substitute for structured app
  operations.
- Detect X11 versus Wayland and report unsupported or unavailable capabilities
  instead of silently guessing with coordinate input.
- Do not edit generated plugin caches or external build trees.

## Verification gates

- Browser Computer Use continues to initialize and operate.
- Linux Computer Use initializes only when the trusted Sky service and an active
  graphical user session are available.
- `listApps` returns a bounded, structured result without a JavaScript API
  error.
- `launch_app` starts a selected desktop application and returns a usable
  window handle, or a precise capability/error result.
- Window inspection, screenshot, and a harmless input action work on each
  supported display backend tested.
- The relevant Codex tests pass using the repository build-tree workflow.
- The source checkout is clean after the change and the final runtime path is
  documented.

## Non-goals

- Reimplementing the Sky Linux backend from scratch.
- Promising generic arbitrary-window control on every Wayland compositor.
- Editing the installed plugin cache as a permanent solution.
