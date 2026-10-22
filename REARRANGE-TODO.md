# Rearrangement TODO

- Locked base for the next operation: `rust-v0.163.0-alpha.11`, peeled SHA `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. The upstream annotated tag object is `40f4a054d875a76b18a2b733b6224976cf64a826`; the local tag ref remains absent. Do not move the boundary.
- Completed pass: checkpoint `alpha.before-rearrange-20261022-followup-fixups` at `5cbf40cf87f195b0c7c9b99ead51a35801ae3863`, tree `3b7f52bba1896d6478c34e0292d594849d544088`; the rewritten `alpha` tree matched exactly. Eleven fixups were absorbed; see `REARRANGE-DONE.md`.
- Next focused pass: fold `build: support source-tree build symlinks consistently` (`5c6bd3e82c`) and its lock-preservation repair (`c8f4f1993d`) into the source-tree symlink owner `build: allow intentional source-tree build symlink` (`009365f0f5`). Preserve the exact current tree.
- Do not run tests or builds. Verify through Git history, range-diffs, clean-tree checks, fixed merge base, and checkpoint tree equality.
- Publication remains gated: do not rewrite `origin/alpha` until explicitly authorized. The last recorded remote tip is stale; fetch/read the live tip only after authorization and use an exact `--force-with-lease`.
