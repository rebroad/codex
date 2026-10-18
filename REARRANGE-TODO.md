# Rearrangement TODO

- Current starting state: `alpha` at `399b40798ba0743c5e46c1a160b114600ef92be3`, tree `082e2c224d72181d4a8c9aa5358775c225ea7938`; clean worktree; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`.
- Before the next history rewrite, create a unique `alpha.before-rearrange-*` checkpoint and record its commit and tree.
- Audit and autosquash the three tip fixups into their verified owners: model-instructions scripts, remote app-server reconnect, and rollout cache diagnostics.
- Continue the semantic fixup and mixed-commit audit across the full downstream range; review repeated file touches by behavior owner.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
