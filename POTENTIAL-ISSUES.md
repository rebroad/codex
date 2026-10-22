# Potential Issues

This file records code concerns noticed during the history-only rearrangement. Do not fix or validate them until the rearrangement goal is complete.

- `scripts/build_codex.sh:244-253`: `cleanup_build_process` attempts to restore the source `Cargo.lock` from its backup, but on copy failure it only prints a warning and continues cleanup. Because this runs from an `EXIT` trap, a successful build may still exit successfully while leaving the source lockfile normalized or modified. Consider making restoration failure affect the final exit status, while preserving the original build status and backup for recovery. This is a potential failure-path issue; it was not reproduced or tested under the no-build constraint.
