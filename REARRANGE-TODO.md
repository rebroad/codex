# Rearrangement TODO

- Last verified history state: `alpha` at `d3969b5ebb0f8105b233725dbff01a0177ab0463`, tree `c01865f5bff57895db0097c8ae90f82a5c038542`; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`; no unattached fixups or squash commits.
- Next pass: retarget `c04bc0de3d` (`docs(downstream): clarify remote-control owner changes`) as a fixup of `65798016fa` (`docs: define downstream fork authority`), the owner that created the row. The auth feature owner precedes `DOWNSTREAM.md`, so the documentation hunk cannot be applied there.
- Before the rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree; replay from the fixed merge base.
- Continue the repeated-path ownership audit after this pass, including other historical `DOWNSTREAM.md` row updates and any remaining mixed commits.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
