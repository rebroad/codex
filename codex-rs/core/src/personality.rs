use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use codex_api::prompt_debug_http_enabled;
use codex_api::prompt_debug_http_log;
use codex_protocol::config_types::Personality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::PERSONALITY_PLACEHOLDER;
use tracing::warn;

const PERSONALITIES_DIRNAME: &str = "personalities";
const COMEDIC_PERSONALITY_TEMPLATE: &str = include_str!("personality_templates/comedic.md");

pub(crate) fn render_model_instructions(
    model_info: &ModelInfo,
    personality: Option<Personality>,
    codex_home: &Path,
) -> String {
    if let Some(model_messages) = &model_info.model_messages
        && let Some(template) = &model_messages.instructions_template
    {
        let personality_message = resolve_personality_message(model_info, codex_home, personality)
            .unwrap_or_default();
        return template.replace(PERSONALITY_PLACEHOLDER, personality_message.as_str());
    }

    if personality.is_some() {
        warn!(
            model = %model_info.slug,
            "Model personality requested but model_messages is missing, falling back to base instructions."
        );
    }

    model_info.base_instructions.clone()
}

pub(crate) fn resolve_personality_message(
    model_info: &ModelInfo,
    codex_home: &Path,
    personality: Option<Personality>,
) -> Option<String> {
    let personality = personality?;
    let model_variables = model_info
        .model_messages
        .as_ref()
        .and_then(|messages| messages.instructions_variables.as_ref());
    match personality {
        Personality::None => model_variables
            .and_then(|variables| variables.personality_default.clone())
            .or_else(|| Some(String::new())),
        Personality::Friendly | Personality::Pragmatic => {
            let from_model = match personality {
                Personality::Friendly => model_variables
                    .and_then(|variables| variables.personality_friendly.clone()),
                Personality::Pragmatic => model_variables
                    .and_then(|variables| variables.personality_pragmatic.clone()),
                Personality::None | Personality::Comedic | Personality::Custom(_) => None,
            };
            from_model.or_else(|| {
                codex_protocol::openai_models::builtin_personality_message(personality)
                    .map(str::to_string)
            })
        }
        Personality::Comedic => resolve_file_backed_personality(codex_home, "comedic"),
        Personality::Custom(name) => resolve_file_backed_personality(codex_home, &name),
    }
}

fn resolve_file_backed_personality(codex_home: &Path, name: &str) -> Option<String> {
    let path = personality_file_path(codex_home, name);
    let result = if name == "comedic" {
        ensure_bootstrap_personality_file(&path)
            .and_then(|_| fs::read_to_string(&path))
            .map_err(|err| {
                warn!(path = ?path, error = %err, "failed to bootstrap built-in comedic personality file");
                err
            })
    } else {
        fs::read_to_string(&path)
    };

    match result {
        Ok(text) => {
            if prompt_debug_http_enabled() {
                prompt_debug_http_log(format!(
                    "personality file route: name={name} path={} bytes={}",
                    path.display(),
                    text.len()
                ));
            }
            Some(text)
        }
        Err(err) => {
            warn!(name = name, path = ?path, error = %err, "personality file missing or unreadable");
            if name == "comedic" {
                if prompt_debug_http_enabled() {
                    prompt_debug_http_log(format!(
                        "personality file route: name={name} path={} source=builtin-fallback bytes={}",
                        path.display(),
                        COMEDIC_PERSONALITY_TEMPLATE.len()
                    ));
                }
                Some(COMEDIC_PERSONALITY_TEMPLATE.to_string())
            } else {
                None
            }
        }
    }
}

fn ensure_bootstrap_personality_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, COMEDIC_PERSONALITY_TEMPLATE)
}

fn personality_file_path(codex_home: &Path, name: &str) -> PathBuf {
    codex_home
        .join(PERSONALITIES_DIRNAME)
        .join(format!("{name}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::openai_models::ConfigShellToolType;
    use codex_protocol::openai_models::ModelInfo;
    use codex_protocol::openai_models::ModelMessages;
    use codex_protocol::openai_models::ModelVisibility;
    use codex_protocol::openai_models::ReasoningEffort;
    use codex_protocol::openai_models::ReasoningEffortPreset;
    use codex_protocol::openai_models::TruncationPolicyConfig;
    use codex_protocol::openai_models::default_input_modalities;
    use codex_protocol::config_types::ReasoningSummary;
    use tempfile::tempdir;

    fn test_model_info(model_messages: Option<ModelMessages>) -> ModelInfo {
        ModelInfo {
            slug: "test-model".to_string(),
            display_name: "Test Model".to_string(),
            description: None,
            default_reasoning_level: Some(ReasoningEffort::Medium),
            supported_reasoning_levels: vec![ReasoningEffortPreset {
                effort: ReasoningEffort::Medium,
                description: ReasoningEffort::Medium.to_string(),
            }],
            shell_type: ConfigShellToolType::UnifiedExec,
            visibility: ModelVisibility::List,
            supported_in_api: true,
            priority: 1,
            availability_nux: None,
            upgrade: None,
            base_instructions: "Base instructions".to_string(),
            model_messages,
            supports_reasoning_summaries: false,
            default_reasoning_summary: ReasoningSummary::Auto,
            support_verbosity: false,
            default_verbosity: None,
            apply_patch_tool_type: None,
            web_search_tool_type: Default::default(),
            truncation_policy: TruncationPolicyConfig::bytes(10_000),
            supports_parallel_tool_calls: false,
            supports_image_detail_original: false,
            context_window: Some(128_000),
            auto_compact_token_limit: None,
            effective_context_window_percent: 95,
            experimental_supported_tools: Vec::new(),
            input_modalities: default_input_modalities(),
            used_fallback_model_metadata: false,
            supports_search_tool: false,
        }
    }

    #[test]
    fn comedic_personality_bootstraps_to_file() {
        let tmp = tempdir().expect("tempdir");
        let message = resolve_personality_message(&test_model_info(None), tmp.path(), Some(Personality::Comedic))
            .expect("comedic personality");
        assert!(message.contains("Prioritize humor"));
        let path = personality_file_path(tmp.path(), "comedic");
        assert!(path.exists());
        let file_text = fs::read_to_string(path).expect("read comedic file");
        assert_eq!(file_text, message);
    }

    #[test]
    fn custom_personality_reads_from_file() {
        let tmp = tempdir().expect("tempdir");
        let path = personality_file_path(tmp.path(), "playful");
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, "custom personality").expect("write custom personality");

        let message = resolve_personality_message(
            &test_model_info(None),
            tmp.path(),
            Some(Personality::Custom("playful".to_string())),
        )
        .expect("custom personality");
        assert_eq!(message, "custom personality");
    }

    #[test]
    fn builtin_personalities_stay_in_code() {
        let tmp = tempdir().expect("tempdir");
        assert_eq!(
            resolve_personality_message(&test_model_info(None), tmp.path(), Some(Personality::Friendly))
                .expect("friendly"),
            codex_protocol::openai_models::builtin_personality_message(Personality::Friendly)
                .expect("builtin friendly")
        );
        assert_eq!(
            resolve_personality_message(&test_model_info(None), tmp.path(), Some(Personality::Pragmatic))
                .expect("pragmatic"),
            codex_protocol::openai_models::builtin_personality_message(Personality::Pragmatic)
                .expect("builtin pragmatic")
        );
    }
}
