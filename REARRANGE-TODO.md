# Rearrangement TODO

- Last verified history state: `alpha` at `c0c6dead96a264949f7793f20abe8ca1a1cac10f`, tree `e3396ca8547671d23ca2c944084c667c846db400`; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`; 624 downstream commits; no unattached fixups or squash commits.
- Completed pass: retargeted `c04bc0de3d` (`docs(downstream): clarify remote-control owner changes`) as a fixup of `65798016fa` (`docs: define downstream fork authority`), the owner that created the row. The auth feature owner predates `DOWNSTREAM.md`, so the documentation hunk was correctly applied to the guide artifact owner.
- Before the rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree; replay from the fixed merge base.
- Continue the repeated-path ownership audit after this pass, including other historical `DOWNSTREAM.md` row updates and any remaining mixed commits.
- Continue semantic review across the remaining 624 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
