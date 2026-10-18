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
