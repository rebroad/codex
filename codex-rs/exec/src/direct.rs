use anyhow::Result;
use codex_core::ModelClient;
use codex_core::Prompt;
use codex_core::ResponseEvent;
use codex_core::detached_memory_responses_metadata;
use codex_core::resolve_installation_id;
use codex_features::Feature;
use codex_login::AgentIdentityAuthPolicy;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_models_manager::manager::RefreshStrategy;
use codex_otel::SessionTelemetry;
use codex_otel::TelemetryAuthMode;
use codex_protocol::ThreadId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsage;
use codex_rollout_trace::InferenceTraceContext;
use codex_terminal_detection::user_agent;
use futures::StreamExt;
use std::io::Write;
use std::sync::Arc;

use codex_core::config::Config;

const DEFAULT_DIRECT_SYSTEM_PROMPT: &str = "You are a helpful assistant. Respond directly to the user request without running tools or shell commands.";

pub(crate) async fn run(prompt_text: String, config: &Config, json_mode: bool) -> Result<()> {
    let auth_manager = AuthManager::shared_from_config(config, true).await?;
    let models_manager = codex_core::build_models_manager(config, Arc::clone(&auth_manager));
    let model = models_manager
        .get_default_model(
            &config.model,
            true,
            RefreshStrategy::OnlineIfUncached,
            config.http_client_factory(),
        )
        .await;
    let model_info = models_manager
        .get_model_info(&model, &config.to_models_manager_config())
        .await;
    let auth_snapshot = auth_manager.auth().await;
    let thread_id = ThreadId::new();
    let session_source = SessionSource::Exec;
    let session_telemetry = SessionTelemetry::new(
        thread_id,
        &model,
        &model_info.slug,
        auth_snapshot.as_ref().and_then(CodexAuth::get_account_id),
        auth_snapshot
            .as_ref()
            .and_then(CodexAuth::get_account_email),
        auth_snapshot
            .as_ref()
            .map(|auth| TelemetryAuthMode::from(auth.auth_mode())),
        "codex exec --direct".to_string(),
        config.otel.log_user_prompt,
        user_agent(),
        session_source.clone(),
    );
    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let responses_metadata = detached_memory_responses_metadata(
        installation_id,
        thread_id.to_string(),
        thread_id.to_string(),
        format!("{thread_id}:0"),
        &session_source,
        &config.cwd,
        &config.permissions.effective_permission_profile(),
        None,
    )
    .await;

    let bare_prompt = config.bare_prompt;
    let mut prompt = Prompt::default();
    prompt.input = build_inputs(
        (!bare_prompt).then_some(DEFAULT_DIRECT_SYSTEM_PROMPT),
        &prompt_text,
    );
    if !bare_prompt && let Some(base_instructions) = config.base_instructions.as_deref() {
        prompt.base_instructions.text = base_instructions.to_string();
    }

    let mut client_session = ModelClient::new(
        Some(auth_manager),
        if config.features.enabled(Feature::UseAgentIdentity) {
            AgentIdentityAuthPolicy::ChatGptAuth
        } else {
            AgentIdentityAuthPolicy::JwtOnly
        },
        thread_id,
        config.model_provider.clone(),
        session_source,
        "codex exec --direct".to_string(),
        config.model_verbosity,
        config.features.enabled(Feature::ContentItemKinds),
        config.features.enabled(Feature::EnableRequestCompression),
        config.features.enabled(Feature::RuntimeMetrics),
        None,
        config
            .features
            .enabled(Feature::ConcurrentReasoningSummaries),
        None,
        config.http_client_factory(),
    )
    .new_session();
    let mut stream = client_session
        .stream(
            &prompt,
            &model_info,
            &session_telemetry,
            config.model_reasoning_effort.clone(),
            config
                .model_reasoning_summary
                .unwrap_or(model_info.default_reasoning_summary),
            config.service_tier.clone(),
            &responses_metadata,
            &InferenceTraceContext::disabled(),
        )
        .await?;

    let mut output = String::new();
    let mut usage = None;
    while let Some(event) = stream.next().await {
        match event? {
            ResponseEvent::OutputTextDelta(delta) => output.push_str(&delta),
            ResponseEvent::Completed { token_usage, .. } => usage = token_usage,
            _ => {}
        }
    }

    if json_mode {
        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "{}",
            serde_json::json!({
                "type": "turn.completed",
                "output": output,
                "usage": usage.map(usage_json),
            })
        )?;
    } else {
        let mut stdout = std::io::stdout().lock();
        write!(stdout, "{output}")?;
        if !output.ends_with('\n') {
            writeln!(stdout)?;
        }
        stdout.flush()?;
        if let Some(usage) = usage {
            print_usage(&usage);
        }
    }
    Ok(())
}

fn build_inputs(system_prompt: Option<&str>, prompt_text: &str) -> Vec<ResponseItem> {
    let mut inputs = Vec::with_capacity(2);
    if let Some(system_prompt) = system_prompt {
        inputs.push(ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: system_prompt.to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        });
    }
    inputs.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: prompt_text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    });
    inputs
}

fn usage_json(usage: TokenUsage) -> serde_json::Value {
    serde_json::json!({
        "input_tokens": usage.input_tokens,
        "cached_input_tokens": usage.cached_input_tokens,
        "output_tokens": usage.output_tokens,
        "reasoning_output_tokens": usage.reasoning_output_tokens,
    })
}

fn print_usage(usage: &TokenUsage) {
    eprintln!(
        "Token usage: input={} cached_input={} output={} reasoning_output={}",
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens,
    );
}
