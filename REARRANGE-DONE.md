
## Full-range rearrangement (completed 2026-11-16)

- Followed the user's scope clarification: rearrange the live `alpha` history back to the locked `rust-v0.163.0-alpha.11` merge-base, not from an earlier rearranged snapshot.
- Preserved pre-rewrite checkpoint `alpha.before-rearrange-20261116-ownership-audited` (`f86e5c60dc255f0fb7f90893b34d67d751a0ab93`, tree `3cbd60addc80aeb863a7d620ecb27f2bbed584ba`).
- Consolidated Markdown-only record commits into the final record state; source-coupled implementation commits remain in the series. Split mixed-purpose build fixes by owner, retained dependency-sensitive follow-ups after their required code, and retitled the upstream-only test commits.
- Final source tree matches the checkpoint byte-for-byte outside `REARRANGE-TODO.md` and `REARRANGE-DONE.md`. The merge-base is the locked commit, the branch has no merge commits or `fixup!` subjects, and the working tree was clean before these closure records.
- No tests or builds were run, as this is a history-only operation.
