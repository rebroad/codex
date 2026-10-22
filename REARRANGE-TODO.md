# Rearrangement TODO

- Locked base: `rust-v0.163.0-alpha.11` (`5425d3e625daa58eea3ad1de27c087eb3b2ffbd3`). The local tag ref is absent; authoritative upstream `ls-remote` resolved the annotated tag to this exact peeled commit. Do not move this boundary.
- New work since the last completed local audit (`3ab33f57d7694ab9495e2333707d1e289b893f35`): 31 commits, including 11 `fixup!` commits. Audit every fixup against its downstream owner, verify that the 20 other commits remain independently scoped, then autosquash valid repairs.
- The audited suffix contains new source features, so the old `codex.oops` equality check is historical evidence only. Preserve the exact current tree at the new operation checkpoint.
- Do not run tests or builds. Verify only Git history, owner/range diffs, clean status, fixed merge base, and exact checkpoint tree equality.
- Publication remains gated: do not rewrite `origin/alpha` until the user explicitly authorizes it. The last recorded fetched tip was `460e43ac4b6491749f25d9b6ca135af6720cd88a`; verify the live remote and use an exact `--force-with-lease` if publication is authorized.
