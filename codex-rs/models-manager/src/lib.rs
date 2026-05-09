pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;

pub use codex_app_server_protocol::AuthMode;
pub use codex_login::AuthManager;
pub use codex_login::CodexAuth;
pub use codex_model_provider_info::ModelProviderInfo;
pub use codex_model_provider_info::WireApi;
pub use config::ModelsManagerConfig;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::openai_models::ModelAvailabilityNux;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    let mut response: ModelsResponse = serde_json::from_str(include_str!("../models.json"))?;
    add_gpt_5_4_nano(&mut response);
    Ok(response)
}

fn add_gpt_5_4_nano(response: &mut ModelsResponse) {
    if response.models.iter().any(|model| model.slug == "gpt-5.4-nano") {
        return;
    }

    let Some(template) = response
        .models
        .iter()
        .find(|model| model.slug == "gpt-5.4")
        .cloned()
    else {
        return;
    };

    let mut model = template;
    model.slug = "gpt-5.4-nano".to_string();
    model.display_name = "gpt-5.4-nano".to_string();
    model.description = Some("Fast, low-cost model for high-volume simple tasks.".to_string());
    model.default_reasoning_level = Some(ReasoningEffort::Low);
    model.context_window = Some(400_000);
    model.priority = model.priority.saturating_add(1);
    model.upgrade = None;
    model.availability_nux = Some(ModelAvailabilityNux {
        message: "gpt-5.4-nano is available for fast, inexpensive tasks.".to_string(),
    });
    model.supports_parallel_tool_calls = true;
    model.supports_search_tool = false;
    model.supports_image_detail_original = true;
    model.web_search_tool_type = WebSearchToolType::Text;
    model.truncation_policy = TruncationPolicyConfig::tokens(10_000);
    model.supported_reasoning_levels = vec![
        ReasoningEffortPreset {
            effort: ReasoningEffort::Low,
            description: "Fast responses with lighter reasoning".to_string(),
        },
        ReasoningEffortPreset {
            effort: ReasoningEffort::Medium,
            description: "Balances speed and reasoning depth for everyday tasks".to_string(),
        },
    ];
    model.default_reasoning_summary = ReasoningSummary::None;
    model.support_verbosity = true;
    response.models.push(model);
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}
