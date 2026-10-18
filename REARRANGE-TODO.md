# Rearrangement TODO

- Current state: `alpha` is clean at `71396cb7c0e111cced4bbc4c8afa577ce43fb2a7`, tree `600cc96a4bd0e291b986c129555b11ec29fb6b93`; merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`.
- Before autosquashing the audited schema fixup, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree.
- Autosquash `71396cb7c0` into its verified owner `24c22fec81`, then verify exact checkpoint tree and unchanged merge base.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
