use anyhow::Context;
use clap::Parser;
use clap::ValueHint;
use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::ModelClient;
use codex_core::Prompt;
use codex_core::ResponseEvent;
use codex_core::ResponseStream;
use codex_core::auth::enforce_login_restrictions;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::features::Feature;
use codex_core::models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_core::models_manager::manager::ModelsManager;
use codex_core::models_manager::manager::RefreshStrategy;
use codex_otel::SessionTelemetry;
use codex_otel::TelemetryAuthMode;
use codex_protocol::ThreadId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsage;
use codex_utils_cli::CliConfigOverrides;
use futures::StreamExt;
use std::io::IsTerminal;
use std::io::Read;
use std::io::Write;
use std::sync::Arc;

/// Run a single prompt directly against the configured model.
#[derive(Debug, Parser)]
pub struct PromptCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    /// Model to use for this request.
    #[arg(long, short = 'm', value_name = "MODEL")]
    pub model: Option<String>,

    /// Override the developer/system instructions for this request.
    #[arg(long = "system", short = 's', value_name = "SYSTEM_PROMPT")]
    pub system_prompt: Option<String>,

    /// List models that can be used with `codex prompt`.
    #[arg(long = "models", conflicts_with = "prompt", default_value_t = false)]
    pub list_models: bool,

    /// Prompt to send to the model. Use `-` to read from stdin.
    #[arg(value_name = "PROMPT", value_hint = ValueHint::Other)]
    pub prompt: Option<String>,

    /// Print the outgoing JSON request and incoming SSE payloads.
    #[arg(long = "debug", default_value_t = false)]
    pub debug: bool,
}

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant. Respond directly to the user request without running tools or shell commands.";

pub async fn run_prompt_command(cli: PromptCli) -> anyhow::Result<()> {
    let PromptCli {
        config_overrides,
        model,
        system_prompt,
        list_models,
        prompt,
        debug,
    } = cli;

    let prompt_text = if list_models {
        None
    } else {
        Some(read_prompt(prompt)?)
    };

    let system_prompt = system_prompt
        .clone()
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

    let config = Arc::new(load_config(config_overrides, model).await?);
    let auth_manager = AuthManager::shared(
        config.codex_home.clone(),
        true,
        config.cli_auth_credentials_store_mode,
    );
    let models_manager = ModelsManager::new(
        config.codex_home.clone(),
        Arc::clone(&auth_manager),
        config.model_catalog.clone(),
        CollaborationModesConfig::default(),
    );

    if list_models {
        print_models(&models_manager).await;
        return Ok(());
    }

    if let Err(err) = enforce_login_restrictions(&config) {
        eprintln!("{err}");
        std::process::exit(1);
    }

    let prompt_text = prompt_text.ok_or_else(|| anyhow::anyhow!("prompt is required"))?;
    run_prompt(
        prompt_text,
        system_prompt,
        config,
        auth_manager,
        models_manager,
        debug,
    )
    .await
}

async fn load_config(
    config_overrides: CliConfigOverrides,
    model: Option<String>,
) -> anyhow::Result<Config> {
    let overrides = ConfigOverrides {
        model,
        review_model: None,
        cwd: None,
        approval_policy: None,
        sandbox_mode: None,
        model_provider: None,
        service_tier: None,
        config_profile: None,
        codex_linux_sandbox_exe: None,
        main_execve_wrapper_exe: None,
        js_repl_node_path: None,
        js_repl_node_module_dirs: None,
        zsh_path: None,
        base_instructions: None,
        developer_instructions: None,
        personality: None,
        compact_prompt: None,
        include_apply_patch_tool: Some(false),
        show_raw_agent_reasoning: None,
        tools_web_search_request: Some(false),
        ephemeral: None,
        additional_writable_roots: Vec::new(),
    };

    let cli_overrides = config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;

    Config::load_with_cli_overrides_and_harness_overrides(cli_overrides, overrides)
        .await
        .map_err(anyhow::Error::from)
}

