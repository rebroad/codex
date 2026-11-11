use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::CodexErrorInfo;
use codex_app_server_protocol::ThreadGoalGetResponse;
use codex_app_server_protocol::ThreadGoalSetResponse;
use codex_app_server_protocol::ThreadGoalStatus;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStatus;
use codex_features::Feature;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn bad_request_keeps_goal_active_without_replaying_request() -> Result<()> {
    let responses_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "detail": "Bad Request"
        })))
        .mount(&responses_server)
        .await;

    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses_server.uri())
        .with_model("gpt-5.4")
        .enable_feature(Feature::Goals)
        .write(codex_home.path())?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_managed_config()
        .build_initialized()
        .await?;

    let request = server
        .send_thread_start_request_with_auto_env(ThreadStartParams::default())
        .await?;
    let ThreadStartResponse { thread, .. } = server.read_response(request).await?;
    let request = server
        .send_raw_request(
            "thread/goal/set",
            Some(json!({ "threadId": thread.id, "objective": "Finish the work" })),
        )
        .await?;
    let _: ThreadGoalSetResponse = server.read_response(request).await?;

    let completed: TurnCompletedNotification = server.read_notification("turn/completed").await?;
    assert_eq!(TurnStatus::Failed, completed.turn.status);
    assert_eq!(
        Some(CodexErrorInfo::HttpConnectionFailed {
            http_status_code: Some(400),
        }),
        completed
            .turn
            .error
            .and_then(|error| error.codex_error_info),
    );

    let request = server
        .send_raw_request("thread/goal/get", Some(json!({ "threadId": thread.id })))
        .await?;
    let goal: ThreadGoalGetResponse = server.read_response(request).await?;
    assert_eq!(
        Some(ThreadGoalStatus::Active),
        goal.goal.map(|goal| goal.status)
    );

    let requests = responses_server
        .received_requests()
        .await
        .expect("recorded response requests");
    assert_eq!(1, requests.len());
    Ok(())
}
