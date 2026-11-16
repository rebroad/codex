
## Full-range rearrangement (completed 2026-11-16)

- Followed the user's scope clarification: rearrange the live `alpha` history back to the locked `rust-v0.163.0-alpha.11` merge-base, not from an earlier rearranged snapshot.
- Preserved pre-rewrite checkpoint `alpha.before-rearrange-20261116-ownership-audited` (`f86e5c60dc255f0fb7f90893b34d67d751a0ab93`, tree `3cbd60addc80aeb863a7d620ecb27f2bbed584ba`).
- Consolidated Markdown-only record commits into the final record state; source-coupled implementation commits remain in the series. Split mixed-purpose build fixes by owner, retained dependency-sensitive follow-ups after their required code, and retitled the upstream-only test commits.
- Final source tree matches the checkpoint byte-for-byte outside `REARRANGE-TODO.md` and `REARRANGE-DONE.md`. The merge-base is the locked commit, the branch has no merge commits or `fixup!` subjects, and the working tree was clean before these closure records.
- No tests or builds were run, as this is a history-only operation.
- Final `git range-diff` against the preserved full-range checkpoint: 310 unchanged pairs, 82 changed pairs, 333 old-only commits, and four new follow-up commits. The old-only set includes the consolidated Markdown-only history and fixups absorbed or split by owner.
- Published the rewritten `alpha` using `--force-with-lease` against fetched remote tip `e7e89cf3f0598b8eeff065243c5bacb6164f2e05`; the branch's associated PR #16 was closed. A post-push fetch confirmed `origin/alpha` matched local `alpha`.
- Final history count after this closure record: 397 commits from the locked merge-base, zero merges, zero `fixup!` subjects.