fn read_prompt(prompt: Option<String>) -> anyhow::Result<String> {
    match prompt {
        Some(p) if p != "-" => Ok(p),
        other => {
            let force_stdin = matches!(other.as_deref(), Some("-"));
            if std::io::stdin().is_terminal() && !force_stdin {
                anyhow::bail!("No prompt provided. Pass one as an argument or pipe it via stdin.");
            }
            if !force_stdin {
                eprintln!("Reading prompt from stdin...");
            }
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("Failed to read prompt from stdin")?;
            if buffer.trim().is_empty() {
                anyhow::bail!("No prompt provided via stdin.");
            }
            Ok(buffer)
        }
    }
}

async fn print_models(models_manager: &ModelsManager) {
    let presets = models_manager
        .list_models(RefreshStrategy::OnlineIfUncached)
        .await;
    if presets.is_empty() {
        println!("No models are currently available.");
        return;
    }

    println!("Available models:");
    for preset in presets {
        let default_marker = if preset.is_default { " (default)" } else { "" };
        println!(
            "  {}{} - {}",
            preset.model, default_marker, preset.description
        );
        println!(
            "    Default reasoning effort: {}",
            preset.default_reasoning_effort
        );
        if !preset.supported_reasoning_efforts.is_empty() {
            println!("    Supported reasoning efforts:");
            for option in preset.supported_reasoning_efforts {
                println!("      - {}: {}", option.effort, option.description);
            }
        }
    }
}

async fn run_prompt(
    prompt_text: String,
    system_prompt: String,
    config: Arc<Config>,
    auth_manager: Arc<AuthManager>,
    models_manager: ModelsManager,
    debug_http: bool,
) -> anyhow::Result<()> {
    if debug_http {
        eprintln!("--debug is not yet wired to raw HTTP/SSE tracing on this branch.");
    }

    let auth_snapshot = auth_manager.auth().await;
    let provider = config.model_provider.clone();
    let conversation_id = ThreadId::new();
    let model = models_manager
        .get_default_model(&config.model, RefreshStrategy::OnlineIfUncached)
        .await;
    let model_info: ModelInfo = models_manager.get_model_info(&model, &config).await;
    let telemetry_auth_mode = auth_snapshot.as_ref().map(|auth| match auth.auth_mode() {
        codex_core::auth::AuthMode::ApiKey => TelemetryAuthMode::ApiKey,
        codex_core::auth::AuthMode::Chatgpt => TelemetryAuthMode::Chatgpt,
    });
    let session_telemetry = SessionTelemetry::new(
        conversation_id,
        model.as_str(),
        model_info.slug.as_str(),
        auth_snapshot.as_ref().and_then(CodexAuth::get_account_id),
        auth_snapshot
            .as_ref()
            .and_then(CodexAuth::get_account_email),
        telemetry_auth_mode,
        "codex prompt".to_string(),
        config.otel.log_user_prompt,
        codex_core::terminal::user_agent(),
        SessionSource::Cli,
    );

    let mut prompt = Prompt::default();
    prompt.input = build_prompt_inputs(&system_prompt, &prompt_text);
    if let Some(base_instructions) = &config.base_instructions {
        prompt.base_instructions.text = base_instructions.clone();
    }
    let mut client_session = ModelClient::new(
        Some(auth_manager),
        conversation_id,
        provider,
        SessionSource::Cli,
        config.model_verbosity,
        codex_core::ws_version_from_features(config.as_ref()),
        config.features.enabled(Feature::EnableRequestCompression),
        config.features.enabled(Feature::RuntimeMetrics),
        None,
    )
    .new_session();
    let reasoning_summary = config
        .model_reasoning_summary
        .unwrap_or(model_info.default_reasoning_summary);
    let mut stream = client_session
        .stream(
            &prompt,
            &model_info,
            &session_telemetry,
            config.model_reasoning_effort,
            reasoning_summary,
            config.service_tier,
            None,
        )
        .await?;

    consume_stream(&mut stream).await
}

