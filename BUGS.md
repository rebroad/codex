# Known bugs

## Source lockfile restoration failure can leave a successful build with a modified checkout

In `scripts/build_codex.sh`, `cleanup_build_process` is registered as an `EXIT` trap. When restoring `${SOURCE_REPO}/codex-rs/Cargo.lock` from `${SOURCE_LOCK_BACKUP}`, a failed `cp -p` only emits a warning. Cleanup then continues without explicitly failing an otherwise successful build. Consequently, a source-tree build may report success while leaving its original lockfile unrestored. The backup is retained on this failure path.

This failure path was established by source inspection and is also described in `POTENTIAL-ISSUES.md`. It has not been reproduced through execution. A subsequent fix should propagate restoration failure without losing the original build failure status or recovery backup. No source changes, tests, or builds were performed during the rearrangement.
