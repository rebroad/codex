# Rearrangement TODO

- Last verified history state: `alpha` at `4c8ae362dc7b6c0723cebd8e9575e58d46e0291b`, tree `668772b21697a913f5f66a5b427ea73684ab50b7`; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`; 625 downstream commits; no unattached fixups or squash commits.
- Completed pass: retargeted `c04bc0de3d` (`docs(downstream): clarify remote-control owner changes`) as a fixup of `65798016fa` (`docs: define downstream fork authority`), the owner that created the row. The auth feature owner predates `DOWNSTREAM.md`, so the documentation hunk was correctly applied to the guide artifact owner.
- Completed pass: folded `9265725b28` (`docs(downstream): clarify active takeover behavior`) into `0786455a8e` (`DOWNSTREAM.md`), the standalone guide update that introduced the row, after `git log -S` confirmed row ownership. The full replay preserved the checkpoint tree and merge base.
- Before the rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree; replay from the fixed merge base.
- Continue the repeated-path ownership audit after this pass, including other historical `DOWNSTREAM.md` row updates and any remaining mixed commits.
- Next ownership check: `cdbd7a7f9c83` (`docs(downstream): document atomic permission presets`) adds a new `DOWNSTREAM.md` row. Verify whether its behavior owner predates the guide and whether this addition belongs with that owner or with the guide artifact.
- Continue semantic review across the remaining 625 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
