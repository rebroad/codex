# Rearrangement audit

## Record-churn consolidation

- Locked `rust-v0.163.0-alpha.11` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.
- The pre-operation `alpha` tip was `f8c796d3938fb3d07e61a68e1c3a8718262bba52`; its 939-commit downstream range had no merges or fixup/squash subjects, and 257 commits touched DONE while 95 touched TODO.
- Checkpoint `alpha.before-rearrange-20261030-record-churn-01` is `793fdaf69af663e4a669c436dd46849884d32b98`, tree `d6e1f60da392c90144d8a7eed0d5490542ce82fd`. It includes the current clarified goal and the separate operation-baseline record commits.
- Replayed only commits after the locked base with an index filter removing changes to TODO/DONE. The resulting branch has 595 commits, no merges, no fixup/squash subjects, and no historical TODO/DONE touches before these final audit commits.
- `git range-diff` pairs 595 commits identically and reports 346 old-only commits, with no changed or new pairs. Those 346 commits are the record-only edits removed from history.
- The resulting source tree matches the checkpoint for every path except the two audit records. The successful replay's pre-record tip is `f9e82d3b0903fce88a5fa267464803ffa78f35b4`, tree `288d8d99192f3c49d03ef76efa315a8dfb80a979`; its merge-base is exactly the locked commit.
- A separate `git-filter-repo` trial was discarded after it changed the merge-base in its isolated clone. It did not alter the source checkout, checkpoint, `alpha`, or the already-published rearrangement branch.
- No source-code edits, builds, or tests were made; this goal is Git-history-only.

## Remaining gate

Transfer the verified branch to the source checkout, publish it under a new remote branch, and confirm the remote tip. Existing `alpha` and `alpha.rearrange-20261030` remain untouched.
