# Rearrangement audit trail

## Current audit baseline

- `alpha` started this continuation clean at `399b40798ba0743c5e46c1a160b114600ef92be3`, tree `082e2c224d72181d4a8c9aa5358775c225ea7938`.
- The current merge base with `upstream/latest-alpha-cli` is `c333d755d44d4eb764cffd2bd862d86d790e91e6`. No upstream rebase or advancement was performed in this continuation.
- The required `REARRANGE-TODO.md` and `REARRANGE-DONE.md` records were missing at this starting point; this file and the TODO file are being introduced as separate commits per `GOAL-rearrange.md`.
- Earlier checkpointed ownership decisions and tree-preservation evidence remain in `REARRANGE-PROGRESS.md`; that historical record states the full semantic and repeated-file audit is still in progress.
- Initial current-tip inspection found three `fixup!` commits: `a34d86357a` (rollout cache diagnostics), `67f004c525` (remote app-server reconnect), and `e05d70ed52` (model-instructions scripts). Their patches affect the named feature's code/tests or formatting and are pending exact owner verification and checkpointed autosquash.

## Tip-fixup ownership and autosquash pass

- Checkpoint: `alpha.before-rearrange-20261018-tip-fixups` at `b354678808d4568c5d8c81fb3fa5dee9225fffcd`, tree `7fda08ea79e81e10d85729851e59ac9de9889a37`.
- Verified exact downstream owners: `a34d86357a` belongs to `194448ebac` (`Add rollout cache diagnostics sidecars`); `67f004c525` belongs to `1f8823c8c5` (`feat(app-server-client): reconnect remote clients`); `e05d70ed52` belongs to `1f733294ae` (`extract/rebuid model_instructions tools`). Their patches update only the corresponding feature files and have no cross-owner content.
- Autosquashed the three audited fixups using the unchanged merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`. `git range-diff` showed all other commits unchanged and only those three fixups absorbed into their owners.
- Verification: resulting tree `7fda08ea79e81e10d85729851e59ac9de9889a37` exactly matches the checkpoint; merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`; 610 downstream commits remain; no `fixup!` or `squash!` subjects remain; worktree is clean. The full semantic and repeated-file audit is still in progress. This pass changed history only, so tests were not rerun.

## Completion status

- No history rewrite has been performed in this continuation yet. The full downstream audit, final checkpoint/tree/base checks, and publication remain outstanding.
