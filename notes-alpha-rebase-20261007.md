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
- `just test -p codex-tui --retries 0` passes natively with the inherited `TERM=dumb` and `NO_COLOR=1` environment: 5,737 passed, 8 skipped. Bubblewrap and Unix-socket tests require the escalated shell; no terminal environment overrides are needed after making cursor snapshots branch on color support.
- Keep `RUST_MIN_STACK=16 MiB` for local tests. The original 8 MiB upstream baseline still stack-overflows in at least three current TUI tests (`overview_selection_applies_user_permissions_only_to_unloaded_threads`, `side_fork_skips_parent_title_lookup_but_normal_ephemeral_fork_keeps_it`, and `startup_resume_and_fork_use_configured_or_explicit_cwd`). Their stack failures reproduce at 8 MiB; the full TUI suite passes at 16 MiB. Localized future boxing remains possible, but would require independent changes to several large test futures and would repeat the per-test repair pattern the upstream baseline was intended to avoid.
- Android release build/install to Flip7 succeeded before the user redirected the required final validation to native Linux. It is not counted as the requested native/local validation.
- Native `scripts/build_codex.sh --release` compiled the release binary. Default local install was denied by the sandbox's read-only Cargo bin path. Re-running with `INSTALL_BIN_DIR=/var/tmp/codex-native-bin` built and installed there; its graceful daemon-restart handoff then encountered the sandbox's read-only daemon lock. The binary itself ran locally.
- The non-direct local bare-prompt command initially returned “Hi!” with 28,778 input + 6 output tokens (28,416 cached). Although the session request contained only the user prompt, server-side session handling added the cached context. Top-level bare-prompt now uses the isolated request path automatically, without requiring `--direct`; configured `model_instructions_file` and `developer_instructions` are retained while default model instructions are omitted. Focused tests pass for both prompt-only and explicit-instruction cases. The rebuilt native binary returned “Hi!” with 8 input + 6 output tokens (14 total).
- `just test` rebuilds/checks `codex-cli` with Cargo before nextest, preventing a stale `target/debug/codex` from another checkout from being selected by TUI tests. The native build reports `0.162.0-alpha.18`. The locally installed CLI and running managed daemon are still `0.162.0-alpha.9`; the daemon reports no saved feature overrides, and local `config.toml` has no explicit feature toggles.
- Kept the slash-composer snapshot's `v0.0.0` stable via the test version normalizer. The Vim empty-slash fixture now has two extra rows so it retains the full `/ide` description and `/permissions` suggestion when `/wake` is also shown.
- Terminal cursor tests now capture color-enabled and `NO_COLOR` output separately and assert the behavior selected by the inherited environment. Both the default environment and the full default-environment TUI suite pass.
- The daemon recovery heading was first added by upstream `d6093d3228`. Downstream `b1ae712eef` narrows the modal to a daemon freshly started by this invocation; a mismatch on an already-running daemon is reported non-blockingly. The local stale daemon is a potential source of feature differences, but the exact mismatching feature was not captured from a reproduction.
- `just fix -p codex-tui` completed successfully after the final passing suite; no tests were rerun afterward. Clippy reported existing `if_same_then_else` and `too_many_arguments` warnings in `status_indicator_widget.rs` and `lib.rs`.
- Repair fixups were autosquashed into their intended downstream commits. The autosquash replayed 120 entries and preserved the tree exactly: `c45909a665793ee200fb4f8e191c0bd3dcd5fe6b` before and after. The only standalone follow-ups are the rollout helper compile fix and this validation note; no `fixup!` commits remain.
- Final local branch status before these latest validation/snapshot follow-ups was `ahead 267, behind 115` relative to `origin/alpha`. A normal `git push origin alpha` was rejected as non-fast-forward; no force push was attempted.

## Remaining gates

- The source branch is divergent from `origin/alpha` (`ahead 269, behind 115`). Do not force-push; the prior normal push was rejected. The rewritten branch and follow-up commits remain local until an authorized publication strategy is chosen.

## 2026-10-08 continuation

- Follow-up validation repairs were committed as 13 fixups against downstream owners; the wrapper-default fix targets `4cc1ddb1e1` (`build: add shared Cargo and linker harness`). Before autosquash, the local tip was `15137cb2aa` with tree `07c5e43efe6aa3bdac902414a825e76ec23193f8`; safety ref `alpha-pre-autosquash-20261008-07c5e43` preserves it.
- `scripts/codex_cargo_env.sh` now clears Cargo's configured `RUSTC_WRAPPER` by default, including when `CARGO_INCREMENTAL=0`. Sccache is opt-in with `CODEX_CARGO_ENABLE_SCCACHE=1` and only activates for non-incremental builds. Verified generated environments in default incremental, default non-incremental, and explicit opt-in cases; `bash -n` passed.
- The full alpha.18 upstream test attempt compiled until `gstreamer-sys` required GStreamer >=1.28. The host has no GStreamer development package, and configured Debian packages provide 1.26.2, so the full baseline suite remains environment-blocked; do not interpret this as a downstream regression.
