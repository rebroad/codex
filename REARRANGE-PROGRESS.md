# Rearrangement progress

## Ownership issues to resolve

- Commit `5f65909dce` (`feat(tui): Request soft keyboard on prompt tap`) is mixed. It adds the Termux keyboard request and prompt-click behavior, but also changes rollout fixtures to set `ephemeral: false`, adds source/build repository resolution and tests, and adjusts daemon compatibility behavior. During the next rearrangement, classify these changes by behavior and move each to its actual owner; do not assume the whole commit belongs to the keyboard feature.
- Commit `b68509b402` is labeled as a fixup to `5f65909dce`, but it changes the composer paste-target assertion in `owned_transcript_tests.rs` to use the clipboard-paste API introduced by upstream commit `a9b0815167`. Determine whether this test adaptation belongs with the keyboard/prompt-click behavior, the clipboard behavior, or another owner. The test itself was added by `5f65909dce`, so inspect the behavior it covers before deciding.
- Commit `ae29cf9692` (`Keep formatter caches in build tree`) reformats the `TMUX_REQUEST` constant inside `Tui::request_soft_keyboard`. This is an incidental formatting change in the keyboard implementation file, not a semantic formatter-cache change; decide whether to keep it with the formatter-cache change or move it alongside the keyboard implementation while preserving the exact tree.
- The `codex-rs/tui/src/chatwidget/interaction.rs` change for pausing goals before interrupting belongs to its own commit: it was originally `a027787cab` (`fix(tui): pause goals before interrupting`) and is replayed here as `34c4d3d12f`. This behavior is unrelated to Android or clipboard support, so keep it separate during rearrangement. Note: `f4193ee1dfa` currently resolves to `fix(android): scope clipboard APIs to supported targets` and does not modify `interaction.rs`; verify the intended owner if another `f4193ee1dfa` change was meant.

No history rearrangement has been performed for these findings. Resolve them only during a separately started rearrangement task with the checkpoint and tree-preservation checks in `GOAL-rearrange.md`.

## Active rearrangement starting point (2026-10-18)

- Branch: `alpha`; starting commit: `ed5dcf09fb1fcb5ccdeecec3a9057210b2471131`; starting tree: `5a210bf77dbab944213c7b4abd22cf9728ff4f58`.
- Upstream merge base: `e53e932dc855927afb1cafab5f1ae6edf2695e81`.
- Worktree was clean. The existing `alpha.before-rearrange-20261015-pass1` checkpoint is an earlier state and does not match this starting tree. Create a new unique checkpoint for this exercise before any history rearrangement.
- Current ownership questions above remain open; the downstream range contains hundreds of commits, so work must proceed in audited, tree-preserving passes with this record updated as each pass is verified.

## Fixup target scan

- The current range has 928 commits, including 216 subjects beginning `fixup!` (the additional commit is the active progress record).
- A subject-based scan found seven fixups with no downstream commit whose subject matches their target: `4bb90c6f23`, `e7202d91ba`, `0914bba603`, `eab0e9d92a`, `979241d5fe`, `f97c075b69`, and `332f210d60`. All target the upstream `Support projectless TUI sessions with workspace defaults (#49160)` commit, which is at or before the merge base and cannot own downstream fixups.
- Diff and parent-chain inspection indicates distinct outcomes: `4bb90c6f23` plus `0914bba603` produce a projectless-config behavior change; `e7202d91ba` adds temporary diagnostic output that `0914bba603` removes; `eab0e9d92a` and `979241d5fe` cancel each other's code change; `f97c075b69` and `332f210d60` are test adjustments around background-terminal lookups. Keep the behavior and test purposes in separate commits, and discard only the zero-net temporary experiments after confirming the combined tree effect during the rebase.
- Candidate ownership for the config behavior and test updates is not an existing downstream feature commit: the relevant symbols/tests are introduced by the upstream projectless-session commit. Treat these as clearly scoped standalone downstream changes, not fixups to the upstream commit. Verify the exact rebase result and checkpoint tree before considering this pass complete.

## Fixup target pass 1

- Start checkpoint: `alpha.before-rearrange-20261018-fixup-audit` at `94a16cf9639da135e32762267fa60e8bc990cc24`, tree `102f6663f2bddccd01d0324d9432ed079fd0f013`.
- Kept the net projectless-config behavior as standalone commit `fa990b1c0832fed76325a0e043428b22005abcc3` (`fix(tui): honor projectless server settings`); diagnostic add/remove and background-terminal code add/revert pairs were dropped. Folded the two test adjustments into one commit, but its subject still incorrectly names the upstream owner; rename it in the follow-up checkpointed correction.
- Verification: rebase completed without conflicts; resulting tree `102f6663f2bddccd01d0324d9432ed079fd0f013` matches the checkpoint tree exactly; merge base remains `e53e932dc855927afb1cafab5f1ae6edf2695e81`. The series is now 924 commits. This pass changed history only; no source-tree behavior changed, so tests were not rerun.
- `alpha` now diverges from `origin/alpha` due to the authorized history rewrite. Do not publish until the full rearrangement audit is complete and the expected remote tip is reverified.
- Follow-up correction renamed the remaining test adaptation to `c7375af992aa2c11b4c0d78b2eaa46550701302e` (`test(tui): track background terminal lookups`). Verification against the same checkpoint still passes; merge base remains unchanged.
- Re-scan after the pass: 925 downstream commits remain, with 209 `fixup!` commits; all 209 have a matching downstream subject, so there are no remaining subject-level targets that point only to commits outside the downstream range. This is a screening result, not yet semantic approval of every target.
- Next ownership pass: resolve the mixed keyboard/source-build/fixture/daemon commit `5f65909dce` by behavior and inspect each candidate owner's history before splitting or attaching changes.

