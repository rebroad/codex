use super::AuthRequestTelemetryContext;
use super::ModelClient;
use super::PendingUnauthorizedRetry;
use super::UnauthorizedRecoveryExecution;
use chrono::Duration;
use chrono::Utc;
use codex_api::PromptDebugHttpConfig;
use codex_api::capture_dir as prompt_debug_capture_dir;
use codex_api::configure_prompt_debug_http;
use codex_api::set_prompt_debug_http_account_email;
use codex_login::AuthCredentialsStoreMode;
use codex_login::AuthDotJson;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::save_auth;
use codex_login::token_data::IdTokenInfo;
use codex_login::token_data::TokenData;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;
use serde_json::json;
use serial_test::serial;
use std::path::Path;
use tempfile::TempDir;

fn test_model_client(session_source: SessionSource) -> ModelClient {
    let provider = crate::model_provider_info::create_oss_provider_with_base_url(
        "https://example.com/v1",
        crate::model_provider_info::WireApi::Responses,
    );
    ModelClient::new(
        /*auth_manager*/ None,
        ThreadId::new(),
        provider,
        session_source,
        /*model_verbosity*/ None,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
    )
}

fn test_model_info() -> ModelInfo {
    serde_json::from_value(json!({
        "slug": "gpt-test",
        "display_name": "gpt-test",
        "description": "desc",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            {"effort": "medium", "description": "medium"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "upgrade": null,
        "base_instructions": "base instructions",
        "model_messages": null,
        "supports_reasoning_summaries": false,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 272000,
        "auto_compact_token_limit": null,
        "experimental_supported_tools": []
    }))
    .expect("deserialize test model info")
}

fn test_session_telemetry() -> SessionTelemetry {
    SessionTelemetry::new(
        ThreadId::new(),
        "gpt-test",
        "gpt-test",
        /*account_id*/ None,
        /*account_email*/ None,
        /*auth_mode*/ None,
        "test-originator".to_string(),
        /*log_user_prompts*/ false,
        "test-terminal".to_string(),
        SessionSource::Cli,
    )
}

fn test_chatgpt_auth(email: &str) -> CodexAuth {
    CodexAuth::ChatgptAuthTokens(AuthDotJson {
        auth_mode: Some(codex_app_server_protocol::AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(TokenData {
            id_token: IdTokenInfo {
                email: Some(email.to_string()),
                ..Default::default()
            },
            access_token: "Access Token".to_string(),
            refresh_token: "refresh-token".to_string(),
            account_id: Some("account_id".to_string()),
        }),
        last_refresh: Some(Utc::now()),
    })
}

fn stale_auth_dot_json(email: &str) -> AuthDotJson {
    AuthDotJson {
        auth_mode: Some(codex_app_server_protocol::AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(TokenData {
            id_token: IdTokenInfo {
                email: Some(email.to_string()),
                ..Default::default()
            },
            access_token: "stale-access-token".to_string(),
            refresh_token: "stale-refresh-token".to_string(),
            account_id: Some("account_id".to_string()),
        }),
        last_refresh: Some(Utc::now() - Duration::days(9)),
    }
}

fn fresh_auth_dot_json(email: &str) -> AuthDotJson {
    AuthDotJson {
        auth_mode: Some(codex_app_server_protocol::AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(TokenData {
            id_token: IdTokenInfo {
                email: Some(email.to_string()),
                ..Default::default()
            },
            access_token: "fresh-access-token".to_string(),
            refresh_token: "fresh-refresh-token".to_string(),
            account_id: Some("account_id".to_string()),
        }),
        last_refresh: Some(Utc::now() - Duration::days(1)),
    }
}

#[test]
fn build_subagent_headers_sets_other_subagent_label() {
    let client = test_model_client(SessionSource::SubAgent(SubAgentSource::Other(
        "memory_consolidation".to_string(),
    )));
    let headers = client.build_subagent_headers();
    let value = headers
        .get("x-openai-subagent")
        .and_then(|value| value.to_str().ok());
    assert_eq!(value, Some("memory_consolidation"));
}

#[tokio::test]
async fn summarize_memories_returns_empty_for_empty_input() {
    let client = test_model_client(SessionSource::Cli);
    let model_info = test_model_info();
    let session_telemetry = test_session_telemetry();

    let output = client
        .summarize_memories(
            Vec::new(),
            &model_info,
            /*effort*/ None,
            &session_telemetry,
        )
        .await
        .expect("empty summarize request should succeed");
    assert_eq!(output.len(), 0);
}

#[test]
fn auth_request_telemetry_context_tracks_attached_auth_and_retry_phase() {
    let auth_context = AuthRequestTelemetryContext::new(
        Some(codex_login::AuthMode::Chatgpt),
        &crate::api_bridge::CoreAuthProvider::for_test(Some("access-token"), Some("workspace-123")),
        PendingUnauthorizedRetry::from_recovery(UnauthorizedRecoveryExecution {
            mode: "managed",
            phase: "refresh_token",
        }),
    );

    assert_eq!(auth_context.auth_mode, Some("Chatgpt"));
    assert!(auth_context.auth_header_attached);
    assert_eq!(auth_context.auth_header_name, Some("authorization"));
    assert!(auth_context.retry_after_unauthorized);
    assert_eq!(auth_context.recovery_mode, Some("managed"));
    assert_eq!(auth_context.recovery_phase, Some("refresh_token"));
}

#[tokio::test]
#[serial(prompt_debug_http)]
async fn current_client_setup_refreshes_prompt_debug_email_from_reloaded_auth() {
    let tempdir = TempDir::new().expect("tempdir");
    let initial_auth = stale_auth_dot_json("old@example.com");
    let auth_manager = AuthManager::from_auth_for_testing_with_home(
        CodexAuth::ChatgptAuthTokens(initial_auth.clone()),
        tempdir.path().to_path_buf(),
    );
    save_auth(
        tempdir.path(),
        &fresh_auth_dot_json("new@example.com"),
        AuthCredentialsStoreMode::File,
    )
    .expect("save fresh auth");

    configure_prompt_debug_http(PromptDebugHttpConfig {
        enabled: true,
        capture_input: false,
        capture_output: false,
        capture_reasoning: false,
        capture_dir: Some(Path::new("/var/tmp/codex-prompt-debug-$EMAIL").to_path_buf()),
    });
    set_prompt_debug_http_account_email(Some("old@example.com".to_string()));

    let client = ModelClient::new(
        Some(auth_manager),
        ThreadId::new(),
        crate::model_provider_info::create_oss_provider_with_base_url(
            "https://example.com/v1",
            crate::model_provider_info::WireApi::Responses,
        ),
        SessionSource::Cli,
        /*model_verbosity*/ None,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
    );

    let before = prompt_debug_capture_dir().expect("capture dir before refresh");
    assert_eq!(before, Path::new("/var/tmp/codex-prompt-debug-old@example.com"));

    client
        .current_client_setup()
        .await
        .expect("current client setup");

    let after = prompt_debug_capture_dir().expect("capture dir after refresh");
    assert_eq!(after, Path::new("/var/tmp/codex-prompt-debug-new@example.com"));

    configure_prompt_debug_http(PromptDebugHttpConfig::default());
    set_prompt_debug_http_account_email(None);
}
