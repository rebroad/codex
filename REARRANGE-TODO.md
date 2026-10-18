# Rearrangement TODO

- Last verified history state: `alpha` at `b9c44c4edfe30c79b35887e01015bedb390fc18e`, tree `6eee9da71c387eafb0b0fefc2d3abd0a0a8cbed1`; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`; no unattached fixups or squash commits.
- Continue the repeated-path ownership audit, including the remaining generated config-schema commits and historical `DOWNSTREAM.md` row updates; split only changes with distinct semantic owners.
- Before the next history rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
