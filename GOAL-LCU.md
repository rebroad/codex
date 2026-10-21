# Goal: Linux Computer Use

Enable native Linux Computer Use in the Codex backend for GUI-only and visual
tasks while keeping shell and API tools as the default for ordinary Linux
work.

## Completion status

| Work item | Completion | Remaining work |
| --- | ---: | --- |
| Linux Sky backend and CUA surface registration | 100% | None for the current host |
| Codex app-server MCP configuration, reload, and fresh-client visibility | 100% | None for the current host |
| Linux discovery, launch, window binding, accessibility, screenshot, and harmless input | 100% | None for the current X11 host |
| Capability detection and clear display/session/compositor errors | 100% | Current backend reports display, X11, Wayland, accessibility, and MCP failures; unsupported Wayland remains explicit |
| Browser Computer Use preservation | 100% | None observed |
| On-demand use without affecting shell-first workflows | 100% | Shell-first policy documented and enforced by usage guidance |
| Codex repository tests and repeatable validation | 100% | None for the current backend path |
| ChatGPT Desktop production integration | 0% | Requires the external Electron runtime source/build owner |

**Overall estimate: 100% for the Codex backend.** The backend is live on this
host and the required validation gates pass.

## Optional external follow-ups

- If integrated ChatGPT Desktop support is required, obtain the external
  Electron source/build owner and enable the Linux production `computerUse`
  feature gate there. The Codex backend does not depend on this integration,
  and generated plugin caches remain out of scope.

## Final validation command

```bash
python3 scripts/linux_cua_check.py --screenshot
```

The command must return a JSON object with `status: "ok"`, a Linux CUA API
surface, app/window counts, `displayBackend`, screenshot bytes, and accessibility
bytes. Use the focused unit tests to validate configuration and failure paths.

## Constraints

- Use the existing Sky Linux backend; do not reimplement it.
- Keep computer actions capability-gated and subject to existing approval and
  guardian policy checks.
- Do not substitute arbitrary shell execution for structured app operations.
- Do not promise generic arbitrary-window control on every Wayland compositor.
- Keep source edits in this checkout and builds in the designated external
  build tree.
