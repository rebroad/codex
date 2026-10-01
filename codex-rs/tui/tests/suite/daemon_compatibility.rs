//! Verifies feature mismatches do not divert TUI startup from an existing daemon.

use super::focus_palette::PtyCodex;
use super::focus_palette::write_test_config;
use anyhow::Result;
use codex_app_server_protocol::JSONRPCMessage;
use futures::SinkExt;
use futures::StreamExt;
use serde_json::json;
use tokio::net::UnixListener;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incompatible_daemon_remains_attached_for_feature_mismatches() -> Result<()> {
    for scenario in ["default", "explicit", "host policy"] {
        let cwd = codex_utils_cargo_bin::repo_root()?;
        let home = tempfile::tempdir()?;
        write_test_config(home.path(), &cwd)?;
        if scenario == "host policy" {
            let path = home.path().join("config.toml");
            let contents = std::fs::read_to_string(&path)?;
            std::fs::write(
                path,
                format!(
                    "features.code_mode_host = {{enabled=false, disable_in_process_fallback=true}}\n{contents}"
                ),
            )?;
        }
        let socket = codex_app_server_client::app_server_control_socket_path(home.path())?;
        std::fs::create_dir_all(socket.parent().unwrap())?;
        let listener = UnixListener::bind(socket.as_path())?;
        let server_cwd = cwd.clone();
        let server = tokio::spawn(async move {
            let mut socket = loop {
                let (stream, _) = listener.accept().await?;
                if let Ok(socket) = tokio_tungstenite::accept_async(stream).await {
                    break socket;
                }
            };
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let JSONRPCMessage::Request(request) = serde_json::from_str(&text)? else {
                    continue;
                };
                let result = match request.method.as_str() {
                    "initialize" => json!({"userAgent": "daemon-test/0.0.0"}),
                    "account/read" => json!({
                        "account": {"type": "apiKey"},
                        "requiresOpenaiAuth": false,
                        "workspaceRouting": null,
                    }),
                    "config/read" => json!({
                        "config": {"projects": {
                            server_cwd.to_string_lossy(): {"trust_level": "trusted"}
                        }},
                        "origins": {},
                        "layers": [],
                    }),
                    "configRequirements/read" => json!({"requirements": null}),
                    "model/list" => json!({"data": [], "nextCursor": null}),
                    "hooks/list"
                    | "collaborationMode/list"
                    | "plugin/list"
                    | "thread/attachment/list"
                    | "thread/loaded/list"
                    | "skills/list" => json!({"data": []}),
                    "thread/list" => json!({"data": [], "nextCursor": null}),
                    "thread/start" => {
                        json!({
                            "thread": {
                                "id": "00000000-0000-0000-0000-000000000001",
                                "sessionId": "00000000-0000-0000-0000-000000000001",
                                "preview": "New test session",
                                "ephemeral": false,
                                "modelProvider": "openai",
                                "createdAt": 1,
                                "updatedAt": 2,
                                "status": {"type": "active", "activeFlags": []},
                                "cwd": server_cwd,
                                "cliVersion": "0.0.0",
                                "source": "cli",
                                "turns": [],
                            },
                            "model": "gpt-5.6-terra",
                            "modelProvider": "openai",
                            "cwd": server_cwd,
                            "approvalPolicy": "never",
                            "approvalsReviewer": "user",
                            "sandbox": {"type": "dangerFullAccess"},
                            "reasoningEffort": null,
                        })
                    }
                    "experimentalFeature/list" => json!({"data": [{
                        "name": "api_key_model_discovery", "stage": "stable",
                        "displayName": null, "description": null, "announcement": null,
                        "enabled": scenario == "explicit", "defaultEnabled": true,
                    }], "nextCursor": null}),
                    method => anyhow::bail!("unexpected app-server request: {method}"),
                };
                let response = json!({"id": request.id, "result": result});
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let args = if scenario == "explicit" {
            vec!["-c", "features.api_key_model_discovery=false"]
        } else {
            vec![]
        };
        let mut terminal = PtyCodex::start(&cwd, home, &args)?;
        terminal.wait_for_startup()?;
        if let Err(error) = terminal.wait_for_screen("warning") {
            return Err(anyhow::anyhow!(
                "{error}; screen:\n{}",
                terminal.screen_contents()
            ));
        }
        terminal.write_input(b"\x14")?;
        let (snapshot, warning_marker) = match scenario {
            "default" => (
                "daemon_feature_mismatch",
                "existing background server has different feature settings",
            ),
            "explicit" => (
                "daemon_override_mismatch",
                "existing background server has different feature settings",
            ),
            "host policy" => (
                "daemon_host_policy_mismatch",
                "existing background server has different feature settings",
            ),
            _ => unreachable!(),
        };
        terminal.wait_for_screen(warning_marker)?;
        let screen = terminal.screen_contents();
        let warning = screen
            .lines()
            .find(|line| line.contains(warning_marker))
            .ok_or_else(|| anyhow::anyhow!("missing existing-server warning: {screen}"))?;
        insta::assert_snapshot!(snapshot, warning);
        assert!(terminal.screen_contains("Ask Codex to do anything"));
        drop(terminal);
        tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 5), server).await???;
    }
    Ok(())
}
