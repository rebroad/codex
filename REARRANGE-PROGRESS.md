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
- Current ownership questions above remain open; the downstream range contains 927 commits, so work must proceed in audited, tree-preserving passes with this record updated as each pass is verified.
