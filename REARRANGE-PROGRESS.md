# Rearrangement progress

## Ownership issues to resolve

- Commit `5f65909dce` (`feat(tui): Request soft keyboard on prompt tap`) is mixed. It adds the Termux keyboard request and prompt-click behavior, but also changes rollout fixtures to set `ephemeral: false`, adds source/build repository resolution and tests, and adjusts daemon compatibility behavior. During the next rearrangement, classify these changes by behavior and move each to its actual owner; do not assume the whole commit belongs to the keyboard feature.
- Commit `b68509b402` is labeled as a fixup to `5f65909dce`, but it changes the composer paste-target assertion in `owned_transcript_tests.rs` to use the clipboard-paste API introduced by upstream commit `a9b0815167`. Determine whether this test adaptation belongs with the keyboard/prompt-click behavior, the clipboard behavior, or another owner. The test itself was added by `5f65909dce`, so inspect the behavior it covers before deciding.
- Commit `ae29cf9692` (`Keep formatter caches in build tree`) reformats the `TMUX_REQUEST` constant inside `Tui::request_soft_keyboard`. This is an incidental formatting change in the keyboard implementation file, not a semantic formatter-cache change; decide whether to keep it with the formatter-cache change or move it alongside the keyboard implementation while preserving the exact tree.

No history rearrangement has been performed for these findings. Resolve them only during a separately started rearrangement task with the checkpoint and tree-preservation checks in `GOAL-rearrange.md`.
