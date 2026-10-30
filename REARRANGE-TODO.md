# Rearrangement checklist

Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.

Current operation: resolved the annotated tag from `https://github.com/openai/codex.git` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. Fresh checkpoint `alpha.before-rearrange-20261030-audit-final-01` is locked at commit `f30481b83ce7fd03972e4c56f8d1750019b0c89d`, tree `6978130716d52ac445bfb73b2abb6a868f1a1e25`; merge-base is the locked SHA. Checkpoint evidence is in `REARRANGE-DONE.md`. Current `alpha` is `71550b9972b473a2a00c586cc2dca829b2de2c50`, tree `6414a597c48b027895f99318b1582dd56d98f881`, after a DONE-only record commit. The range has 910 commits and no merges. Earlier claimed checkpoint refs remain invalid and unused.

Historical correction: reviewed handoff commits `f9b4603fb1` and `638dca833f` are object-only, not ancestors of current `alpha`. Reachable handoff commit `72325dcdbb` is focused; its test-build selection and deleted-worktree cleanup are handoff behavior. Android memory preemption, build-tree routing, symlink resolution, and in-process sccache have separate reachable owners. See the correction in `REARRANGE-DONE.md`; do not perform the obsolete split or retain the unreachable lock-normalization plan.

Resolved two more owner paths: the explicit source-tree symlink commit `e85b55b012` extends downstream resolver `5e8d8ddaac` and is queued as its fixup; package-root discovery `e2ad84f1db` replaces a `CODEX_REPO_ROOT` requirement already present at the locked base, so it remains standalone rather than targeting upstream history.

- [x] Finish semantic verification of the fixed-base inventory: all 107 paths with three or more touches and all 127 exactly-two-touch paths are path-mapped in `REARRANGE-DONE.md`; mixed-commit ownership and Android build cross-file decisions are recorded before replay.
- [ ] Create a new uniquely named operation checkpoint at the clean reviewed `alpha` tip; record its exact commit and tree before rearranging refs or commits.
- [ ] Split `f5567e53099892a52cc0413e6b6ba0d71efde065`: move notice-state/reporting hunks to `7b85ac6aab`, compiler stop/resume accounting to `b9bb6b1aaf`, and account for the periodic-report removal from `895d7fc67f` while preserving its later reintroduction/policy in `79c3b52ced`.
- [ ] Split `84158b998a`: move release/outcome and refill-opportunity telemetry to `a3d94ed9d5`, carrying in the `atomic_json` helper and `--session-stats-file` option from `1f9500a2af` because `84158b998a` first consumes them; keep adaptive target rebasing and PSI/cooldown-gated refill with the recovery owner `1f9500a2af`. Route later telemetry repairs (`02dd2cb1b1`, `c2ec760309`, `8b892b4b46`) and recovery-policy repair (`3d5eac4916`) by hunk, preserving separate tuning owner `32ae43371b`.
- [ ] Split `719ac665e3` in the rollout inspector: `recordCatalog`, omission summaries/coverage, and token-usage row rendering continue ordinal usage owner `056b383b35`; bounded structured tool-output decoding and safe preview rendering remain a separate new behavior.
- [ ] Split reachable mixed commit `0674f2749c`: move managed-daemon remote-control default-enabled settings, deserialization, tests, and README into their own downstream feature commit; retain dependency/Cargo/transport graph trimming with the transport owner. The `DOWNSTREAM.md` preservation row is a separate docs-only commit (`cf06f4855c`), so verify its placement without attributing it to `0674`. Ignore unreachable historical object `0718e7bec9` when assigning current-alpha ownership.
- [ ] Continue the all-path repeated-file audit for paths with three or more touches that are not yet covered by `REARRANGE-DONE.md`.
- [ ] Consolidate later repairs/completions with their downstream owners while preserving independent purposes.
- [ ] Retarget `fca975f429` as a fixup to `17845ddb71`; its only changes are imports needed by that sandbox implementation and full-access fixture adaptation.
- [ ] Remove the fully reverted markdown snapshot URL refresh `5ea87e2287` from the final series and omit its exact inverse hunk from branding commit `bdbe346e0e`; preserve the current checkpoint blob and all other branding changes.
- [ ] Retarget `e85b55b012` as a fixup to `5e8d8ddaac`; its source symlink handling, sync/build/Cargo adjustments, and tests all extend the resolver's source/build-tree contract.
- [ ] Retain relevant functionality; record material omissions and unresolved ownership decisions.
- [ ] Rearrange history without running tests/builds or changing source/test contents.
- [ ] Verify clean `alpha`, unchanged locked merge-base, exact checkpoint tree equality, and intended fixup ownership using Git history/range-diffs only.
- [ ] Update `REARRANGE-DONE.md` with final evidence; reduce this file to a one-line completion marker.
