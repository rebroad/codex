# Rebase TODO: upstream 0.163.0-alpha.11

This is the authoritative, shrinking checklist of work required before the rebase goal is complete. The rebase itself, pushed working branch, native debug build, both device release installs, pairing helpers, and under-20-token device exec checks are recorded as done in [REBASE-DONE.md](REBASE-DONE.md). Keep this list focused on verified open gates; move an item out only after its completion evidence is recorded in `REBASE-DONE.md`.

## Remaining gates

- [ ] **Resolve and rerun the scoped test suite.** The latest elevated selected-package run covered `codex-core`, app-server protocol, app-server, app-server daemon, Linux sandbox, and TUI; it reported 13,567 passed, 62 failed, 4 timed out, and 25 skipped. An isolated elevated TUI run reported 5,779 passed, 41 failed, 3 timed out, and 8 skipped. Classify the failures in `/var/tmp/codex-alpha-rebase-selected-packages-final.log` and `/var/tmp/codex-alpha-rebase-tui-isolated.log`, fix genuine regressions as correctly owned fixups, and distinguish environment-only failures with focused evidence. Then rerun the relevant scoped packages and record the final results. Do not accept host-specific snapshot output.
- [ ] **Correct `restart-if-idle` drain behavior.** Active turns must not put app-server into drain mode. Once no active turns remain, it should already be ready to exit and allow replacement, so a separate drain mode is unnecessary. Keep `--restart-if-idle` out of rebase validation runs.
- [ ] **Final audit and closeout.** Confirm all required validations and fixups are committed and pushed on `rebasing-v0.163.0-alpha.11`, verify the repository is clean, record any justified environment limitation, and remove this final checklist item only after every other item is complete. Do not rewrite history as part of this goal.

## Completion rule

When every required gate is verified, record the final evidence in `REBASE-DONE.md` and reduce this file to zero remaining checklist items (then remove the empty TODO file or leave a one-line completed marker, updating `GOAL-rebase.md` accordingly). The goal is complete only when no required work remains.
