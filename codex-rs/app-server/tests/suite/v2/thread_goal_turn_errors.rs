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
use core_test_support::responses;
use core_test_support::responses::sse_response;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn bad_request_restarts_goal_with_recovery_guidance() -> Result<()> {
    let responses_server = MockServer::start().await;
    let request_count = Arc::new(AtomicUsize::new(0));
    let success_body = responses::sse(vec![
        responses::ev_response_created("resp-recovery"),
        responses::ev_assistant_message("msg-recovery", "Continuing after the request error."),
        responses::ev_completed("resp-recovery"),
    ]);
    let request_count_for_response = Arc::clone(&request_count);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(move |_: &wiremock::Request| {
            if request_count_for_response.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(400).set_body_json(json!({"detail": "Bad Request"}))
            } else {
                sse_response(success_body.clone())
            }
        })
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

    let failed: TurnCompletedNotification = server.read_notification("turn/completed").await?;
    assert_eq!(TurnStatus::Failed, failed.turn.status);
    assert_eq!(
        Some(CodexErrorInfo::HttpConnectionFailed {
            http_status_code: Some(400)
        }),
        failed.turn.error.and_then(|error| error.codex_error_info),
    );

    let recovered: TurnCompletedNotification = server.read_notification("turn/completed").await?;
    assert_eq!(TurnStatus::Completed, recovered.turn.status);

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
    assert_eq!(2, requests.len());
    assert_ne!(requests[0].body, requests[1].body);
    Ok(())
}