fn build_prompt_inputs(system_prompt: &str, prompt_text: &str) -> Vec<ResponseItem> {
    vec![
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: system_prompt.to_string(),
            }],
            end_turn: None,
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: prompt_text.to_string(),
            }],
            end_turn: None,
            phase: None,
        },
    ]
}

async fn consume_stream(stream: &mut ResponseStream) -> anyhow::Result<()> {
    use owo_colors::OwoColorize;
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let mut printed_response = false;
    let mut reasoning_summary_line = String::new();
    let mut summary_active = false;

    let flush_summary = |summary_active: &mut bool, reasoning_summary_line: &mut String| -> bool {
        if *summary_active {
            if !reasoning_summary_line.is_empty() {
                eprintln!();
                reasoning_summary_line.clear();
            } else {
                eprintln!();
            }
            *summary_active = false;
            true
        } else {
            false
        }
    };

    while let Some(event) = stream.next().await {
        match event? {
            ResponseEvent::Created => {}
            ResponseEvent::OutputTextDelta(delta) => {
                if flush_summary(&mut summary_active, &mut reasoning_summary_line) {
                    stderr.flush()?;
                }
                stdout.write_all(delta.as_bytes())?;
                stdout.flush()?;
                printed_response = true;
            }
            ResponseEvent::OutputItemAdded(item) | ResponseEvent::OutputItemDone(item) => {
                if flush_summary(&mut summary_active, &mut reasoning_summary_line) {
                    stderr.flush()?;
                }
                if let Some(text) = assistant_text(&item)
                    && !printed_response
                {
                    stdout.write_all(text.as_bytes())?;
                    stdout.flush()?;
                    printed_response = true;
                }
            }
            ResponseEvent::ReasoningSummaryDelta { delta, .. } => {
                reasoning_summary_line.push_str(&delta);
                summary_active = true;
                let colored = format!("(reasoning summary) {reasoning_summary_line}");
                eprint!("\r{}", colored.yellow());
                stderr.flush()?;
            }
            ResponseEvent::ReasoningContentDelta { delta, .. } => {
                eprintln!("(reasoning detail) {delta}");
            }
            ResponseEvent::ReasoningSummaryPartAdded { .. } => {
                if flush_summary(&mut summary_active, &mut reasoning_summary_line) {
                    stderr.flush()?;
                }
            }
            ResponseEvent::RateLimits(snapshot) => {
                eprintln!("Rate limits: {snapshot:?}");
            }
            ResponseEvent::ServerModel(_)
            | ResponseEvent::ServerReasoningIncluded(_)
            | ResponseEvent::ModelsEtag(_) => {}
            ResponseEvent::Completed { token_usage, .. } => {
                if flush_summary(&mut summary_active, &mut reasoning_summary_line) {
                    stderr.flush()?;
                }
                if printed_response {
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                    printed_response = false;
                }
                if let Some(usage) = token_usage {
                    print_token_usage(&usage);
                }
            }
        }
    }

    if printed_response {
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    if flush_summary(&mut summary_active, &mut reasoning_summary_line) {
        stderr.flush()?;
    }
    Ok(())
}

fn assistant_text(item: &ResponseItem) -> Option<String> {
    if let ResponseItem::Message { role, content, .. } = item
        && role == "assistant"
    {
        let mut text = String::new();
        for chunk in content {
            match chunk {
                ContentItem::InputText { text: value }
                | ContentItem::OutputText { text: value } => text.push_str(value),
                ContentItem::InputImage { .. } => {}
            }
        }
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

fn print_token_usage(usage: &TokenUsage) {
    eprintln!(
        "Token usage: total={} input={} cached_input={} output={} reasoning_output={}",
        usage.total_tokens,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_text_handles_basic_message() {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![
                ContentItem::OutputText {
                    text: "Hello".to_string(),
                },
                ContentItem::OutputText {
                    text: " world".to_string(),
                },
            ],
            end_turn: None,
            phase: None,
        };
        assert_eq!(assistant_text(&item), Some("Hello world".to_string()));
    }
}