## Mixed keyboard commit ownership

- `5f65909dce` combines four behaviors. Keep only the six keyboard implementation and click-ownership test files in `feat(tui): Request soft keyboard on prompt tap`.
- Move `scripts/resolve_source_repo.py` and `scripts/test_resolve_source_repo.py` changes into a fixup of `bb5dfd5fd905e98a2d8bcaf493f01d8f0a9e2a1a` (`build: resolve source and external build trees`). Move the `justfile` call-site change separately into a fixup of `ce23ac328c96aae5c8e2b22e86b1249054d5b561` (`build: route just recipes through build trees`), since that historical commit owns routing just recipes while the former owns the resolver implementation.
- Move the two `ephemeral: false` rollout fixture fields into a fixup of `41d3a76d880cb0258f87d7d603caee2bbcf5f9a3` (`feat(core): persist response usage attribution`), which introduced the required metadata field.
- Move the `CompatibilityError` test construction into a fixup of `e6b0f44733973aface123ed2bd1513367810c9e6` (`fix(tui): fall back for unmanaged daemon mismatch`), which introduced that error type and its `restart_features` field.
- Keep `b68509b402` as a fixup to the keyboard feature: although the assertion uses the clipboard-paste API introduced later upstream, it still checks that the prompt click remains owned by the composer and retains its paste target. This is part of the added keyboard-click coverage, not an independent clipboard feature.
- The four separated fixups have different owners and must remain distinct until autosquashed; do not combine them by file count or commit count.
- Completed as a checkpointed split/autosquash pass. The keyboard commit now contains only its six keyboard-related files; the resolver implementation, justfile routing call-site, rollout fixture defaults, and daemon test adaptation were each folded into their separately identified owner commits. The existing keyboard test-adaptation fixup and the two in-range daemon fixups were also folded into their owners.
- Verification against `alpha.before-rearrange-20261018-keyboard-split` (`e73639ad8a1cf25ae035ebfc5b05de96994be4c0`, tree `5762944e8e7ef54ce938114f6cafeca616901a14`) passed: final tree matches exactly; merge base remains `e53e932dc855927afb1cafab5f1ae6edf2695e81`; the series is 924 commits. The subject-level scan finds 206 remaining fixups, all with a matching downstream subject. Semantic ownership for those remaining fixups is still unverified; continue auditing rather than treating subject matches as proof.
- This was a history-only pass and preserved the checkpoint tree, so no tests were rerun.

## Semantic audit: rebase-record fixup cluster

- Of the 50 fixups targeting `docs: split rebase todo and done records`, 46 touch only `REBASE-TODO.md` and/or `REBASE-DONE.md`; their subjects and content are coherent with that owner.
- Split the `AGENTS.md` log-capture correction in `4cef03e7f72828f18dce6f583b7c6a322795e3ac` from the rebase-record work. Its follow-up `90a7ef74c48a2fff209ec799e6d9bc1735391d1e` edits the same guidance. Both update the build-log instruction introduced in `17934bfff5dfecec89b2b7aa8dfe230e7854d765` and should target that owner.
- Split the `DOWNSTREAM.md` reconnect-behavior row from `5f064e40cc5ea3ade4bbfbc94fc5374c191c2638`; its `REBASE-TODO.md`/`REBASE-DONE.md` hunks remain with the record-splitting owner. The reconnect row belongs with the `DOWNSTREAM.md` authority introduced by `e5aa501b03f3046f5b17662303c87727c9a34dba`.
- `1742e12db87a557dc0b2ff25af12dbea1d36b9e4` also mixes reconnect code/tests with the initial `DOWNSTREAM.md` row. Keep its code/tests as a fixup to `feat(app-server-client): reconnect remote clients` (`4bd600882a2d88bd0d9e105e0e72083b1ad0741b`), and move the documentation row to a separate fixup to `docs: define downstream fork authority` (`e5aa501b03f3046f5b17662303c87727c9a34dba`). The later row update from `5f064...` should target the same documentation owner.
- Keep the `GOAL-rebase.md` completion-rule update in `e3def2f5fcbf44b1e289527a608ab175aeb6a027` with the record-splitting owner: the rule text it changes was introduced by that owner.
- Next pass: split those cross-purpose hunks and autosquash only the audited fixups. Then verify the checkpoint tree and merge base again.
- More specifically, `DOWNSTREAM.md` does not exist yet in the `feat(app-server-client): reconnect remote clients` commit's tree, while the document-authority commit creates it. Therefore keep code/tests in that feature fixup, but target both the new reconnect row and its later wording update at `docs: define downstream fork authority`; this makes the doc patches applicable at their true artifact owner.
- The two AGENTS log-guidance commits are independent of the rebase-record split and both belong with the original AGENTS guidance owner. The docs-split fixup should retain only its rebase-record content.
- These splits are audited but not yet applied; the current tree remains checkpointed before this pass.
