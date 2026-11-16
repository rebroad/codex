
## Full-range rearrangement (completed 2026-11-16)

- Followed the user's scope clarification: rearrange the live `alpha` history back to the locked `rust-v0.163.0-alpha.11` merge-base, not from an earlier rearranged snapshot.
- Preserved pre-rewrite checkpoint `alpha.before-rearrange-20261116-ownership-audited` (`f86e5c60dc255f0fb7f90893b34d67d751a0ab93`, tree `3cbd60addc80aeb863a7d620ecb27f2bbed584ba`).
- Consolidated Markdown-only record commits into the final record state; source-coupled implementation commits remain in the series. Split mixed-purpose build fixes by owner, retained dependency-sensitive follow-ups after their required code, and retitled the upstream-only test commits.
- Final source tree matches the checkpoint byte-for-byte outside `REARRANGE-TODO.md` and `REARRANGE-DONE.md`. The merge-base is the locked commit, the branch has no merge commits or `fixup!` subjects, and the working tree was clean before these closure records.
- No tests or builds were run, as this is a history-only operation.
- `git range-diff` at pre-publication audit tip `ee1d7ae521`: 310 unchanged pairs, 82 changed pairs, 333 old-only commits, and four new follow-up commits. The old-only set includes the consolidated Markdown-only history and fixups absorbed or split by owner.
- Published the rewritten `alpha` using `--force-with-lease` against fetched remote tip `e7e89cf3f0598b8eeff065243c5bacb6164f2e05`; the branch's associated PR #16 was closed. A post-push fetch confirmed `origin/alpha` matched local `alpha`.
- Final history count after publication and audit-record updates: 398 commits from the locked merge-base, zero merges, zero `fixup!` subjects.

## Follow-up ownership audit (started 2026-11-16)

- Reopened after completion verification found dependency-deferred repairs at the range tip that still have plausible downstream owners: `4ab1244f29` resume metadata locking, `65743da7e1` Rust guard output, `5dcc2d7e5f` idle drain behavior, and `92a2f881fd` resume progress. Snapshot reconciliation commits `4e7b3cc944` and `35d2b1fa26` also need owner-level review.
- Follow-up checkpoint: `alpha.before-rearrange-20261117-owner-corrections` at `f56162dda39d4bf2f156d542b78dea31785ef282`, tree `c5683fe256b98006bb3ab1e5897f0aecf0b0aefb`. The locked base remains `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.
- No source/test changes, tests, or builds are authorized. Reopen publication only after owner corrections and checkpoint equality are verified.
