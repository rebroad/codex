# Rearrangement TODO

- Current working state: `alpha` is clean after the tip-fixup pass; checkpoint `alpha.before-rearrange-20261018-tip-fixups` records commit `b354678808d4568c5d8c81fb3fa5dee9225fffcd` and tree `7fda08ea79e81e10d85729851e59ac9de9889a37`; merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`.
- Before the next history rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
