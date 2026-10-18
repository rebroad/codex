# Rearrangement TODO

- Last verified semantic-history state before these audit-record commits: `alpha` at `eedae236246937473820652899bb0aee5c21b546`, tree `3e40d809cf059d2b3fa6680c9d62a6df3a213382`; merge base `c333d755d44d4eb764cffd2bd862d86d790e91e6`; 631 downstream commits; no unattached fixups or squash commits.
- Completed pass: retargeted `c04bc0de3d` (`docs(downstream): clarify remote-control owner changes`) as a fixup of `65798016fa` (`docs: define downstream fork authority`), the owner that created the row. The auth feature owner predates `DOWNSTREAM.md`, so the documentation hunk was correctly applied to the guide artifact owner.
- Completed pass: folded `9265725b28` (`docs(downstream): clarify active takeover behavior`) into `0786455a8e` (`DOWNSTREAM.md`), the standalone guide update that introduced the row, after `git log -S` confirmed row ownership. The full replay preserved the checkpoint tree and merge base.
- Before the rewrite, create a new unique `alpha.before-rearrange-*` checkpoint and record its commit and tree; replay from the fixed merge base.
- Continue the repeated-path ownership audit after this pass, including other historical `DOWNSTREAM.md` row updates and any remaining mixed commits.
- Reviewed `cdbd7a7f9c83` (`docs(downstream): document atomic permission presets`): the behavior owner `07b07fca04` predates `DOWNSTREAM.md` by 13 days, so the source file cannot be changed at that commit. The later commit is a standalone documentation addition and should remain separate.
- Next: continue the repeated `DOWNSTREAM.md` ownership scan, including its other standalone row additions, then continue the wider semantic audit.
- Completed repeated-path reviews: `codex-rs/core/src/config/mod.rs` (23 commits), `codex-rs/config/src/config_toml.rs` (14 commits), and `codex-rs/core/src/session/mod.rs` (23 commits). See DONE for the intentional staged compaction and API-evolution clusters; proceed with other high-touch paths.
- Continue semantic review across the remaining 629 downstream commits, splitting mixed-purpose commits and reviewing repeated file touches by behavior owner; document justified multi-owner touches.
- Retain relevant downstream functionality and record material omissions or upstream-compatibility trade-offs.
- Before completion, verify exact checkpoint tree, unchanged merge base, clean `alpha`, and no unattached `fixup!`/`squash!` commits; update durable notes and push only after the full audit is complete.
