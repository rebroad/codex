# Rearrangement checklist

Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.

Current operation: resolved the annotated tag from `https://github.com/openai/codex.git` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. The earlier claimed checkpoint `alpha.before-rearrange-20261030-resume` (`684c4fdcca4685f5366e809979f60c2704eac2b0`) is absent from local refs and its commit object is unavailable; do not rely on it. Current clean `alpha` is `dd505a03149201d3f69665385e08d58ad7d665bb`, tree `664939c664432a8e088f147516f6a3d99d54d615`. Before rearranging, create and verify a fresh checkpoint at this exact commit/tree. Current range inventory: 837 commits.

Latest ownership review found `feat(app-server): support active turn handoff` and its WIP checkpoint commit contain independent Android-memory, build-tree-symlink, and sccache changes. Split those to their downstream owners and retain the version-only Cargo lock normalization as a scoped standalone commit before any autosquash.

- [ ] Create the pre-recorded unique checkpoint `alpha.before-rearrange-20261030-resume2` at clean starting commit `dd505a03149201d3f69665385e08d58ad7d665bb` (tree `664939c664432a8e088f147516f6a3d99d54d615`); prior claimed checkpoint is missing and unusable.
- [ ] Finish the 860-commit semantic and repeated-file inventory, including the mixed handoff/WIP commits and any split-owner edits.
- [ ] Verify every fixup hunk against its behavior owner; split/retarget the mixed commits and preserve independent historical purposes.
- [ ] Consolidate later repairs/completions with their downstream owners while preserving independent purposes.
- [ ] Retain relevant functionality; record material omissions and unresolved ownership decisions.
- [ ] Rearrange history without running tests/builds or changing source/test contents.
- [ ] Verify clean `alpha`, unchanged locked merge-base, exact checkpoint tree equality, and intended fixup ownership using Git history/range-diffs only.
- [ ] Update `REARRANGE-DONE.md` with final evidence; reduce this file to a one-line completion marker.
