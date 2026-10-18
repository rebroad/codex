# Rearrangement TODO

- Current pre-pass state: `alpha` is clean at `b71a72d5d513e7bd6c5bff4ab4fba8d0b718d006`, tree `715a365cada7d5c3a29825c47a18e000a4c41251`; merge base remains `c333d755d44d4eb764cffd2bd862d86d790e91e6`.
- Next audit: split the stale provider-schema definition cleanup out of `71cc648a0e` (`feat(remote-control): capture websocket traffic`) and attach it to the downstream config-provider override owner `24c22fec81`; leave remote-control schema additions with their feature.
- Before the next history rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree.
- Continue semantic review across the remaining 610 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
