#![allow(clippy::unwrap_used)]

use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex_exec::test_codex_exec;
use wiremock::MockServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bare_prompt_runs_without_direct_and_sends_only_the_user_prompt() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let test = test_codex_exec();
    let server = MockServer::start().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-bare"),
            responses::ev_assistant_message("msg-bare", "Hi!"),
            responses::ev_completed("resp-bare"),
        ]),
    )
    .await;

    test.cmd_with_server(&server)
        .arg("--skip-git-repo-check")
        .arg("--bare-prompt")
        .arg("only this prompt")
        .assert()
        .success()
        .stdout(predicates::str::contains("Hi!"));

    let request = response_mock.single_request();
    assert_eq!(
        request.message_input_texts("user"),
        vec!["only this prompt"]
    );
    assert!(request.body_json().get("tools").is_none());
    Ok(())
}
