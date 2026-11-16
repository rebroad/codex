
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

## Full-range owner corrections (completed locally 2026-11-17)

- Re-ran the ownership correction rebase over the entire locked range `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3..alpha`, using checkpoint `alpha.before-rearrange-20261117-owner-corrections` (`f56162dda39d4bf2f156d542b78dea31785ef282`, tree `c5683fe256b98006bb3ab1e5897f0aecf0b0aefb`). The base was not advanced.
- Folded resume metadata synchronization into `fix(thread-store): bound legacy resume context`; idle-wait preservation into `feat(app-server): support active turn handoff`; and snapshot corrections into `feat(tui): report live response reception`.
- Split resume-progress integration by dependency: progress estimation and lifecycle changes belong to `feat(tui): display resume progress`, while startup/provider-selection integration belongs to `feat(tui): improve resume session selection`.
- Split the Rust output correction by behavior: guard event formatting with the timestamped guard owner, periodic ballast formatting with the compact timing owner, successful-finish suppression with the finish-filter owner, and the compiler marker changes with their emitter, test, and documentation owners. These changes reproduce the checkpoint output exactly.
- Audited the separate build-script split candidate `185240760e`: it is outside the checkpoint’s locked downstream range. In the in-range history, Android linker-argument deduplication belongs to `build: add isolated Codex build orchestrator`, and remote command quoting belongs to `Add --restart option to build_codex.sh`; the final blame and owner commits show the behaviors separated.
- Reviewed the post-consolidation highest-touch paths: `scripts/build_codex.sh` (34 commits), `scripts/codex_rustc_oom_wrapper.sh` (27), `scripts/codex_rustc_timing.sh` (18), `scripts/README_android_ballast.md` (15), `scripts/tests/test_codex_rustc_timing.sh` (14), `scripts/codex_cargo_env.sh` (14), `scripts/tests/test_codex_rust_guard_output.py` (12), and `codex-rs/core/config.schema.json` (12). Their touches correspond to distinct orchestration, guard/timing, integration, tests, documentation, or config changes; shared-file location alone did not justify merging those purposes. Markdown progress churn remains consolidated.
- Local verification: merge-base is still `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`; the source tree equals the checkpoint except the two audit records; `alpha` has 394 commits in the range, zero merges, and zero `fixup!`/`squash!`/`amend!` subjects. `git range-diff` against the checkpoint reports 372 unchanged, 20 changed, 6 owner-correction commits absorbed, and no new commits outside the audit records. The worktree was clean before this entry.
- No tests or builds were run. Remote publication and its post-fetch verification remain the final gate.
