//! Application policy is enforced through public thread and turn RPCs.

use anyhow::Result;
use app_test_support::ChatGptAuthFixture;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::write_chatgpt_auth;
use app_test_support::write_models_cache;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_config::types::AuthCredentialsStoreMode;
use core_test_support::responses;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn standalone_startup_applies_local_policy_before_fetching_stored_account_requirements()
-> Result<()> {
    let backend = MockServer::start().await;
    let bundle_path = "/backend-api/wham/config/bundle";
    Mock::given(method("GET"))
        .and(path(bundle_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&backend)
        .await;
    let home = TempDir::new()?;
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "chatgpt_base_url = '{}/backend-api/'\ncli_auth_credentials_store = 'file'\n",
            backend.uri()
        ),
    )?;
    std::fs::write(
        home.path().join("requirements.toml"),
        "[application.network]",
    )?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("stored-token")
            .account_id("selected")
            .plan_type("enterprise"),
        AuthCredentialsStoreMode::File,
    )?;
    let _app = TestAppServer::builder()
        .with_codex_home(home.path())
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized()
        .await?;
    assert!(
        backend
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != bundle_path)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_limit_fallback_republishes_managed_network_policy_before_retry() -> Result<()> {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer primary-token"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": { "type": "usage_limit_reached", "message": "limit reached" }
        })))
        .expect(1)
        .mount(&backend)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer fallback-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses::sse(vec![
                    responses::ev_response_created("fallback-response"),
                    responses::ev_assistant_message("fallback-message", "continued"),
                    responses::ev_completed("fallback-response"),
                ])),
        )
        .expect(1)
        .mount(&backend)
        .await;

    let home = TempDir::new()?;
    MockResponsesConfig::new(&backend.uri())
        .with_provider_config("requires_openai_auth = true\nsupports_websockets = false")
        .with_root_config(&format!("chatgpt_base_url = \"{}\"", backend.uri()))
        .write(home.path())?;
    std::fs::write(
        home.path().join("requirements.toml"),
        "[application.network]\nenabled = false\n",
    )?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("fallback-token")
            .account_id("fallback-account")
            .chatgpt_user_id("fallback-user"),
        AuthCredentialsStoreMode::File,
    )?;
    std::fs::rename(
        home.path().join("auth.json"),
        home.path().join("auth2.json"),
    )?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("primary-token")
            .account_id("primary-account")
            .chatgpt_user_id("primary-user"),
        AuthCredentialsStoreMode::File,
    )?;

    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized()
        .await?;
    let thread = app.start_thread(ThreadStartParams::default()).await?.thread;
    let completed = app
        .start_turn_and_wait_for_completion(TurnStartParams {
            thread_id: thread.id,
            input: vec![UserInput::Text {
                text: "continue".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    assert_eq!(completed.turn.status, TurnStatus::Completed);
    let authorization_headers = backend
        .received_requests()
        .await
        .expect("mock server should record received requests")
        .into_iter()
        .filter(|request| {
            request.method.as_str() == "POST" && request.url.path() == "/v1/responses"
        })
        .map(|request| {
            request.headers["authorization"]
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        authorization_headers,
        ["Bearer primary-token", "Bearer fallback-token"]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_reloads_apply_local_edits_and_cancel_active_responses() -> Result<()> {
    for requirements in ["[application.network]", "[malformed"] {
        let (_release, gate) = tokio::sync::oneshot::channel();
        let (destination, _) = start_streaming_sse_server(vec![vec![StreamingSseChunk {
            gate: Some(gate),
            body: String::new(),
        }]])
        .await;
        let home = TempDir::new()?;
        MockResponsesConfig::new(destination.uri()).write(home.path())?;
        write_models_cache(home.path()).await?;
        let mut app = TestAppServer::builder()
            .with_codex_home(home.path())
            .build_initialized()
            .await?;
        let thread = app.start_thread(ThreadStartParams::default()).await?.thread;
        let params = serde_json::json!({
            "threadId": thread.id, "input": [{"type": "text", "text": "Hello"}]
        });
        app.send_request("turn/start", Some(params)).await?;
        timeout(
            Duration::from_secs(/*secs*/ 30),
            destination.wait_for_request_count(/*count*/ 1),
        )
        .await?;
        std::fs::write(home.path().join("requirements.toml"), requirements)?;
        app.send_request("config/read", Some(serde_json::json!({})))
            .await?;
        let completed: TurnCompletedNotification = timeout(
            Duration::from_secs(/*secs*/ 5),
            app.read_notification("turn/completed"),
        )
        .await??;
        assert_eq!(completed.turn.status, TurnStatus::Failed);
        destination.shutdown().await;
    }
    Ok(())
}
