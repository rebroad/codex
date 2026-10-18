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

## Generated config-schema ownership audit

- The downstream range touches `codex-rs/core/config.schema.json` 32 times. In `71cc648a0e` (`feat(remote-control): capture websocket traffic`), the schema adds the remote-control diagnostics types and setting, but also removes the old `ModelProviderInfo`, `ModelProviderCapabilities`, and `RemoteCompactionSupport` definitions.
- `git blame` attributes the removed model-provider definitions to upstream schema commits; the downstream provider-override owner `24c22fec81` changes `ConfigToml.model_providers` from `ModelProviderInfo` to `ModelProviderInfoOverrides`, but its schema update leaves those now-unreferenced definitions behind. The deletion therefore completes the schema change owned by `24c22fec81`; it is independent of remote-control traffic capture.
- The only intervening schema edits between `24c22fec81` and `71cc648a0e` add `sandbox_log_path` and update the background-terminal timeout description. They do not overlap the provider definitions. The model-provider cleanup is planned as a separate fixup to `24c22fec81`; the remote-control schema addition stays with `71cc648a0e`.
- Checkpointed split: `alpha.before-rearrange-20261018-config-schema` at `af0394b1b3b33211bcd6c57d3cddf27a262931a0`, tree `600cc96a4bd0e291b986c129555b11ec29fb6b93`. Split `71cc648a0e` into the focused remote-control feature commit `88fb3dd4c4` and `71396cb7c0` (`fixup! feat(config): allow built-in provider overrides`), containing only the 171-line stale provider-schema cleanup. The two commits recreate the original `71cc648a0e` tree exactly; the full final tree also matches the checkpoint, and the merge base is unchanged. `git range-diff` shows only this split; the fixup still needs autosquashing into its owner.
- Autosquash checkpoint: `alpha.before-rearrange-20261018-config-schema-autosquash` at `7ad42e27e224bdf931d386a34735bc9a50442cc9`, tree `ce72c0304e35e8b331c8ffff82afaeafa6c9e56f`. Autosquashed the schema cleanup into `feat(config): allow built-in provider overrides` (`8f5c109395`); the remote-control feature remains focused at `1cfad53e85`. Resolved the expected schema context conflicts by applying only the provider-definition removals at the provider owner and only the remote-control additions at the remote-control feature. Verification: final tree exactly matches the checkpoint, merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`, 616 downstream commits remain, no `fixup!`/`squash!` subjects remain, schema parses, and the worktree is clean. History-only pass; tests were not rerun.
- Next mixed-commit candidate: `d51f8ed0f9` adds a two-line executor-skills fixture cwd correction plus an 11-line historical audit section to `DOWNSTREAM.md`. The test fix and rebase report have separate purposes; keep the report as a standalone documentation change with no product owner.
- A suffix-rebase attempt used the stale pre-rewrite object `d51f8ed0f9`, which is not an ancestor of the current `alpha`; Git began skipping many already-applied patches and stopped in an unrelated provider-schema commit. Aborted it and verified `alpha` exactly matches `alpha.before-rearrange-20261018-downstream-audit-split` (`4e19ab0de9394c4e7914ebda4ee2017c557f3b41`, tree `c41d3a2eff5a6fc4f1bbe76b39458332f3dddbf2`) with the same merge base. The current in-range test commit is `29b9dfd3e301`; retry by editing that exact hash in a same-base full-range rebase.

## Completion status

- No history rewrite has been performed in this continuation yet. The full downstream audit, final checkpoint/tree/base checks, and publication remain outstanding.
