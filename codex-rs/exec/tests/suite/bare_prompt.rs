#![allow(clippy::unwrap_used)]

use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex_exec::test_codex_exec;
use std::path::Path;
use wiremock::MockServer;

fn rollout_files(directory: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut rollouts = Vec::new();
    if !directory.exists() {
        return Ok(rollouts);
    }
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            rollouts.extend(rollout_files(&path)?);
        } else if path.file_name().is_some_and(|name| {
            name.to_string_lossy().starts_with("rollout-")
                && path.extension().is_some_and(|ext| ext == "jsonl")
        }) {
            rollouts.push(path);
        }
    }
    Ok(rollouts)
}

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
            responses::ev_completed_with_tokens("resp-bare", 8),
        ]),
    )
    .await;

    test.cmd_with_server(&server)
        .arg("--skip-git-repo-check")
        .arg("--bareprompt")
        .arg("say hi")
        .assert()
        .success()
        .stdout(predicates::str::contains("Hi!"))
        .stderr(predicates::str::contains("tokens used\n8"));

    let request = response_mock.single_request();
    assert_eq!(request.message_input_texts("user"), vec!["say hi"]);
    assert_eq!(request.body_json()["input"].as_array().unwrap().len(), 1);
    assert_eq!(
        request.header("x-openai-internal-codex-responses-lite"),
        None
    );
    assert!(request.body_json().get("tools").is_none());
    assert_eq!(rollout_files(&test.home_path().join("sessions"))?.len(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_direct_does_not_create_a_rollout() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let test = test_codex_exec();
    let server = MockServer::start().await;
    responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-direct"),
            responses::ev_assistant_message("msg-direct", "Hi!"),
            responses::ev_completed("resp-direct"),
        ]),
    )
    .await;

    test.cmd_with_server(&server)
        .arg("--skip-git-repo-check")
        .arg("--direct")
        .arg("say hello")
        .assert()
        .success()
        .stdout(predicates::str::contains("Hi!"));

    assert!(rollout_files(&test.home_path().join("sessions"))?.is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bare_prompt_keeps_explicit_base_instructions() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let test = test_codex_exec();
    let server = MockServer::start().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-bare-override"),
            responses::ev_assistant_message("msg-bare-override", "Hi!"),
            responses::ev_completed("resp-bare-override"),
        ]),
    )
    .await;
    let instructions_path = test.cwd_path().join("instructions.md");
    std::fs::write(&instructions_path, "keep this explicit instruction")?;
    let instructions_path = instructions_path.to_string_lossy().replace('\\', "/");

    test.cmd_with_server(&server)
        .arg("-c")
        .arg(format!(
            "model_instructions_file={}",
            serde_json::to_string(&instructions_path)?
        ))
        .arg("--skip-git-repo-check")
        .arg("--bare-prompt")
        .arg("only this prompt")
        .assert()
        .success()
        .stdout(predicates::str::contains("Hi!"));

    let request = response_mock.single_request();
    assert_eq!(
        request.message_input_texts("developer"),
        vec!["keep this explicit instruction"]
    );
    assert_eq!(
        request.message_input_texts("user"),
        vec!["only this prompt"]
    );
    Ok(())
}
