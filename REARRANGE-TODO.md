# Rearrangement checklist

Locked base: `rust-v0.163.0-alpha.11` -> `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`.

Current operation: resolved the annotated tag from `https://github.com/openai/codex.git` to tag object `40f4a054d875a76b18a2b733b6224976cf64a826`, peeled commit `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`. Clean pre-record alpha was `717be43fd2ef9b443352c5efd95e9dcdc96ddc43`, tree `97fd66c08aac37930aa38b56767f66bf8ee2b05d`. New checkpoint `alpha.before-rearrange-20261030-resume` is at `684c4fdcca4685f5366e809979f60c2704eac2b0`, tree `7045441150eaebf4621f65c9a20ea0c80e702bab`; fixed merge-base verified. Initial range inventory: 860 commits, 29 `fixup!`/`squash!` subjects.

- [x] Resolve and lock `rust-v0.163.0-alpha.11` to `5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`; verify clean start and record checkpoint `alpha.before-rearrange-20261030-resume` (`684c4fdcca4685f5366e809979f60c2704eac2b0`, tree `7045441150eaebf4621f65c9a20ea0c80e702bab`).
- [ ] Inventory every downstream commit strictly after the locked base; classify mixed and repeated-file changes by behavior owner.
- [ ] Validate every `fixup!` target; split cross-owner changes and retarget where needed.
- [ ] Consolidate later repairs/completions with their downstream owners while preserving independent purposes.
- [ ] Retain relevant functionality; record material omissions and unresolved ownership decisions.
- [ ] Rearrange history without running tests/builds or changing source/test contents.
- [ ] Verify clean `alpha`, unchanged locked merge-base, exact checkpoint tree equality, and intended fixup ownership using Git history/range-diffs only.
- [ ] Update `REARRANGE-DONE.md` with final evidence; reduce this file to a one-line completion marker.
