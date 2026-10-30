# Rearrangement checklist

Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.

Current operation: resolved the annotated tag from `https://github.com/openai/codex.git` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. The earlier claimed checkpoint `alpha.before-rearrange-20261030-resume` (`684c4fdcca4685f5366e809979f60c2704eac2b0`) is absent from local refs and its commit object is unavailable; do not rely on it. Preparatory checkpoint `alpha.before-rearrange-20261030-resume2` points to `dd505a03149201d3f69665385e08d58ad7d665bb`, tree `664939c664432a8e088f147516f6a3d99d54d615`, and does not capture current alpha. At this continuation, clean `alpha` is `908a1e503bbde1d2b48c527a005353b10d673948`, tree `dc22cdeebc19007ea8b6cbcc8d3ab9e06b4add42`; the locked merge-base remains exact. Create a fresh operation checkpoint only after the current semantic/repeated-file audit and its records are ready. The current range has 844 commits and one `fixup!` subject.

Latest ownership review found `feat(app-server): support active turn handoff` and its WIP checkpoint commit contain independent Android-memory, build-tree-symlink, and sccache changes. Split those to their downstream owners and retain the version-only Cargo lock normalization as a scoped standalone commit before any autosquash.

- [ ] Create unique operation checkpoint `alpha.before-rearrange-20261030-corrected` at clean starting commit `ab084c061d930b29047e9e890f61179e60294240` (tree `88c0890f3bfadb97fa0c939cf065a27bd3e28d36`).
- [ ] Finish the 844-commit semantic and repeated-file inventory, including the mixed handoff/WIP commits and high-touch Rust-build guard history.
- [ ] Split `f5567e53099892a52cc0413e6b6ba0d71efde065`, whose compiler-loss fixup subject masks separate notice-transition and periodic-report changes; verify each resulting hunk's owner and preserve independent purposes.
- [ ] Consolidate later repairs/completions with their downstream owners while preserving independent purposes.
- [ ] Retain relevant functionality; record material omissions and unresolved ownership decisions.
- [ ] Rearrange history without running tests/builds or changing source/test contents.
- [ ] Verify clean `alpha`, unchanged locked merge-base, exact checkpoint tree equality, and intended fixup ownership using Git history/range-diffs only.
- [ ] Update `REARRANGE-DONE.md` with final evidence; reduce this file to a one-line completion marker.
