# Handoff: unrelated test failures

This is a handoff for the agent investigating failures observed while validating
the app-server/TUI session-sharing change. The failures below are outside the
changed ownership, endpoint, and TUI attach paths. Reproduce them before making
changes; several appear to be caused by this execution environment rather than
by product regressions.

## Reproduction environment

- Source checkout: `/mnt/kingston/@home/rebroad/src/codex`
- Build checkout: `/home/rebroad/src/codex.build` (the real path is under
  `/mnt/kingston/@home/rebroad/src/codex.build`)
- Run Rust commands from `/home/rebroad/src/codex.build/codex-rs`.
- Sync source first when needed:
  `cpto --no-lngit /mnt/kingston/@home/rebroad/src/codex /home/rebroad/src/codex.build`
- Tests were run with:
  `RUSTC_WRAPPER= CARGO_NET_OFFLINE=true just test ...`

The saved raw logs from the original run are in `/var/tmp` while this machine
remains available:

- `/var/tmp/codex-app-server-test.log`
- `/var/tmp/codex-cli-compat-test.log`
- `/var/tmp/codex-app-server-transport-test.log`

## App-server suite

Command:

```text
RUSTC_WRAPPER= CARGO_NET_OFFLINE=true just test -p codex-app-server
```

Result: 1,188 passed, 33 failed, 2 skipped. The failures were:

- `outgoing_message::tests::cancel_requests_for_thread_cancels_all_thread_requests`
- `outgoing_message::tests::pending_requests_for_thread_returns_thread_requests_in_request_id_order`
- `bespoke_event_handling::tests::canonical_dynamic_tool_start_emits_item_and_requests_client`
- `suite::v2::analytics::{remote_unified_background_plugin_script_emits_measurements_after_turn_completion,unified_background_plugin_script_emits_measurements_after_turn_completion,classic_plugin_script_emits_measurement_analytics,remote_unified_plugin_script_emits_measurement_analytics,unified_plugin_script_emits_measurement_analytics}`
- `suite::v2::command_exec::{command_exec_accepts_permission_profile,command_exec_env_overrides_merge_with_server_environment_and_support_unset,command_exec_non_streaming_respects_output_cap,command_exec_permission_profile_does_not_reuse_default_network_proxy,command_exec_permission_profile_project_roots_use_command_cwd,command_exec_permission_profile_starts_selected_network_proxy,command_exec_pipe_streams_output_and_accepts_write,command_exec_process_ids_are_connection_scoped_and_disconnect_terminates_process,command_exec_without_process_id_keeps_buffered_compatibility,command_exec_streaming_does_not_buffer_output,command_exec_tty_implies_streaming_and_reports_pty_output,command_exec_tty_supports_initial_size_and_resize}`
- `suite::v2::dynamic_tools::dynamic_tool_call_round_trip_handles_content_items`
- `suite::v2::executor_skills::{restricted_executor_skill_can_read_permitted_reference,restricted_executor_skill_is_listed_only_when_permitted,restricted_executor_skill_rejects_reference_until_permission_approved}`
- `suite::v2::imagegen_extension::standalone_image_edit_uses_attached_model_visible_image`
- `suite::v2::hooks_list::automatic_marketplace_upgrade_refreshes_hook_runtime_for_loaded_session`
- `suite::v2::model_list::{list_models_pagination_works,list_models_returns_all_models_with_large_limit}`
- `suite::v2::turn_interrupt::turn_interrupt_aborts_running_turn`
- `suite::v2::turn_start::{turn_start_exec_approval_toggle_v2,turn_start_emits_thread_scoped_warning_notification_for_trimmed_skills,turn_start_file_change_approval_accept_for_session_persists_v2,turn_start_streams_apply_patch_change_updates_v2}`

### Strong environment signal

Many failures include:

```text
bwrap: No permissions to create new namespace, likely because the kernel does not allow non-privileged user namespaces
```

The same run also reported:

```text
WARNING: proceeding, even though we could not create PATH aliases: Read-only file system (os error 30)
```

The executor-skills, imagegen, command-exec, turn-interrupt, and apply-patch
failures directly show the bubblewrap error or a deadline after sandbox startup
failed. The agent should first repeat the affected tests in an environment where
unprivileged user namespaces/bubblewrap are available, or establish the
repository's supported way to run these tests without that restriction.

The model-list and outgoing-message failures may be independent; isolate them
with their individual test filters after the sandbox-dependent failures are
removed from the run. The analytics and marketplace failures may also be
affected by the same sandbox/plugin startup failure or by test parallelism.

## CLI suite

Command:

```text
RUSTC_WRAPPER= CARGO_NET_OFFLINE=true just test -p codex-cli
```

Result: 374 passed, 5 failed. The failures were:

- `queue::queue_rejects_local_daemon_that_does_not_support_queueing`
- `queue::queue_rejects_overrides_that_bypass_local_daemon`
- `sandbox_network_proxy::sandbox_with_network_proxy_allows_explicit_loopback_access`
- `sandbox_network_proxy::sandbox_with_network_proxy_blocks_direct_loopback_access`
- `sandbox_cloud_config::sandbox_fetches_and_enforces_cloud_managed_permission_profile`

The two network-proxy tests and the cloud-config test fail because the child
process exits with the same bubblewrap unprivileged-user-namespace error. The
queue tests need separate investigation; inspect their complete log sections and
rerun them individually before treating them as product failures.

## Transport suite note

The complete app-server-transport run ended successfully with 154 tests passed;
one remote-control test needed a retry and was reported as flaky:

`transport::remote_control::tests::remote_control_waits_for_account_id_before_enrolling`

It passed on the second attempt. Do not conflate this with the 33 app-server
failures, but rerun it if investigating test stability.

## Scope boundary

The session-sharing implementation itself had these successful checks:

- TUI suite: 3,705 passed, 6 skipped.
- Owner-record focused test: passed.
- Full app-server-transport suite: passed, with the single retry noted above.

Start by proving which failures disappear in a namespace-capable test
environment. Only then change test or product code, and preserve the distinction
between an environment limitation, a flaky test, and a real regression.
