# Downstream Feature Inventory

This document is the source of truth for downstream features that must survive restacks and rebases onto `upstream/latest-alpha-cli`.

Each feature should be marked during a rebase pass as one of:

- `satisfied by upstream`
- `reapplied downstream`
- `intentionally deferred`

## Must Keep

| Feature | Status | Acceptance note |
| --- | --- | --- |
| `--auth-file` | pending | `codex --auth-file <path> ...` uses the selected auth file without changing the current CLI shape. |
| Usage accounting | pending | Local usage accounting continues to record usage data needed for downstream status and logging workflows. |
| Usage logs | pending | Usage log files continue to be written in a format that downstream tools can read. |
| Model-prices / USD support for usage accounting | pending | Usage accounting continues to include USD-derived values required by downstream workflows. |
| `codex status` | pending | `codex status` remains a one-shot CLI feature with the current downstream intent. |
| `codex status --` | pending | `codex status -- ...` remains supported with the current downstream CLI surface. |
| `usage clear` | pending | `codex usage clear ...` keeps its current interface and clears locally tracked usage data. |
| `scripts/rebuild_codex.sh` | pending | The rebuild script remains present and usable from the designated build tree. |
| `scripts/build_armv7.sh` | pending | The ARMv7 build helper remains present and usable. |
| `tlogin` | pending | `codex tlogin ...` start/complete flow keeps its current downstream CLI shape. |
| `scripts/ci_triage.sh` | pending | The CI triage helper remains present and runnable. |
| Prompt debug / backend capture | pending | Backend capture remains usable and produces data that downstream inspection tools can consume. |
| `tools/prompt-debug-inspector` | pending | Prompt-debug inspector continues to read current capture directories and launch successfully. |
| `tools/rollout-inspector` | pending | Rollout inspector continues to read rollout files and launch successfully. |
| `tools/codex-super-inspector` | pending | Codex super inspector continues to launch and read expected inputs. |
| `AGENTS.md` | pending | Downstream operator instructions remain present in the repo root. |
| bwrap / symlink fixes | pending | Keep downstream patches only if upstream still misses the required bwrap and symlink behavior. |
| Writable-roots additions | pending | Keep downstream writable-root additions only if upstream still misses them. |
| `codex fork` | pending | `codex fork ...` keeps the current downstream behavior and CLI surface. |
| `scripts/plot_usage_log.py` | pending | Plotting tool continues to read current usage logs and produce output. |
| Prompt customizations: emojis | pending | Downstream prompt surface continues to allow the current emoji behavior. |
| Prompt customizations: stronger `apply_patch` guidance | pending | Downstream prompt surface keeps the stronger `apply_patch` guidance. |
| `--bare-prompt` | pending | `--bare-prompt` remains available with its current downstream behavior. |
| Delayed-decline / steer fix | pending | Keep downstream patch only if upstream still misses the required delayed-decline/steer behavior. |
| Config reload on `SIGHUP` | pending | Sending `SIGHUP` continues to trigger config reload behavior. |
| Version output includes date, time, and commit hash | pending | Version output continues to expose date, time, and commit hash. |

## Upstream Check Required

These features should only remain as downstream patches if upstream still does not satisfy them:

- bwrap / symlink fixes
- writable-roots additions
- delayed-decline / steer fix

## Interface Compatibility

The following surfaces are compatibility targets and must keep their current downstream command names and important flags:

- `--auth-file`
- `codex status`
- `codex status --`
- `codex usage clear`
- `codex fork`
- `codex tlogin`
- `--bare-prompt`
- `scripts/rebuild_codex.sh`
- `scripts/build_armv7.sh`
- `scripts/ci_triage.sh`
- `scripts/plot_usage_log.py`
- `tools/prompt-debug-inspector`
- `tools/rollout-inspector`
- `tools/codex-super-inspector`

Internal implementations may change as long as those user-facing interfaces stay stable.

## Rebase Notes

- Prefer upstream implementations when they already satisfy a must-keep feature.
- Prefer downstream-owned files, scripts, and tools over large edits in upstream hot files.
- Fold fixups into the feature commit being carried forward.
- Do not preserve old temporary carry commits, formatting-only commits, or TODO/WIP commits unless they encode active required behavior.
