use std::collections::BTreeMap;

use codex_protocol::config_types::Personality;
use codex_protocol::openai_models::ModelsResponse;

#[derive(Debug, Clone, Default)]
pub struct ModelsManagerConfig {
    pub model_replacements: BTreeMap<String, String>,
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub tool_output_token_limit: Option<usize>,
    pub base_instructions: Option<String>,
    pub personality: Option<Personality>,
    pub model_catalog: Option<ModelsResponse>,
}

impl ModelsManagerConfig {
    /// Rewrites configured model ID substrings in deterministic key order.
    pub fn rewrite_model_id(&self, model: &str) -> String {
        self.model_replacements
            .iter()
            .fold(model.to_string(), |model, (search, replace)| {
                model.replace(search, replace)
            })
    }
}
