# Rearrangement TODO

- Locked base: `rust-v0.163.0-alpha.11` (`5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`). User checkpoint: `codex.oops` at `fa08bd2dd52a9d2cd016636d3ecedd2e62013c1f`, tree `5173ba87952048611f8d4e8b561d0dd355ad8e02`.
- Checkpoint reconciliation: the two source-file deviations are restored and match `codex.oops`; the only remaining tracked-tree difference is the intentional `GOAL-rearrange.md` base clarification. See `REARRANGE-DONE.md`.
- Remaining audit: review only commits strictly after the locked base and repeated-path clusters not yet recorded in `REARRANGE-DONE.md`.
- Final gate: complete the downstream audit, verify `alpha` is clean, its merge base is the locked SHA, source content matches `codex.oops`, and no downstream fixups are unattached; then finish the audit trail and publish.
