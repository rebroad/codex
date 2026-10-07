# Alpha rebase onto alpha.18 (2026-10-07)

## Rebase provenance

- Rearranged `alpha` started clean at `88613755eeab062efe01e12e912d8423a4ad8ff9`, tree `572ece22c54ab85b7e92b5f00eb7fc97df437035`.
- The `alpha.before-rearrange-20261007-post-audit` checkpoint was `efe390b7a29f313796c0c2d6bda03f1fd0877477` with the same tree. New backup `alpha.before-rebase-20261007-alpha18` points to the original `alpha` start commit and tree.
- Refreshed target: `upstream/latest-alpha-cli` at `cedb4fd34c54cf81f44bfeabb000c7cc01eced82` (`Release 0.162.0-alpha.18`). The source was shallow; fetching the full target history established the prior series base at `be48ae396e57621cee374423a19c2f427e1b4373`.
- Replayed 113 downstream commits. New `alpha` begins directly on the target; its current merge base is the target SHA above.
- Dropped `635a0c0b13` (alpha.9 release, superseded by alpha.18) and `30fde1fa85` (obsolete upstream-context snapshot refresh; used current target snapshots instead).
- Conflict resolutions retained the downstream release-channel workflows alongside upstream version logic; latest Android lock behavior; reconnect state test; resume-progress loading in the current recorder API; task availability rules; Linux sandbox availability handling; personality metadata; and apply-patch sandbox-denied escalation. User explicitly approved disabling updater installer updates and keeping daemon-restart handoff on 2026-10-07.

## Post-rebase repairs and generated files

- Regenerated config schema and stable plus experimental app-server schema exports after replay. The app-server fixture generation first found an upstream rollout helper that was only referenced in tests but compiled in the production library; scoped the helper and its imports to test builds and removed an unused re-export.
- Adapted downstream apply-patch escalation to the current upstream API, which removed the line-ending update-mode type. Kept the sandbox-denied action path and used upstream's unconditional line-ending-preserving update behavior.
- Adapted the model-cache test fixture to current `ModelMessages` fields; added the new `root_turn_id` field in a TUI test; removed an obsolete status-card import; retained Archive's upstream availability during active tasks while keeping the downstream Wake policy.

## Validation

- `just write-config-schema`: passed.
- `just write-app-server-schema` and `just write-app-server-schema --experimental`: passed after the rollout helper repair.
- `just test -p codex-app-server-protocol -p codex-rollout --config-file /var/tmp/codex-nextest-local.toml`: 477 passed, 1 skipped. The temporary nextest profile raised the slow timeout for the experimental schema comparison; stable and experimental fixture comparisons passed.
- `just test -p codex-apply-patch`: 99 passed.
- `just test -p codex-tui slash_command`: 189 passed, 5,556 skipped.
- Full `just test -p codex-tui` compiled after repairing the API mismatches but did not pass. Failures included stale snapshots (including startup layout spacing and fork-vs-upstream issue URLs), sandbox/Unix-socket dependent cases, and subprocess cases. The full TUI suite has not been rerun after the final slash-command repair; the focused slash-command tests pass.
- Android release build/install to Flip7 succeeded before the user redirected the required final validation to native Linux. It is not counted as the requested native/local validation.
- Native `scripts/build_codex.sh --release` compiled the release binary. Default local install was denied by the sandbox's read-only Cargo bin path. Re-running with `INSTALL_BIN_DIR=/var/tmp/codex-native-bin` built and installed there; its graceful daemon-restart handoff then encountered the sandbox's read-only daemon lock. The binary itself ran locally.
- Local `/var/tmp/codex-native-bin/codex exec --bare-prompt --json 'say hi'` returned “Hi!” but reported 28,778 input + 6 output tokens. The direct variant, `codex exec --direct --bare-prompt --json 'say hi'`, returned “Hi!” with 8 input + 6 output (14 total). Thus the exact non-direct command does not meet a strict under-20-token total; the direct bare-prompt path does. Existing source tests verify that bare-prompt requests contain only the user prompt and no tools, so the excess on the normal session path is server-side context.
- `git diff --check upstream/latest-alpha-cli..alpha` reports trailing spaces in line 8 of the startup-layout snapshot from the target/replayed snapshot; inspect/refresh snapshots before claiming the full TUI suite is clean.
- Repair fixups were autosquashed into their intended downstream commits. The autosquash replayed 120 entries and preserved the tree exactly: `c45909a665793ee200fb4f8e191c0bd3dcd5fe6b` before and after. The only standalone follow-ups are the rollout helper compile fix and this validation note; no `fixup!` commits remain.
- Final local branch status is `ahead 265, behind 115` relative to `origin/alpha`. A normal `git push origin alpha` was rejected as non-fast-forward; no force push was attempted.

## Remaining gates

- Review and reconcile the full TUI snapshot differences and rerun the relevant TUI test set.
- The exact non-direct local bare-prompt command remains above 20 total tokens; using `--direct` is the verified 14-token path.
- The source branch is divergent from `origin/alpha` (`ahead 265, behind 115`). Do not force-push; the normal push was rejected. The rewritten branch and follow-up commits remain local until an authorized publication strategy is chosen.
