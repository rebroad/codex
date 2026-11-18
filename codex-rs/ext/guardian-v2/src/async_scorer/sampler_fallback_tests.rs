use super::LunaSampler;
use super::ProxyPrewarmLimit;
use super::proxy_websocket_servers_with_http;
use super::sample_request;
use super::sampler_config;
use anyhow::Result;
use codex_config::types::AuthCredentialsStoreMode;
use codex_login::AuthKeyringBackendKind;
use codex_login::AuthManager;
use codex_login::test_support::transport_default_auth_route_config;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_limit_retries_guardian_review_with_auth2_json() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let primary = responses::start_websocket_server(vec![
        vec![vec![json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "code": "usage_limit_reached",
                    "message": "You've hit your usage limit"
                }
            }
        })]],
        vec![vec![json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "code": "usage_limit_reached",
                    "message": "You've hit your usage limit"
                }
            }
        })]],
        vec![vec![json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "code": "usage_limit_reached",
                    "message": "You've hit your usage limit"
                }
            }
        })]],
    ])
    .await;
    let http = responses::start_mock_server().await;
    let recovered = responses::mount_sse_once_match(
        &http,
        wiremock::matchers::header("authorization", "Bearer fallback-token"),
        responses::sse(vec![
            responses::ev_assistant_message("fallback-review", "low"),
            responses::ev_completed("fallback-review"),
        ]),
    )
    .await;
    let targets = vec![&primary, &primary, &primary, &primary];
    let base_url = proxy_websocket_servers_with_http(
        &targets,
        ProxyPrewarmLimit::AllConnections,
        Some(&http.uri()),
    )
    .await?;

    let home = TempDir::new()?;
    let auth_json = |access_token: &str| {
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "e30.e30.signature",
                "access_token": access_token,
                "refresh_token": format!("{access_token}-refresh"),
                "account_id": "workspace-one"
            },
            "last_refresh": "2026-11-21T00:00:00Z"
        })
    };
    fs::write(
        home.path().join("auth.json"),
        serde_json::to_vec(&auth_json("primary-token"))?,
    )?;
    fs::write(
        home.path().join("auth2.json"),
        serde_json::to_vec(&auth_json("fallback-token"))?,
    )?;
    let auth = AuthManager::shared(
        home.path().to_path_buf(),
        /*enable_codex_api_key_env*/ false,
        AuthCredentialsStoreMode::File,
        /*forced_chatgpt_workspace_id*/ None,
        /*chatgpt_base_url*/ None,
        AuthKeyringBackendKind::default(),
        transport_default_auth_route_config(),
    )
    .await;
    let mut config = sampler_config(base_url.clone());
    config.provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(Some(base_url)),
        Some(auth.clone()),
    );
    let sampler = LunaSampler::new(config);
    sampler.prewarm().await;

    assert_eq!(sampler.sample(sample_request("turn-1")).await?, "low");
    assert!(auth.fallback_auth_is_active());
    assert_eq!(
        auth.auth_cached().unwrap().get_token_data()?.access_token,
        "fallback-token"
    );
    assert!(primary.handshakes().iter().any(|handshake| {
        handshake.header("authorization").as_deref() == Some("Bearer primary-token")
    }));
    assert_eq!(
        recovered.single_request().header("authorization"),
        Some("Bearer fallback-token".to_owned())
    );
    Ok(())
}
