# Rearrangement TODO

- Current state: `alpha` is clean at `6ae9c21ec4027d9d7eb7e2f27d5ad4d6921f667b`, tree `ce72c0304e35e8b331c8ffff82afaeafa6c9e56f`; merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`; no unattached fixups or squash commits remain.
- Next audit: split the executor-skills fixture correction from the unrelated historical rebase-audit section currently bundled in `d51f8ed0f9`; retain the audit report as a separately scoped documentation commit.
- Before the next history rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree. Use the current in-range commit `29b9dfd3e301` for the edit stop and replay from the fixed merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
