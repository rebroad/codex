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
| Capability detection and clear display/session/compositor errors | 80% | Add deeper AT-SPI failure classification when Sky reports it |
| Browser Computer Use preservation | 100% | None observed |
| On-demand use without affecting shell-first workflows | 75% | Document activation and token-cost policy |
| Codex repository tests and repeatable validation | 100% | None for the current backend path |
| ChatGPT Desktop production integration | 0% | Requires the external Electron runtime source/build owner |

**Overall estimate: 78%.** The backend is live on this host; remaining work is
repeatable verification, capability/error handling, and optional Desktop
production integration.

## Remaining work

1. Keep the repeatable diagnostic healthy with:
   `python3 scripts/linux_cua_check.py --screenshot`.
2. Extend capability checks with deeper backend-specific AT-SPI failure
   classification; the diagnostic already reports missing display/session and
   unvalidated Wayland configuration with a distinct exit code.
3. Document and enforce shell-first, on-demand Computer Use activation so
   normal Linux tasks do not pay unnecessary context/tool costs.
4. Add and run any further focused automated test around the diagnostic's
   and error paths, while preserving browser-only operation.
5. If integrated Desktop support is required, obtain the external Electron
   source/build owner and enable the Linux production `computerUse` feature
   gate there. Do not edit generated plugin caches.

## Constraints

- Use the existing Sky Linux backend; do not reimplement it.
- Keep computer actions capability-gated and subject to existing approval and
  guardian policy checks.
- Do not substitute arbitrary shell execution for structured app operations.
- Do not promise generic arbitrary-window control on every Wayland compositor.
- Keep source edits in this checkout and builds in the designated external
  build tree.
