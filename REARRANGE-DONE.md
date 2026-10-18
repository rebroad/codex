# Rearrangement audit trail

## Current audit baseline

- `alpha` started this continuation clean at `399b40798ba0743c5e46c1a160b114600ef92be3`, tree `082e2c224d72181d4a8c9aa5358775c225ea7938`.
- The current merge base with `upstream/latest-alpha-cli` is `c333d755d44d4eb764cffd2bd862d86d790e91e6`. No upstream rebase or advancement was performed in this continuation.
- The required `REARRANGE-TODO.md` and `REARRANGE-DONE.md` records were missing at this starting point; this file and the TODO file are being introduced as separate commits per `GOAL-rearrange.md`.
- Earlier checkpointed ownership decisions and tree-preservation evidence remain in `REARRANGE-PROGRESS.md`; that historical record states the full semantic and repeated-file audit is still in progress.
- Initial current-tip inspection found three `fixup!` commits: `a34d86357a` (rollout cache diagnostics), `67f004c525` (remote app-server reconnect), and `e05d70ed52` (model-instructions scripts). Their patches affect the named feature's code/tests or formatting and are pending exact owner verification and checkpointed autosquash.

## Completion status

- No history rewrite has been performed in this continuation yet. The full downstream audit, final checkpoint/tree/base checks, and publication remain outstanding.
