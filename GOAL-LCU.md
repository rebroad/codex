# Goal: Linux Computer Use

Enable native Linux Computer Use in the Codex backend for GUI-only and visual
tasks while keeping shell and API tools as the default for ordinary Linux
work.

## Completion status

| Work item | Completion | Remaining work |
| --- | ---: | --- |
| Linux Sky backend and CUA surface registration | 100% | None for the current host |
| Codex app-server MCP configuration and reload | 100% | Confirm tool visibility from a fresh client turn |
| Linux discovery, launch, window binding, accessibility, and harmless input | 90% | Add explicit screenshot/state verification to the repeatable check |
| Capability detection and clear display/session/compositor errors | 25% | Implement and test structured failure reporting |
| Browser Computer Use preservation | 100% | None observed |
| On-demand use without affecting shell-first workflows | 75% | Document activation and token-cost policy |
| Codex repository tests and repeatable validation | 20% | Add the narrowest appropriate test/diagnostic workflow and run it |
| ChatGPT Desktop production integration | 0% | Requires the external Electron runtime source/build owner |

**Overall estimate: 65%.** The backend is live on this host; remaining work is
repeatable verification, capability/error handling, and optional Desktop
production integration.

## Remaining work

1. From a fresh app-server client turn, verify that the reloaded `linux_cua`
   MCP server is visible and that its JavaScript context exposes
   `cua.listApps`, `cua.computer.launch_app`, and Linux `cua.getApp`.
2. Add a repeatable, harmless Linux validation path covering screenshot/state
   capture in addition to discovery, launch, accessibility, and input.
3. Add capability checks and structured errors for missing display, session,
   AT-SPI, and unsupported Wayland/compositor conditions.
4. Document and enforce shell-first, on-demand Computer Use activation so
   normal Linux tasks do not pay unnecessary context/tool costs.
5. Add and run the narrow Codex-side tests or diagnostics needed to protect the
   Linux MCP configuration and preserve browser-only operation.
6. If integrated Desktop support is required, obtain the external Electron
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
