# Rebase TODO: upstream 0.163.0-alpha.11

This is the authoritative, shrinking checklist of work required before the rebase goal is complete. The rebase itself and pushed working branch are recorded as done in [REBASE-DONE.md](REBASE-DONE.md). Native and device release builds, pairing helpers, and under-20-token device exec checks passed on an earlier code revision; repeat those validations after the remaining code fixes. Keep this list focused on verified open gates; move an item out only after its completion evidence is recorded in `REBASE-DONE.md`.

## Remaining gates

- [ ] **Finish focused test triage and establish a useful scoped result.** The latest selected-package run, logged at `/var/tmp/codex-alpha-rebase-selected-final-32m.log`, reports 13,440 passed, 51 failed, 143 timed out, and 25 skipped across 13,634 tests (2,615 seconds). Its long timeout cascade is dominated by TUI tests and app-server deadline/mock-server failures under suite-wide load; it is not evidence of dozens of separate regressions. Rerun relevant packages or cases at lower concurrency. Keep the injected sandbox-variable check and the two bubblewrap-dependent failures classified as environment limitations. Compare the recurring TUI behavior/fixture failures and snapshots with the pinned upstream base; change downstream code only when a downstream regression is demonstrated. Record exact focused results and any remaining baseline/environment limitations in `REBASE-DONE.md`.
- [ ] **Revalidate final builds and devices after code fixes.** The most recent native debug build and Pi 3 release installation, pairing helper, and 14-token `codex exec` checks were at `b4ec1e34bf`, before later code fixups; see `REBASE-DONE.md`. Repeat them against the final code revision. Flip7 release install and its pairing/under-20-token checks remain open because SSH to `192.168.192.8:8022` reported `No route to host`; retry when it is reachable.
- [ ] **Final audit and closeout.** Confirm all required validations and fixups are committed and pushed on `rebasing-v0.163.0-alpha.11`, verify the repository is clean, record any justified environment limitation, and remove this final checklist item only after every other item is complete. Do not rewrite history as part of this goal.

## Completion rule

When every required gate is verified, record the final evidence in `REBASE-DONE.md` and reduce this file to zero remaining checklist items (then remove the empty TODO file or leave a one-line completed marker, updating `GOAL-rebase.md` accordingly). The goal is complete only when no required work remains.
