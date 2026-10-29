//! Best-effort loaded-thread snapshots during managed app-server shutdown.

use super::connection_handling_websocket::DEFAULT_READ_TIMEOUT;
use super::connection_handling_websocket::create_config_toml;
use anyhow::Context;
use anyhow::Result;
use app_test_support::DISABLE_PLUGIN_STARTUP_TASKS_ARG;
use app_test_support::create_final_assistant_message_sse_response;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalResponse;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_transport::daemon_recovery;
use codex_app_server_transport::daemon_recovery_file_path;
use codex_uds::UnixStream;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::json;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command as StdCommand;
use std::process::Stdio;
use tempfile::TempDir;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::Duration;
use tokio::time::sleep;
use tokio::time::timeout;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn managed_restart_resumes_loaded_threads_and_goal_without_client() -> Result<()> {
    let home = TempDir::new()?;
    let (release_response, response_gate) = oneshot::channel();
    let (release_recovered, recovered_gate) = oneshot::channel();
    let (mock, _completions) = start_streaming_sse_server(vec![
        vec![stream_chunk(Some(response_gate), "Done")?],
        vec![stream_chunk(Some(recovered_gate), "Recovered")?],
    ])
    .await;
    create_config_toml(home.path(), mock.uri(), "never")?;
    let socket_path = home.path().join("control/server.sock");
    let recovery_file = daemon_recovery_file_path(home.path());
    std::fs::create_dir_all(recovery_file.parent().context("snapshot parent")?)?;
    std::fs::write(&recovery_file, "invalid JSON")?;
    let mut server = spawn_server(home.path(), &socket_path)?;
    let mut client = connect_daemon_client(
        &socket_path,
        InitializeCapabilities {
            experimental_api: true,
            extensions: Some(std::collections::HashMap::from([(
                "io.modelcontextprotocol/ui".into(),
                json!({"mimeTypes":["text/html"]}),
            )])),
            ..Default::default()
        },
    )
    .await?;
    assert!(!recovery_file.exists());
    let thread = start_thread(
        &mut client,
        /*id*/ 2,
        json!({
            "model":"gpt-5.2",
            "developerInstructions":"Session-only instructions",
            "dynamicTools":[{"name":"lookup_ticket","description":"Look up a ticket",
                             "inputSchema":{"type":"object","properties":{}}}]
        }),
    )
    .await?;
    for (id, method, params) in [
        (
            3,
            "skills/extraRoots/set",
            json!({"extraRoots":[home.path()]}),
        ),
        (
            4,
            "experimentalFeature/enablement/set",
            json!({"enablement":{"mcp_2026_07_28":true}}),
        ),
    ] {
        request(&mut client, id, method, params).await?;
    }
    let idle = start_thread(&mut client, /*id*/ 7, json!({})).await?;
    start_turn(&mut client, /*id*/ 5, &thread.thread.id).await?;
    wait_for_requests(&mock, /*count*/ 1).await?;
    request(
        &mut client,
        /*id*/ 6,
        "thread/goal/set",
        json!({
            "threadId":thread.thread.id,"objective":"continue after restart"
        }),
    )
    .await?;
    request_shutdown(&server, &socket_path).await?;
    release_response.send(()).expect("response is waiting");
    wait_success(&mut server).await?;
    assert_eq!(
        daemon_recovery::read_candidates(&recovery_file)?,
        [thread.thread.id.clone(), idle.thread.id.clone()]
            .into_iter()
            .collect()
    );
    let mut saved = daemon_recovery::read_candidates(&recovery_file)?;
    saved.insert("!missing".into());
    daemon_recovery::write_candidates(&recovery_file, &saved)?;
    let mut successor = spawn_server(home.path(), &socket_path)?;
    // No client reconnects until the goal has made an inference request.
    wait_for_requests(&mock, /*count*/ 2).await?;
    let requests = mock.requests().await;
    let recovered: serde_json::Value = serde_json::from_slice(&requests[1])?;
    assert_eq!(recovered["model"], "gpt-5.2");
    assert!(
        recovered["input"]
            .as_array()
            .context("recovered conversation")?
            .iter()
            .any(|item| item["role"] == "user" && item["content"].to_string().contains("Hello"))
    );
    assert!(
        recovered["tools"].to_string().contains("lookup_ticket"),
        "{recovered}"
    );
    assert!(!recovery_file.exists());
    let mut reconnected = connect_daemon_client(
        &socket_path,
        InitializeCapabilities {
            experimental_api: true,
            extensions: Some(std::collections::HashMap::from([(
                "io.modelcontextprotocol/ui".into(),
                json!({"mimeTypes":["text/html"]}),
            )])),
            ..Default::default()
        },
    )
    .await?;
    let resumed = request(
        &mut reconnected,
        /*id*/ 2,
        "thread/resume",
        json!({
            "threadId":thread.thread.id,"excludeTurns":true
        }),
    )
    .await?;
    assert_eq!(resumed["thread"]["id"], thread.thread.id);
    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let loaded = request(
                &mut reconnected,
                /*id*/ 4,
                "thread/loaded/list",
                json!({}),
            )
            .await?;
            if loaded["data"]
                .as_array()
                .context("loaded IDs")?
                .contains(&json!(idle.thread.id))
            {
                return Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    request(
        &mut reconnected,
        /*id*/ 3,
        "thread/goal/clear",
        json!({"threadId":thread.thread.id}),
    )
    .await?;
    release_recovered
        .send(())
        .expect("recovered goal is waiting");
    request_shutdown(&successor, &socket_path).await?;
    wait_success(&mut successor).await?;
    Ok(())
}

#[tokio::test]
async fn managed_force_shutdown_exits_with_active_work() -> Result<()> {
    let home = TempDir::new()?;
    let (_release_response, response_gate) = oneshot::channel();
    let (mock, _completions) =
        start_streaming_sse_server(vec![vec![stream_chunk(Some(response_gate), "Done")?]]).await;
    create_config_toml(home.path(), mock.uri(), "never")?;
    let socket_path = home.path().join("control/server.sock");
    let mut server = spawn_server(home.path(), &socket_path)?;
    let mut client = connect_default_daemon_client(&socket_path).await?;
    let thread = start_thread(&mut client, /*id*/ 2, json!({})).await?;
    start_turn(&mut client, /*id*/ 3, &thread.thread.id).await?;
    wait_for_requests(&mock, /*count*/ 1).await?;
    request_shutdown(&server, &socket_path).await?;
    assert_still_running(&mut server, "graceful shutdown must wait").await;
    request_shutdown(&server, &socket_path).await?;
    wait_success(&mut server).await?;
    Ok(())
}

enum SnapshotScenario {
    WriteFailure,
    Ephemeral,
    Archived,
    Child,
    Parented,
}

#[test_case::test_case(SnapshotScenario::WriteFailure)]
#[test_case::test_case(SnapshotScenario::Ephemeral)]
#[test_case::test_case(SnapshotScenario::Archived)]
#[test_case::test_case(SnapshotScenario::Child)]
#[test_case::test_case(SnapshotScenario::Parented)]
#[tokio::test]
async fn managed_shutdown_skips_nonpersistent_threads_and_tolerates_save_failure(
    scenario: SnapshotScenario,
) -> Result<()> {
    let home = TempDir::new()?;
    create_config_toml(home.path(), "http://127.0.0.1:1", "never")?;
    let source = if matches!(scenario, SnapshotScenario::Child) {
        codex_protocol::protocol::SessionSource::SubAgent(
            codex_protocol::protocol::SubAgentSource::ThreadSpawn {
                parent_thread_id: codex_protocol::ThreadId::new(),
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            },
        )
    } else {
        codex_protocol::protocol::SessionSource::Cli
    };
    let stored = if matches!(scenario, SnapshotScenario::Parented) {
        app_test_support::create_fake_parented_rollout_with_source(
            home.path(),
            "2026-09-01T12-00-00",
            "2026-09-01T12:00:00Z",
            "Saved task",
            Some("mock_provider"),
            /*git_info*/ None,
            source,
            codex_protocol::SessionId::new(),
            codex_protocol::ThreadId::new(),
        )?
    } else {
        app_test_support::create_fake_rollout_with_source(
            home.path(),
            "2026-09-01T12-00-00",
            "2026-09-01T12:00:00Z",
            "Saved task",
            Some("mock_provider"),
            /*git_info*/ None,
            source,
        )?
    };
    let socket_path = home.path().join("control/server.sock");
    let mut server = spawn_server(home.path(), &socket_path)?;
    let mut client = connect_default_daemon_client(&socket_path).await?;
    if matches!(scenario, SnapshotScenario::Ephemeral) {
        request(
            &mut client,
            /*id*/ 2,
            "thread/start",
            json!({"ephemeral":true}),
        )
        .await?;
    } else {
        request(
            &mut client,
            /*id*/ 2,
            "thread/resume",
            json!({"threadId":stored}),
        )
        .await?;
    }
    let path = daemon_recovery_file_path(home.path());
    if matches!(scenario, SnapshotScenario::WriteFailure) {
        std::fs::create_dir_all(&path)?;
    } else if matches!(scenario, SnapshotScenario::Archived) {
        request(
            &mut client,
            /*id*/ 3,
            "thread/archive",
            json!({"threadId":stored}),
        )
        .await?;
    }
    request_shutdown(&server, &socket_path).await?;
    wait_success(&mut server).await?;
    if matches!(scenario, SnapshotScenario::WriteFailure) {
        assert!(path.is_dir());
    } else {
        assert_eq!(daemon_recovery::read_candidates(&path)?, Default::default());
    }
    Ok(())
}

#[tokio::test]
async fn managed_shutdown_preserves_admitted_resume() -> Result<()> {
    let home = TempDir::new()?;
    let mock = wiremock::MockServer::start().await;
    let (mcp, control) =
        core_test_support::apps_test_server::AppsTestServer::mount_with_startup_control(&mock)
            .await?;
    let release = control.hold_next_successful_initialize();
    create_config_toml(home.path(), "http://127.0.0.1:1", "never")?;
    let config = home.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "{}\n[mcp_servers.stalled]\nurl = {:?}\nrequired = true\nstartup_timeout_sec = 120\nhttp_headers = {{ Authorization = \"Bearer synthetic-test-token\" }}\n",
            std::fs::read_to_string(&config)?,
            format!("{}/api/codex/ps/mcp", mcp.chatgpt_base_url),
        ),
    )?;
    let id = app_test_support::create_fake_rollout(
        home.path(),
        "2026-09-01T12-00-00",
        "2026-09-01T12:00:00Z",
        "Saved task",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let socket = home.path().join("control/server.sock");
    let mut server = spawn_server(home.path(), &socket)?;
    let mut client = connect_default_daemon_client(&socket).await?;
    client
        .send(Message::Text(
            json!({"id":2,"method":"thread/resume","params":{"threadId":id}})
                .to_string()
                .into(),
        ))
        .await?;
    // Required MCP startup holds the admitted resume before its runtime is registered.
    timeout(DEFAULT_READ_TIMEOUT, async {
        while control.initialize_attempts() == 0 {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("required MCP initialization did not begin")?;
    request_shutdown(&server, &socket).await?;
    assert_still_running(&mut server, "shutdown must wait for the admitted resume").await;
    assert!(!daemon_recovery_file_path(home.path()).exists());
    release
        .send(())
        .expect("release required MCP initialization");
    wait_success(&mut server).await?;
    assert_eq!(
        daemon_recovery::read_candidates(&daemon_recovery_file_path(home.path()))?,
        [id].into(),
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn managed_handoff_checkpoints_active_parent_and_child_turns() -> Result<()> {
    use core_test_support::responses;

    let home = TempDir::new()?;
    let (release_spawn, spawn_gate) = oneshot::channel();
    let (release_parent, parent_gate) = oneshot::channel();
    let (release_child, child_gate) = oneshot::channel();
    let (mock, _completions) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: Some(spawn_gate),
            body: responses::sse(vec![
                responses::ev_response_created("spawn-child"),
                responses::ev_function_call_with_namespace(
                    "spawn-child",
                    "collaboration",
                    "spawn_agent",
                    r#"{"task_name":"child","message":"Wait here","fork_turns":"none"}"#,
                ),
                responses::ev_completed("spawn-child"),
            ]),
        }],
        vec![stream_chunk(Some(parent_gate), "Done")?],
        vec![stream_chunk(Some(child_gate), "Done")?],
        vec![stream_chunk(None, "Parent resumed")?],
        vec![stream_chunk(None, "Child resumed")?],
    ])
    .await;
    create_config_toml(home.path(), mock.uri(), "never")?;
    let config_path = home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        config_path,
        format!("{config}\n[features.multi_agent_v2]\nenabled = true\n"),
    )?;
    let socket_path = home.path().join("control/server.sock");
    let executor_binary = codex_utils_cargo_bin::cargo_bin("exec-server")?;
    let _executor_cleanup = ManagedExecutorCleanup::new(home.path(), &executor_binary)?;
    let mut server = spawn_server_with_executor(home.path(), &socket_path, &executor_binary)?;
    let mut client = connect_default_daemon_client(&socket_path).await?;
    let parent = start_thread(&mut client, /*id*/ 2, json!({})).await?;
    start_turn(&mut client, /*id*/ 3, &parent.thread.id).await?;
    wait_for_requests(&mock, /*count*/ 1).await?;
    // Establish the descendant before draining so both active turns belong to
    // the tree checkpoint captured by this replacement.
    release_spawn.send(()).expect("parent is waiting to spawn");
    wait_for_requests(&mock, /*count*/ 3).await?;
    request_shutdown(&server, &socket_path).await?;
    wait_success(&mut server).await?;
    release_parent
        .send(())
        .expect("old parent response is gated");
    release_child.send(()).expect("old child response is gated");
    let recovery_file = daemon_recovery_file_path(home.path());
    let snapshot = daemon_recovery::read_snapshot(&recovery_file)?;
    let active_turns: Vec<_> = snapshot.interrupted.values().collect();
    assert_eq!(active_turns.len(), 2);
    assert!(active_turns.iter().all(|turn| turn.suspended.is_some()));
    assert!(active_turns.iter().any(|turn| {
        turn.suspended
            .as_ref()
            .is_some_and(|thread| thread.parent_thread_id.is_some())
    }));

    let mut successor = spawn_server(home.path(), &socket_path)?;
    timeout(
        Duration::from_secs(30),
        mock.wait_for_request_count(/*count*/ 5),
    )
    .await
    .context("successor did not resume both active tree turns")?;
    assert_eq!(mock.requests().await.len(), 5);
    request_shutdown(&successor, &socket_path).await?;
    wait_success(&mut successor).await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn managed_handoff_reissues_pending_user_interaction_after_replacement() -> Result<()> {
    use core_test_support::responses;

    let home = TempDir::new()?;
    let input_args = serde_json::to_string(&json!({
        "questions": [{
            "id": "confirm_path",
            "header": "Confirm",
            "question": "Proceed with the plan?",
            "options": [{
                "label": "Yes",
                "description": "Continue the current plan."
            }, {
                "label": "No",
                "description": "Stop and revisit the plan."
            }]
        }]
    }))?;
    let ask_for_input = responses::sse(vec![
        responses::ev_response_created("handoff-input-response"),
        responses::ev_function_call("handoff-input", "request_user_input", &input_args),
        responses::ev_completed("handoff-input-response"),
    ]);
    let repeat_ask_for_input = responses::sse(vec![
        responses::ev_response_created("handoff-input-recovered-response"),
        responses::ev_function_call("handoff-input", "request_user_input", &input_args),
        responses::ev_completed("handoff-input-recovered-response"),
    ]);
    let finish_turn = responses::sse(vec![
        responses::ev_response_created("handoff-input-finished-response"),
        responses::ev_assistant_message("handoff-input-finished-message", "Completed"),
        responses::ev_completed("handoff-input-finished-response"),
    ]);
    let (mock, _completions) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: ask_for_input,
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: repeat_ask_for_input,
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: finish_turn,
        }],
    ])
    .await;
    create_config_toml(home.path(), mock.uri(), "on-request")?;
    let socket_path = home.path().join("control/server.sock");
    let executor_binary = codex_utils_cargo_bin::cargo_bin("exec-server")?;
    let _executor_cleanup = ManagedExecutorCleanup::new(home.path(), &executor_binary)?;
    let mut server = spawn_server_with_executor(home.path(), &socket_path, &executor_binary)?;
    let mut client = connect_daemon_client(
        &socket_path,
        InitializeCapabilities {
            experimental_api: true,
            ..Default::default()
        },
    )
    .await?;
    let thread = start_thread(&mut client, /*id*/ 2, json!({})).await?;
    request(
        &mut client,
        /*id*/ 3,
        "turn/start",
        json!({
            "threadId":thread.thread.id,
            "input":[{"type":"text","text":"Plan the next step"}],
            "collaborationMode":{"mode":"plan","settings":{"model":"mock-model","reasoning_effort":null,"developer_instructions":null}}
        }),
    )
    .await?;
    wait_for_requests(&mock, /*count*/ 1)
        .await
        .context("initial user-input request did not reach the mock server")?;

    let original_request = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let frame = client.next().await.context("old client socket closed")??;
            let Message::Text(text) = frame else {
                continue;
            };
            let JSONRPCMessage::Request(request) = serde_json::from_str(&text)? else {
                continue;
            };
            if request.method == "item/tool/requestUserInput" {
                return Ok::<_, anyhow::Error>(request);
            }
        }
    })
    .await
    .context("initial user-input interaction was not issued")??;
    let original_params = original_request
        .params
        .as_ref()
        .context("initial user-input request params")?;
    assert_eq!(original_params["threadId"], thread.thread.id);
    assert_eq!(original_params["itemId"], "handoff-input");

    request_shutdown(&server, &socket_path).await?;
    wait_success(&mut server)
        .await
        .context("pre-replacement server did not exit after saving the checkpoint")?;
    let recovery_file = daemon_recovery_file_path(home.path());
    let snapshot = daemon_recovery::read_snapshot(&recovery_file)?;
    let suspended = snapshot
        .interrupted
        .get(&thread.thread.id)
        .and_then(|turn| turn.suspended.as_ref())
        .context("active turn checkpoint was not saved")?;
    assert_eq!(suspended.pending_interactions.len(), 1);
    assert_eq!(
        suspended.pending_interactions[0].correlation_id,
        "user_input:".to_string() + &suspended.turn_id
    );

    let mut successor = spawn_server(home.path(), &socket_path)?;
    let mut replacement_client = connect_daemon_client(
        &socket_path,
        InitializeCapabilities {
            experimental_api: true,
            ..Default::default()
        },
    )
    .await?;
    replacement_client
        .send(Message::Text(
            json!({
                "id": 2,
                "method": "thread/resume",
                "params": {"threadId":thread.thread.id,"excludeTurns":true}
            })
            .to_string()
            .into(),
        ))
        .await?;
    let mut resume_response_received = false;
    let mut recovered_request = None;
    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let frame = replacement_client
                .next()
                .await
                .context("replacement client socket closed")??;
            let Message::Text(text) = frame else {
                continue;
            };
            match serde_json::from_str::<JSONRPCMessage>(&text)? {
                JSONRPCMessage::Response(response) if response.id == RequestId::Integer(2) => {
                    assert_eq!(response.result["thread"]["id"], thread.thread.id);
                    resume_response_received = true;
                }
                JSONRPCMessage::Error(error) if error.id == RequestId::Integer(2) => {
                    anyhow::bail!("thread/resume failed: {}", error.error.message);
                }
                JSONRPCMessage::Request(request)
                    if request.method == "item/tool/requestUserInput" =>
                {
                    recovered_request = Some(request);
                }
                _ => {}
            }
            if resume_response_received && recovered_request.is_some() {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await
    .context("replacement did not resume the thread and reissue its pending interaction")??;
    let recovered_request = recovered_request.context("missing reissued user-input request")?;
    let recovered_params = recovered_request
        .params
        .as_ref()
        .context("reissued user-input request params")?;
    assert_eq!(recovered_params["threadId"], thread.thread.id);
    assert_eq!(recovered_params["turnId"], suspended.turn_id);
    assert_eq!(recovered_params["itemId"], "handoff-input");

    replacement_client
        .send(Message::Text(
            json!({
                "id":recovered_request.id,
                "result":{"answers":{"confirm_path":{"answers":["yes"]}}}
            })
            .to_string()
            .into(),
        ))
        .await?;
    wait_for_requests(&mock, /*count*/ 3)
        .await
        .context("recovered user-input answer did not trigger the next model request")?;
    let requests = mock.requests().await;
    let resumed: serde_json::Value = serde_json::from_slice(&requests[2])?;
    assert!(
        resumed["input"].to_string().contains("yes"),
        "resumed request did not include the accepted answer: {resumed}"
    );

    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let frame = replacement_client
                .next()
                .await
                .context("replacement client closed before turn completion")??;
            let Message::Text(text) = frame else {
                continue;
            };
            if let JSONRPCMessage::Notification(notification) = serde_json::from_str(&text)?
                && notification.method == "turn/completed"
            {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await
    .context("recovered turn did not complete")??;

    request_shutdown(&successor, &socket_path).await?;
    wait_success(&mut successor)
        .await
        .context("replacement server did not exit after turn completion")?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn managed_handoff_reattaches_a_running_local_command() -> Result<()> {
    use core_test_support::responses;

    let home = TempDir::new()?;
    let command =
        "printf 'handoff-pid=%s\\n' \"$$\"; sleep 20; printf 'handoff-pid=%s done\\n' \"$$\"";
    let first_response = responses::sse(vec![
        responses::ev_response_created("handoff-command-response"),
        responses::ev_exec_command_call_with_args(
            "handoff-command",
            &json!({
                "cmd": command,
                "shell": "/bin/sh",
                "login": false,
                "yield_time_ms": 1_000,
            }),
        ),
        responses::ev_completed("handoff-command-response"),
    ]);
    let (release_old_response, old_response_gate) = oneshot::channel();
    let old_response = responses::sse(vec![
        responses::ev_response_created("handoff-old-server-response"),
        responses::ev_assistant_message("handoff-old-server-message", "Old server"),
        responses::ev_completed("handoff-old-server-response"),
    ]);
    let (mock, _completions) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: first_response,
        }],
        vec![StreamingSseChunk {
            gate: Some(old_response_gate),
            body: old_response,
        }],
    ])
    .await;
    create_config_toml(home.path(), mock.uri(), "on-request")?;
    let socket_path = home.path().join("control/server.sock");
    let executor_binary = codex_utils_cargo_bin::cargo_bin("exec-server")?;
    let executor_cleanup = ManagedExecutorCleanup::new(home.path(), &executor_binary)?;
    let mut server = spawn_server_with_executor(home.path(), &socket_path, &executor_binary)?;
    let mut client = connect_daemon_client(
        &socket_path,
        InitializeCapabilities {
            experimental_api: true,
            ..Default::default()
        },
    )
    .await?;
    let thread = start_thread(&mut client, /*id*/ 2, json!({})).await?;
    client
        .send(Message::Text(
            json!({
                "id": 3,
                "method": "turn/start",
                "params": {
                    "threadId": &thread.thread.id,
                    "input": [{"type": "text", "text": "Hello"}]
                }
            })
            .to_string()
            .into(),
        ))
        .await?;
    wait_for_requests(&mock, /*count*/ 1)
        .await
        .context("initial model request did not arrive")?;
    let turn_start = timeout(Duration::from_secs(30), async {
        let mut turn_id = None;
        let mut requests = None;
        loop {
            tokio::select! {
                _ = mock.wait_for_request_count(2), if requests.is_none() => {
                    requests = Some(mock.requests().await);
                }
                message = client.next() => {
                    let message = message.context("daemon websocket closed")??;
                    let Message::Text(text) = message else {
                        continue;
                    };
                    match serde_json::from_str::<JSONRPCMessage>(&text)? {
                        JSONRPCMessage::Response(response) if response.id == RequestId::Integer(3) => {
                            let response: TurnStartResponse = serde_json::from_value(response.result)?;
                            turn_id = Some(response.turn.id);
                        }
                        JSONRPCMessage::Notification(notification)
                            if notification.method == "turn/completed" =>
                        {
                            anyhow::bail!("turn completed before command started: {notification:?}");
                        }
                        JSONRPCMessage::Request(request)
                            if request.method == "item/commandExecution/requestApproval" =>
                        {
                            let result = serde_json::to_value(CommandExecutionRequestApprovalResponse {
                                decision: CommandExecutionApprovalDecision::Accept,
                            })?;
                            client
                                .send(Message::Text(
                                    json!({"id": request.id, "result": result})
                                        .to_string()
                                        .into(),
                                ))
                                .await?;
                        }
                        _ => {}
                    }
                }
            }
            if let (Some(turn_id), Some(requests)) = (turn_id.as_ref(), requests.as_ref()) {
                let request: serde_json::Value =
                    serde_json::from_slice(&requests[1]).context("running request")?;
                return Ok::<_, anyhow::Error>((turn_id.clone(), request));
            }
        }
    })
    .await;
    let (turn_id, running_request) =
        turn_start.context("timed out waiting for the running command request")??;

    let accepted_steer = request(
        &mut client,
        /*id*/ 4,
        "turn/steer",
        json!({
            "threadId": thread.thread.id,
            "expectedTurnId": turn_id,
            "clientUserMessageId": "handoff-queued-input",
            "input": [{"type": "text", "text": "Keep the final answer concise"}]
        }),
    )
    .await?;
    assert_eq!(accepted_steer["turnId"], turn_id);

    let running_output = running_request["input"]
        .as_array()
        .context("running command request input")?
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == "handoff-command")
        .context("running command output was not sent to the model")?;
    let running_output_text = running_output["output"].as_str().unwrap_or_default();
    let process_session_id = running_output_text
        .split_once("Process running with session ID ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .context("command output did not expose its running session ID")?
        .parse::<u32>()?;
    request_shutdown(&server, &socket_path).await?;
    wait_success(&mut server)
        .await
        .context("pre-replacement app server did not finish draining")?;
    let recovery_file = daemon_recovery_file_path(home.path());
    let snapshot = daemon_recovery::read_snapshot(&recovery_file)?;
    let suspended = snapshot
        .interrupted
        .get(&thread.thread.id)
        .and_then(|turn| turn.suspended.as_ref())
        .context("active turn did not save its executor process checkpoint")?;
    assert_eq!(suspended.executor_processes.len(), 1);
    assert_eq!(
        suspended.executor_processes[0].unified_process_id,
        process_session_id as i32
    );
    let _ = release_old_response.send(());
    let poll_response = responses::sse(vec![
        responses::ev_response_created("handoff-process-poll-response"),
        responses::ev_function_call(
            "handoff-command-poll",
            "write_stdin",
            &json!({"session_id": process_session_id, "yield_time_ms": 30_000}).to_string(),
        ),
        responses::ev_completed("handoff-process-poll-response"),
    ]);
    let poll_completion = mock
        .queue_response(vec![StreamingSseChunk {
            gate: None,
            body: poll_response,
        }])
        .await;
    let final_response = responses::sse(vec![
        responses::ev_response_created("handoff-command-recovered-response"),
        responses::ev_assistant_message("handoff-command-recovered-message", "Command completed"),
        responses::ev_completed("handoff-command-recovered-response"),
    ]);
    let final_completion = mock
        .queue_response(vec![StreamingSseChunk {
            gate: None,
            body: final_response,
        }])
        .await;

    let mut successor = spawn_server(home.path(), &socket_path)?;
    wait_for_requests(&mock, /*count*/ 3)
        .await
        .context("successor did not resume the active turn")?;
    let requests = mock.requests().await;
    let recovered: serde_json::Value = serde_json::from_slice(&requests[2])?;
    assert!(
        recovered["input"]
            .to_string()
            .contains("Keep the final answer concise"),
        "recovered request omitted the accepted steer"
    );

    timeout(Duration::from_secs(30), mock.wait_for_request_count(4))
        .await
        .context("successor did not resume the recovered process")?;
    let requests = mock.requests().await;
    let completion_request: serde_json::Value = serde_json::from_slice(&requests[3])?;
    let command_output = completion_request["input"]
        .as_array()
        .context("completed command request input")?
        .iter()
        .find(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "handoff-command-poll"
        })
        .context("recovered write_stdin output")?;
    assert!(
        command_output["output"]
            .as_str()
            .unwrap_or_default()
            .contains("handoff-pid="),
        "recovered command output did not contain the process marker: {command_output}"
    );
    assert!(
        command_output["output"]
            .as_str()
            .unwrap_or_default()
            .contains("done"),
        "recovered command output did not contain its final marker: {command_output}"
    );
    request_shutdown(&successor, &socket_path).await?;
    wait_success(&mut successor).await?;
    poll_completion
        .await
        .context("write_stdin response did not finish")?;
    final_completion
        .await
        .context("final response did not finish")?;
    mock.shutdown().await;
    drop(executor_cleanup);
    Ok(())
}

