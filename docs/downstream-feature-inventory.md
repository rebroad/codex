# Downstream Feature Inventory

This document is the source of truth for downstream features that must survive restacks and rebases onto `upstream/latest-alpha-cli`.

Each feature should be marked during a rebase pass as one of:

- `satisfied by upstream`
- `reapplied downstream`
- `intentionally deferred`

## Must Keep

| Feature | Status | Acceptance note |
| --- | --- | --- |
| `--auth-file` | reapplied downstream | `codex --auth-file <path> ...` uses the selected auth file without changing the current CLI shape. |
| Usage accounting | reapplied downstream | Local usage accounting continues to record usage data needed for downstream status and logging workflows. |
| Usage logs | reapplied downstream | Usage log files continue to be written in a format that downstream tools can read. |
| Model-prices / USD support for usage accounting | reapplied downstream | Usage accounting continues to include USD-derived values required by downstream workflows. |
| `codex status` | reapplied downstream | `codex status` remains a one-shot CLI feature with the current downstream intent. |
| `codex status --` | reapplied downstream | `codex status -- ...` remains supported with the current downstream CLI surface. |
| `usage clear` | reapplied downstream | `codex usage clear ...` keeps its current interface and clears locally tracked usage data. |
| `scripts/rebuild_codex.sh` | reapplied downstream | The rebuild script remains present and usable from the designated build tree. |
| `scripts/build_armv7.sh` | reapplied downstream | The ARMv7 build helper remains present and usable. |
| `tlogin` | reapplied downstream | `codex tlogin ...` start/complete flow keeps its current downstream CLI shape. |
| `scripts/ci_triage.sh` | reapplied downstream | The CI triage helper remains present and runnable. |
| Prompt debug / backend capture | satisfied by upstream | Current upstream already exposes prompt-debug capture plumbing; downstream inspectors/docs are carried separately. |
| `tools/prompt-debug-inspector` | reapplied downstream | Prompt-debug inspector continues to read current capture directories and launch successfully. |
| `tools/rollout-inspector` | reapplied downstream | Rollout inspector continues to read rollout files and launch successfully. |
| `tools/codex-super-inspector` | reapplied downstream | Codex super inspector continues to launch and read expected inputs. |
| `AGENTS.md` | reapplied downstream | Downstream operator instructions remain present in the repo root. |
| bwrap / symlink fixes | satisfied by upstream | Current upstream already contains the symlink and writable-root sandbox fixes this fork depended on. |
| Writable-roots additions | satisfied by upstream | Current upstream already supports the additional writable-root behavior needed by this fork. |
| `codex fork` | satisfied by upstream | Current upstream already has `codex fork` CLI support; downstream branch keeps the surface stable. |
| `scripts/plot_usage_log.py` | reapplied downstream | Plotting tool continues to read current usage logs and produce output. |
| Prompt customizations: emojis | intentionally deferred | Prompt-level customization still needs a clean carry-forward onto current upstream prompt assets. |
| Prompt customizations: stronger `apply_patch` guidance | intentionally deferred | Prompt-level customization still needs a clean carry-forward onto current upstream prompt assets. |
| `--bare-prompt` | intentionally deferred | CLI/config wiring and current upstream exec/TUI integration still need a targeted reimplementation. |
| Delayed-decline / steer fix | intentionally deferred | Needs a fresh audit against current upstream steer behavior before carrying any downstream patch. |
| Config reload on `SIGHUP` | intentionally deferred | Current upstream has reload primitives, but the signal-trigger wiring still needs a narrow downstream reimplementation. |
| Version output includes date, time, and commit hash | intentionally deferred | Version metadata still needs a clean carry-forward compatible with the current build pipeline. |

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
