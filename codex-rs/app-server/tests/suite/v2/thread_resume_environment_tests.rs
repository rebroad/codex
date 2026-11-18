use super::*;

#[tokio::test]
async fn cold_resume_does_not_wait_for_executor_startup() -> Result<()> {
    skip_if_remote!(
        Ok(()),
        "the delayed executor fixture uses a host-local websocket"
    );

    let responses = responses::start_mock_server().await;
    let codex_home = TempDir::new()?;
    mock_responses_config(&responses.uri()).write(codex_home.path())?;
    let thread_id = create_fake_paginated_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .with_exec_server_delay(Duration::from_secs(30))
        .build_initialized()
        .await?;
    let resume_id = app
        .send_thread_resume_request(ThreadResumeParams {
            thread_id,
            exclude_turns: true,
            ..Default::default()
        })
        .await?;

    let _: ThreadResumeResponse =
        timeout(Duration::from_secs(5), app.read_response(resume_id)).await??;
    timeout(DEFAULT_READ_TIMEOUT, app.shutdown_gracefully()).await??;
    Ok(())
}
