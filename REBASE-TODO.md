# Rebase TODO: upstream 0.163.0-alpha.11

This is the authoritative, shrinking checklist of work required before the rebase goal is complete. The rebase itself and pushed working branch are recorded as done in [REBASE-DONE.md](REBASE-DONE.md). Native and device release builds, pairing helpers, and under-20-token device exec checks passed on an earlier code revision; repeat those validations after the remaining code fixes. Keep this list focused on verified open gates; move an item out only after its completion evidence is recorded in `REBASE-DONE.md`.

## Remaining gates

- [ ] **Resolve and rerun the scoped test suite.** The latest elevated selected-package run at `b4ec1e34bf` covered `codex-core`, app-server protocol, app-server, app-server daemon, Linux sandbox, and TUI; it completed in 908 seconds with 13,586 passed, 42 failed, 6 timed out, and 25 skipped. The remaining failures include injected-environment/sandbox cases, host-sensitive snapshots, TUI aborts/timeouts, Bedrock service-tier cases, auth refresh cases, and other app/TUI assertions. Classify these against the pinned upstream baseline and repair only demonstrated downstream regressions; do not accept host-specific snapshot output. Focused passing reruns and prior broad results are recorded in `REBASE-DONE.md`.
- [ ] **Revalidate final builds and devices after code fixes.** The fresh native debug build passed at `b4ec1e34bf`, and Pi 3 release installation plus pairing helper and 14-token `codex exec` passed at that code revision; see `REBASE-DONE.md`. Flip7 release install and its pairing/under-20-token checks remain open because SSH to `192.168.192.8:8022` reports `No route to host`; retry when it is reachable.
- [ ] **Final audit and closeout.** Confirm all required validations and fixups are committed and pushed on `rebasing-v0.163.0-alpha.11`, verify the repository is clean, record any justified environment limitation, and remove this final checklist item only after every other item is complete. Do not rewrite history as part of this goal.

## Completion rule

When every required gate is verified, record the final evidence in `REBASE-DONE.md` and reduce this file to zero remaining checklist items (then remove the empty TODO file or leave a one-line completed marker, updating `GOAL-rebase.md` accordingly). The goal is complete only when no required work remains.
