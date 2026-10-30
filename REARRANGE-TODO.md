# Rearrangement checklist

Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.

Current operation: resolved the annotated tag from `https://github.com/openai/codex.git` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. The earlier claimed checkpoint `alpha.before-rearrange-20261030-resume` (`684c4fdcca4685f5366e809979f60c2704eac2b0`) is absent from local refs and its commit object is unavailable; do not rely on it. Preparatory checkpoint `alpha.before-rearrange-20261030-resume2` points to `dd505a03149201d3f69665385e08d58ad7d665bb`, tree `664939c664432a8e088f147516f6a3d99d54d615`, and does not capture current alpha. The 2026-10-30 audit snapshot at `9b1f1de5f9e869a595cd462b971009122500e292` had tree `e0d0e6ed0c7296b83270a569cb3f535110fec91d`, clean worktree, and the locked merge-base. Create a fresh operation checkpoint after the semantic/repeated-file audit and its records are ready. At that snapshot, the range had 848 commits and one `fixup!` subject.

Latest ownership review found `feat(app-server): support active turn handoff` and its WIP checkpoint commit contain independent Android-memory, build-tree-symlink, and sccache changes. Split those to their downstream owners and retain the version-only Cargo lock normalization as a scoped standalone commit before any autosquash.

- [ ] Finish the full fixed-base semantic and repeated-file inventory, including mixed handoff/WIP commits, the linked Android Rust-build files, and other high-touch paths such as `scripts/build_codex.sh` (31 first-parent commits).
- [ ] Create a new uniquely named operation checkpoint at the clean reviewed `alpha` tip; record its exact commit and tree before rearranging refs or commits.
- [ ] Split `f5567e53099892a52cc0413e6b6ba0d71efde065`, whose compiler-loss fixup subject masks separate notice-transition and periodic-report changes; verify each resulting hunk's owner and preserve independent purposes.
- [ ] Split `84158b998a`, whose `scripts/codex_android_ballast.py` changes extend both release/refill measurements and PSI-gated recovery refill behavior; assign each hunk to its behavior owner before consolidation.
- [ ] Consolidate later repairs/completions with their downstream owners while preserving independent purposes.
- [ ] Retain relevant functionality; record material omissions and unresolved ownership decisions.
- [ ] Rearrange history without running tests/builds or changing source/test contents.
- [ ] Verify clean `alpha`, unchanged locked merge-base, exact checkpoint tree equality, and intended fixup ownership using Git history/range-diffs only.
- [ ] Update `REARRANGE-DONE.md` with final evidence; reduce this file to a one-line completion marker.
