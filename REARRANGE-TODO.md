# Rearrangement TODO

- Locked base: `rust-v0.163.0-alpha.11` (`5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`). User checkpoint: `codex.oops` at `fa08bd2dd52a9d2cd016636d3ecedd2e62013c1f`, tree `5173ba87952048611f8d4e8b561d0dd355ad8e02`.
- Continuation checkpoint: `alpha.before-rearrange-20261019-continuation-audit` at `584a98452e822261030ff4c2839b3773b1aed83b`, tree `2c32e8acac90007b61f285b6b13513baf26bfeed`.
- Checkpoint reconciliation: the two source-file deviations are restored and match `codex.oops`; remaining tracked-tree differences are intentional goal and audit-record updates. See `REARRANGE-DONE.md`.
- Fixed-base audit progress: schema exports, rollout-inspector assets, build/platform paths, daemon PID state, and turn metadata are reviewed; see `REARRANGE-DONE.md`. No misplaced fixup was found in these clusters. The two checkpoint-restoration commits have no downstream owners because their introducing commits are in the locked base.
- Remaining audit: continue reviewing downstream semantic ownership and unrecorded repeated-path clusters strictly after the locked base; the current source tree matches `codex.oops` content-wise.
- Final gate: complete the downstream audit, verify `alpha` is clean, its merge base is the locked SHA, source content matches `codex.oops`, and no downstream fixups are unattached; then finish the audit trail and publish.
