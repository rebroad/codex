# Active rearrangement checklist

- Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3` (tag object `40f4a054d875a76b18a2b733b6224976cf64a826`).
- Checkpoint: `alpha.before-rearrange-20261030-record-churn-01` at `793fdaf69af663e4a669c436dd46849884d32b98`, tree `d6e1f60da392c90144d8a7eed0d5490542ce82fd`.
- [x] Remove historical downstream edits to TODO/DONE and prune record-only commits.
- [x] Verify the fixed base, unchanged source tree outside TODO/DONE, no merges or fixups, and the range-diff against the checkpoint.
- [ ] Transfer the rewritten history into the source checkout as a new publication branch.
- [ ] Publish that new branch and verify its remote tip and clean worktree.