async fn wait_success(server: &mut Child) -> Result<()> {
    anyhow::ensure!(
        timeout(Duration::from_secs(10), server.wait())
            .await??
            .success(),
        "server did not exit successfully"
    );
    Ok(())
}

async fn assert_still_running(server: &mut Child, message: &str) {
    assert!(
        timeout(Duration::from_millis(300), server.wait())
            .await
            .is_err(),
        "{message}"
    );
}

#[test_case::test_case("running")]
#[test_case::test_case("plan")]
#[test_case::test_case("read_only")]
#[test_case::test_case("stricter_config")]
#[test_case::test_case("different_environment")]
#[test_case::test_case("legacy_snapshot")]
#[test_case::test_case("completed")]
#[test_case::test_case("canceled")]
#[test_case::test_case("compacting")]
#[tokio::test]
async fn managed_shutdown_records_interrupted_turn(outcome: &str) -> Result<()> {
    let home = TempDir::new()?;
    let (release, gate) = oneshot::channel();
    let (release_compaction, compaction_gate) = oneshot::channel();
    let (mock, _completions) = start_streaming_sse_server(vec![
        vec![stream_chunk(Some(gate), "Original")?],
        vec![stream_chunk(Some(compaction_gate), "Compacted")?],
    ])
    .await;
    create_config_toml(home.path(), mock.uri(), "never")?;
    let socket = home.path().join("control/server.sock");
    let mut server = spawn_server(home.path(), &socket)?;
    let mut client = connect_daemon_client(
        &socket,
        InitializeCapabilities {
            experimental_api: true,
            ..Default::default()
        },
    )
    .await?;
    let thread = start_thread(&mut client, /*id*/ 2, json!({})).await?;
    let local_environment = thread
        .thread
        .environments
        .as_ref()
        .and_then(|environments| environments.first())
        .cloned();
    let id = thread.thread.id;
    let mode = if outcome == "plan" { "plan" } else { "default" };
    let schema = json!({"type":"object","properties":{},"additionalProperties":false});
    let turn = request(
        &mut client,
        /*id*/ 3,
        "turn/start",
        json!({
            "threadId":id,"input":[{"type":"text","text":"Do the work"}],"outputSchema":schema,
            "sandboxPolicy": match outcome {
                "read_only" => json!({"type":"readOnly"}),
                "stricter_config" => json!({"type":"workspaceWrite","writableRoots":[],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}),
                _ => serde_json::Value::Null,
            },
            "collaborationMode":{"mode":mode,"settings":{"model":"mock-model","reasoning_effort":null,"developer_instructions":null}}
        }),
    )
    .await?;
    wait_for_requests(&mock, /*count*/ 1).await?;
    match outcome {
        "completed" => {
            release.send(()).expect("original request is waiting");
            timeout(DEFAULT_READ_TIMEOUT, async {
                loop {
                    let Message::Text(text) = client.next().await.context("socket closed")?? else {
                        continue;
                    };
                    if let JSONRPCMessage::Notification(notification) = serde_json::from_str(&text)?
                        && notification.method == "turn/completed"
                    {
                        break;
                    }
                }
                Ok::<(), anyhow::Error>(())
            })
            .await??;
        }
        "canceled" => {
            request(
                &mut client,
                /*id*/ 4,
                "turn/interrupt",
                json!({"threadId":id,"turnId":turn["turn"]["id"]}),
            )
            .await?;
        }
        "compacting" => {
            request(
                &mut client,
                /*id*/ 4,
                "thread/compact/start",
                json!({"threadId":id}),
            )
            .await?;
            wait_for_requests(&mock, /*count*/ 2).await?;
        }
        "running"
        | "plan"
        | "read_only"
        | "stricter_config"
        | "different_environment"
        | "legacy_snapshot" => {}
        _ => unreachable!(),
    }
    request_shutdown(&server, &socket).await?;
    let path = daemon_recovery_file_path(home.path());
    timeout(DEFAULT_READ_TIMEOUT, async {
        while !path.exists() {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    let interrupted = !matches!(outcome, "completed" | "canceled" | "compacting");
    let suspended = interrupted.then(|| codex_protocol::turn_input::SuspendedThreadTurn {
        thread_id: codex_protocol::ThreadId::from_string(&id).expect("thread ID"),
        parent_thread_id: None,
        turn_id: turn["turn"]["id"].as_str().expect("turn ID").to_string(),
        pending_input: Vec::new(),
        pending_interactions: Vec::new(),
        executor_processes: Vec::new(),
    });
    let expected: std::collections::BTreeMap<String, daemon_recovery::InterruptedTurn> =
        if interrupted {
            [(
                id.clone(),
                daemon_recovery::InterruptedTurn {
                    turn_id: turn["turn"]["id"].as_str().context("turn ID")?.to_string(),
                    output_schema: Some(schema.clone()),
                    service_tier: Some("default".into()),
                    cyber_access_program: None,
                    local_environment,
                    suspended: suspended.clone(),
                },
            )]
            .into()
        } else {
            Default::default()
        };
    let snapshot = daemon_recovery::read_snapshot(&path)?;
    assert_eq!(snapshot.loaded, [id.clone()].into());
    assert_eq!(
        serde_json::to_value(snapshot.interrupted)?,
        serde_json::to_value(expected)?
    );
    // Existing binaries can still read the candidate array and ignore the metadata entry.
    let legacy: std::collections::BTreeSet<String> =
        serde_json::from_slice(&std::fs::read(&path)?)?;
    assert!(legacy.contains(&id));
    if server.try_wait()?.is_none() {
        server.kill().await?;
    }
    if !interrupted {
        return Ok(());
    }
    if outcome == "running" {
        // A prior recovery can append an older turn's abort after this turn started.
        codex_rollout::append_rollout_item_to_path(
            thread.thread.path.as_ref().context("rollout path")?,
            &serde_json::from_value(json!({
                "type":"event_msg",
                "payload":{"type":"turn_aborted","turn_id":"older-turn","reason":"interrupted"}
            }))?,
        )
        .await?;
    }
    if outcome == "read_only" {
        let config = home.path().join("config.toml");
        let contents = std::fs::read_to_string(&config)?;
        std::fs::write(
            config,
            contents.replace(
                "sandbox_mode = \"read-only\"",
                "default_permissions = \":workspace\"",
            ),
        )?;
    }
    if matches!(outcome, "different_environment" | "legacy_snapshot") {
        let mut snapshot = daemon_recovery::read_snapshot(&path)?;
        let saved = snapshot
            .interrupted
            .get_mut(&id)
            .context("interrupted turn")?;
        if outcome == "legacy_snapshot" {
            saved.local_environment = None;
        } else {
            saved
                .local_environment
                .as_mut()
                .context("local environment")?
                .environment_id = "remote".into();
        }
        daemon_recovery::write_snapshot(&path, &snapshot)?;
    }
    let mut successor = spawn_server(home.path(), &socket)?;
    if !matches!(outcome, "running" | "plan" | "read_only") {
        let mut client = connect_default_daemon_client(&socket).await?;
        timeout(DEFAULT_READ_TIMEOUT, async {
            loop {
                let loaded =
                    request(&mut client, /*id*/ 2, "thread/loaded/list", json!({})).await?;
                if loaded["data"]
                    .as_array()
                    .context("loaded IDs")?
                    .contains(&json!(id))
                {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        // Resume serializes with the ongoing daemon restore, so the recovery attempt has finished.
        request(
            &mut client,
            /*id*/ 3,
            "thread/resume",
            json!({"threadId":id,"excludeTurns":true}),
        )
        .await?;
        let response = request(
            &mut client,
            /*id*/ 2,
            "thread/read",
            json!({"threadId":id,"includeTurns":true}),
        )
        .await?;
        assert_eq!(response["thread"]["status"]["type"], "idle");
        assert_eq!(mock.requests().await.len(), 1);
        successor.kill().await?;
        return Ok(());
    }
    // Recovery starts without a client, then its internal message survives reconnect.
    wait_for_requests(&mock, /*count*/ 2).await?;
    let mut client = connect_default_daemon_client(&socket).await?;
    request(
        &mut client,
        /*id*/ 2,
        "thread/resume",
        json!({"threadId":id,"excludeTurns":true}),
    )
    .await?;
    let response = request(
        &mut client,
        /*id*/ 3,
        "thread/read",
        json!({"threadId":id,"includeTurns":true}),
    )
    .await?;
    let statuses: Vec<_> = response["thread"]["turns"]
        .as_array()
        .context("turns")?
        .iter()
        .filter(|turn| turn["id"] != "older-turn")
        .map(|turn| turn["status"].clone())
        .collect();
    assert_eq!(statuses, vec![json!("inProgress")]);
    release_compaction
        .send(())
        .expect("continuation is waiting");
    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let response = request(
                &mut client,
                /*id*/ 3,
                "thread/read",
                json!({"threadId":id,"includeTurns":true}),
            )
            .await?;
            let turns = response["thread"]["turns"].as_array().context("turns")?;
            if let Some(continued) = turns.last()
                && continued["status"] == "completed"
            {
                assert_eq!(continued["id"], turn["turn"]["id"]);
                assert!(
                    continued["items"]
                        .as_array()
                        .context("items")?
                        .iter()
                        .all(|item| item["type"] != "userMessage")
                );
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await??;
    let requests = mock.requests().await;
    assert_eq!(requests.len(), 2);
    let body: serde_json::Value = serde_json::from_slice(&requests[1])?;
    assert_eq!(body["text"]["format"]["schema"], schema);
    let input = body["input"].to_string();
    assert!(
        input.contains("Do the work") && !input.contains("Continue the unfinished work"),
        "{input}"
    );
    let collaboration = input
        .rsplit("<collaboration_mode>")
        .next()
        .context("collaboration instructions")?;
    assert_eq!(
        collaboration.contains("# Plan Mode"),
        mode == "plan",
        "{input}"
    );
    successor.kill().await?;
    Ok(())
}

fn stream_chunk(gate: Option<oneshot::Receiver<()>>, body: &str) -> Result<StreamingSseChunk> {
    Ok(StreamingSseChunk {
        gate,
        body: create_final_assistant_message_sse_response(body)?,
    })
}

async fn wait_for_requests(
    mock: &core_test_support::streaming_sse::StreamingSseServer,
    count: usize,
) -> Result<()> {
    timeout(Duration::from_secs(5), mock.wait_for_request_count(count)).await?;
    Ok(())
}

async fn start_turn(
    websocket: &mut WebSocketStream<UnixStream>,
    id: i64,
    thread_id: impl serde::Serialize,
) -> Result<String> {
    let response: TurnStartResponse = serde_json::from_value(
        request(
            websocket,
            id,
            "turn/start",
            json!({"threadId":thread_id,"input":[{"type":"text","text":"Hello"}]}),
        )
        .await?,
    )?;
    Ok(response.turn.id)
}

async fn start_thread(
    websocket: &mut WebSocketStream<UnixStream>,
    id: i64,
    params: serde_json::Value,
) -> Result<ThreadStartResponse> {
    Ok(serde_json::from_value(
        request(websocket, id, "thread/start", params).await?,
    )?)
}

async fn connect_default_daemon_client(socket_path: &Path) -> Result<WebSocketStream<UnixStream>> {
    connect_daemon_client(socket_path, InitializeCapabilities::default()).await
}

pub(super) async fn connect_daemon_client(
    socket_path: &Path,
    capabilities: InitializeCapabilities,
) -> Result<WebSocketStream<UnixStream>> {
    connect_initialized(socket_path, capabilities, "daemon_recovery_test", "0.1.0").await
}

pub(super) fn spawn_server(home: &Path, socket_path: &Path) -> Result<Child> {
    spawn_server_inner(home, socket_path, None, None)
}

#[cfg(unix)]
struct ManagedExecutorCleanup {
    state_file: PathBuf,
    executable: PathBuf,
}

#[cfg(unix)]
impl ManagedExecutorCleanup {
    fn new(home: &Path, executable: &Path) -> Result<Self> {
        Ok(Self {
            state_file: home.join("app-server-daemon").join("local-executor.json"),
            executable: std::fs::canonicalize(executable)?,
        })
    }
}

#[cfg(unix)]
impl Drop for ManagedExecutorCleanup {
    fn drop(&mut self) {
        let Ok(state) = std::fs::read(&self.state_file) else {
            return;
        };
        let Ok(state) = serde_json::from_slice::<serde_json::Value>(&state) else {
            return;
        };
        let Some(pid) = state["pid"]
            .as_u64()
            .and_then(|pid| i32::try_from(pid).ok())
        else {
            return;
        };
        let Ok(executable) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            return;
        };
        if executable != self.executable {
            return;
        }
        // The executor is intentionally detached from the app server; tests
        // must explicitly stop the fixture so it cannot leak into later runs.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

#[cfg(unix)]
fn spawn_server_with_executor(
    home: &Path,
    socket_path: &Path,
    executor_binary: &Path,
) -> Result<Child> {
    let sandbox_binary = codex_utils_cargo_bin::cargo_bin("codex-linux-sandbox")?;
    spawn_server_inner(
        home,
        socket_path,
        Some(executor_binary),
        Some(sandbox_binary.as_path()),
    )
}

fn spawn_server_inner(
    home: &Path,
    socket_path: &Path,
    executor_binary: Option<&Path>,
    sandbox_binary: Option<&Path>,
) -> Result<Child> {
    let binary = codex_utils_cargo_bin::cargo_bin("codex-app-server")?;
    let mut command = Command::new(binary);
    command
        .args(["--listen", &format!("unix://{}", socket_path.display())])
        .arg(DISABLE_PLUGIN_STARTUP_TASKS_ARG)
        .env("CODEX_HOME", home)
        .env_remove("CODEX_EXEC_SERVER_URL")
        .env_remove("CODEX_EXEC_SERVER_SESSION_ID_FILE")
        // Match TestAppServer's isolation from the host's managed configuration.
        .env(
            "CODEX_APP_SERVER_MANAGED_CONFIG_PATH",
            home.join("managed_config.toml"),
        )
        .current_dir(home)
        .arg("--managed-daemon")
        .env(
            codex_app_server_transport::DAEMON_SHUTDOWN_SOCKET_ENV,
            socket_path,
        );
    if let Some(executor_binary) = executor_binary {
        command.env("CODEX_MANAGED_EXECUTOR_EXECUTABLE", executor_binary);
    }
    if let Some(sandbox_binary) = sandbox_binary {
        command.env("CODEX_TEST_LINUX_SANDBOX_EXE", sandbox_binary);
    }
    Ok(command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            if std::env::var_os("CODEX_APP_SERVER_TEST_STDERR").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            },
        )
        .kill_on_drop(true)
        .spawn()?)
}

#[cfg(unix)]
async fn request_shutdown(server: &Child, _socket_path: &Path) -> Result<()> {
    let pid = server.id().context("server pid")?;
    let status = StdCommand::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()?;
    anyhow::ensure!(status.success(), "failed to signal server {pid}: {status}");
    Ok(())
}

#[cfg(windows)]
async fn request_shutdown(server: &Child, socket_path: &Path) -> Result<()> {
    let pid = server.id().context("server pid")?.to_string();
    timeout(DEFAULT_READ_TIMEOUT, async {
        let stream = UnixStream::connect(socket_path).await?;
        let (mut websocket, _) = client_async("ws://localhost/daemon/shutdown", stream).await?;
        websocket.send(Message::Text(pid.clone().into())).await?;
        let reply = websocket
            .next()
            .await
            .context("missing shutdown acknowledgment")??;
        assert_eq!(reply, Message::Text(pid.into()));
        websocket.close(None).await?;
        Ok(())
    })
    .await?
}

async fn connect_initialized(
    socket_path: &Path,
    capabilities: InitializeCapabilities,
    client_name: &str,
    client_version: &str,
) -> Result<WebSocketStream<UnixStream>> {
    let mut websocket = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            if let Ok(stream) = UnixStream::connect(socket_path).await
                && let Ok((websocket, _)) = client_async("ws://localhost/", stream).await
            {
                return websocket;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("server did not open its control socket")?;
    request(
        &mut websocket,
        /*id*/ 1,
        "initialize",
        json!({"clientInfo":{"name":client_name,"version":client_version},"capabilities":capabilities}),
    )
    .await?;
    websocket
        .send(Message::Text(
            json!({"method":"initialized"}).to_string().into(),
        ))
        .await?;
    Ok(websocket)
}

pub(super) async fn request(
    websocket: &mut WebSocketStream<UnixStream>,
    id: i64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    websocket
        .send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await?;
    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let frame = websocket.next().await.context("socket closed")??;
            let Message::Text(text) = frame else { continue };
            match serde_json::from_str::<JSONRPCMessage>(&text)? {
                JSONRPCMessage::Response(response) if response.id == RequestId::Integer(id) => {
                    return Ok(response.result);
                }
                JSONRPCMessage::Error(error) if error.id == RequestId::Integer(id) => {
                    anyhow::bail!("{}: {}", method, error.error.message);
                }
                _ => {}
            }
        }
    })
    .await
    .context("timed out waiting for app-server response")?
}
