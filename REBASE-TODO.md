# Rebase TODO: upstream 0.163.0-alpha.11

This is the authoritative, shrinking checklist of work required before the rebase goal is complete. The rebase itself and pushed working branch are recorded as done in [REBASE-DONE.md](REBASE-DONE.md). Native and device release builds, pairing helpers, and under-20-token device exec checks passed on an earlier code revision; repeat those validations after the remaining code fixes. Keep this list focused on verified open gates; move an item out only after its completion evidence is recorded in `REBASE-DONE.md`.

## Remaining gates

- [ ] **Resolve and rerun the scoped test suite.** The latest elevated selected-package run covered `codex-core`, app-server protocol, app-server, app-server daemon, Linux sandbox, and TUI; it reported 13,567 passed, 62 failed, 4 timed out, and 25 skipped. The isolated elevated TUI run with a 32 MiB Rust test-thread stack reported 5,790 passed, 30 failed, 3 timed out, and 8 skipped; it eliminated the stack-overflow aborts from the earlier default-stack run (5,779 passed, 41 failed, 3 timed out, 8 skipped). Focused reruns now pass for the six remote-control auth integration cases, Bedrock provider requirements and provider config tests, three restricted executor-skill cases, no-connected-app verification, ephemeral goal rejection, provider-requirements RPC, and the external-writer fork/classifier case; see `REBASE-DONE.md`. These broad results still need a fresh scoped rerun after the latest fixes and any remaining failures classified or repaired. Do not accept host-specific snapshot output.
- [ ] **Revalidate final builds and devices after code fixes.** Build/install the native target as debug and Flip7/Pi 3 as release from the final code revision. Verify pairing helper operation and under-20-token `codex exec` on both devices. Earlier passes are recorded in `REBASE-DONE.md` but predate later app-server and core-plugins code fixes.
- [ ] **Final audit and closeout.** Confirm all required validations and fixups are committed and pushed on `rebasing-v0.163.0-alpha.11`, verify the repository is clean, record any justified environment limitation, and remove this final checklist item only after every other item is complete. Do not rewrite history as part of this goal.

## Completion rule

When every required gate is verified, record the final evidence in `REBASE-DONE.md` and reduce this file to zero remaining checklist items (then remove the empty TODO file or leave a one-line completed marker, updating `GOAL-rebase.md` accordingly). The goal is complete only when no required work remains.
