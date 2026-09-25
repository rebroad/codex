# Alpha rebase: skipped commit audit

This note records commits from the earlier `alpha` line that were not replayed
as separate commits during the rebase onto `upstream/latest-alpha-cli`
(`65757fb7dbaafe15a902433cf1a4413d766f316d`). A skipped commit is not treated
as lost functionality until its tree effect has been checked against the
resulting branch.

| Skipped commit | Reason and retained behavior |
| --- | --- |
| `3cbb58fb5fa08e12032d7bcc6f1749705a32afec` — `build(android): document hybrid builds` | The commit's actual diff adds only `codex-rs/cli/src/android_tls_alignment.rs` (the subject/body mention additional build documentation, but those files are not in this commit's patch). The same module is present on current `alpha`, introduced by `9b92eb05a9` (`fix(android): support sandbox and TLS alignment`). Its code effect was therefore retained under the later Android compatibility cluster; replaying this commit would duplicate it. |
| `7067c9194c21a4fb5f4700db50c1cb10bd6cb0d6` — `Reduce status indicator animation frequency` | Its 32 ms scheduling change was not replayed separately because current `alpha` already implements the intended 200 ms cadence in `a86b4d3bca` (`feat(tui): show wait countdown`): active progress/shimmer animation and waiting countdown schedule at 200 ms, while the idle timer uses 1000 ms. `active_animation_schedules_at_most_five_frames_per_second` asserts the active delay is at least 200 ms. This behavior is intentional for Termux CPU use and must remain. |

These are intentional non-replays of duplicate tree effects, not removals of the
Android TLS alignment or the 200 ms animation cap. Recheck this note whenever
the corresponding implementation changes during a later upstream rebase.
