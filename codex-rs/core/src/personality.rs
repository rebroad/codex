use std::fs;
use std::io;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use codex_api::prompt_debug_http_enabled;
use codex_api::prompt_debug_http_log;
use codex_protocol::config_types::Personality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::PERSONALITY_PLACEHOLDER;
use tracing::warn;

const PERSONALITIES_DIRNAME: &str = "personalities";
const COMEDIC_PERSONALITY_TEMPLATE: &str = include_str!("personality_templates/comedic.md");
const COMEDIC_PERSONALITY_DESCRIPTION: &str = include_str!("personality_templates/comedic.txt");
const CAVEMAN_PERSONALITY_TEMPLATE: &str = include_str!("personality_templates/caveman.md");
const CAVEMAN_PERSONALITY_DESCRIPTION: &str = include_str!("personality_templates/caveman.txt");

const CORE_PERSONALITY_NAMES: &[&str] = &["none", "friendly", "pragmatic"];
const STARTER_PERSONALITIES: &[(&str, &str, &str)] = &[
    (
        "comedic",
        COMEDIC_PERSONALITY_TEMPLATE,
        COMEDIC_PERSONALITY_DESCRIPTION,
    ),
    (
        "caveman",
        CAVEMAN_PERSONALITY_TEMPLATE,
        CAVEMAN_PERSONALITY_DESCRIPTION,
    ),
];

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

pub fn ensure_personality_starter_files(codex_home: &Path) -> io::Result<()> {
    let dir = codex_home.join(PERSONALITIES_DIRNAME);
    fs::create_dir_all(&dir)?;
    for (name, template, description) in STARTER_PERSONALITIES {
        let path = personality_file_path(codex_home, name);
        if !path.exists() {
            fs::write(path, template)?;
        }
        let description_path = personality_description_file_path(codex_home, name);
        if !description_path.exists() {
            fs::write(description_path, description)?;
        }
    }
    Ok(())
}

pub fn discover_custom_personalities(codex_home: &Path) -> Vec<Personality> {
    let dir = codex_home.join(PERSONALITIES_DIRNAME);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut personalities = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension() != Some(OsStr::new("md")) {
                return None;
            }
            if is_personality_description_file(&path) {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.trim();
            if stem.is_empty()
                || CORE_PERSONALITY_NAMES
                    .iter()
                    .any(|builtin| stem.eq_ignore_ascii_case(builtin))
            {
                return None;
            }
            Some(Personality::Custom(stem.to_string()))
        })
        .collect::<Vec<_>>();

    personalities.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    personalities
}

pub fn personality_display_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn read_personality_description(codex_home: &Path, name: &str) -> Option<String> {
    let path = personality_description_file_path(codex_home, name);
    let text = fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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
    let bootstrap_template = starter_personality_template(name);
    let result = if let Some(template) = bootstrap_template {
        ensure_bootstrap_personality_file(&path, template)
            .and_then(|_| fs::read_to_string(&path))
            .map_err(|err| {
                warn!(path = ?path, error = %err, "failed to bootstrap built-in personality file");
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
            if let Some(template) = bootstrap_template {
                if prompt_debug_http_enabled() {
                    prompt_debug_http_log(format!(
                        "personality file route: name={name} path={} source=builtin-fallback bytes={}",
                        path.display(),
                        template.len()
                    ));
                }
                Some(template.to_string())
            } else {
                None
            }
        }
    }
}

fn starter_personality_template(name: &str) -> Option<&'static str> {
    match name {
        "comedic" => Some(COMEDIC_PERSONALITY_TEMPLATE),
        "caveman" => Some(CAVEMAN_PERSONALITY_TEMPLATE),
        _ => None,
    }
}

fn ensure_bootstrap_personality_file(path: &Path, template: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, template)
}

fn personality_file_path(codex_home: &Path, name: &str) -> PathBuf {
    codex_home
        .join(PERSONALITIES_DIRNAME)
        .join(format!("{name}.md"))
}

fn personality_description_file_path(codex_home: &Path, name: &str) -> PathBuf {
    codex_home
        .join(PERSONALITIES_DIRNAME)
        .join(format!("{name}.txt"))
}

fn is_personality_description_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".txt"))
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
    fn caveman_personality_bootstraps_to_file() {
        let tmp = tempdir().expect("tempdir");
        let message = resolve_personality_message(
            &test_model_info(None),
            tmp.path(),
            Some(Personality::Custom("caveman".to_string())),
        )
        .expect("caveman personality");
        assert!(message.contains("caveman"));
        let path = personality_file_path(tmp.path(), "caveman");
        assert!(path.exists());
        let file_text = fs::read_to_string(path).expect("read caveman file");
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

    #[test]
    fn discover_custom_personalities_skips_builtins_and_sorts() {
        let tmp = tempdir().expect("tempdir");
        let personalities_dir = tmp.path().join(PERSONALITIES_DIRNAME);
        fs::create_dir_all(&personalities_dir).expect("dir");
        fs::write(personality_file_path(tmp.path(), "zany"), "zany").expect("zany");
        fs::write(personality_file_path(tmp.path(), "friendly"), "ignored").expect("friendly");
        fs::write(personality_file_path(tmp.path(), "caveman"), "caveman").expect("caveman");
        fs::write(
            personality_description_file_path(tmp.path(), "caveman"),
            "description",
        )
        .expect("caveman description");

        let personalities = discover_custom_personalities(tmp.path());
        assert_eq!(
            personalities,
            vec![
                Personality::Custom("caveman".to_string()),
                Personality::Custom("zany".to_string()),
            ]
        );
    }

    #[test]
    fn discover_custom_personalities_includes_starter_files() {
        let tmp = tempdir().expect("tempdir");
        ensure_personality_starter_files(tmp.path()).expect("bootstrap personalities");

        let personalities = discover_custom_personalities(tmp.path());
        assert!(personalities.contains(&Personality::Custom("comedic".to_string())));
        assert!(personalities.contains(&Personality::Custom("caveman".to_string())));
    }

    #[test]
    fn ensure_personality_starter_files_writes_examples() {
        let tmp = tempdir().expect("tempdir");
        ensure_personality_starter_files(tmp.path()).expect("bootstrap personalities");
        assert!(personality_file_path(tmp.path(), "comedic").exists());
        assert!(personality_file_path(tmp.path(), "caveman").exists());
        assert!(personality_description_file_path(tmp.path(), "comedic").exists());
        assert!(personality_description_file_path(tmp.path(), "caveman").exists());
    }

    #[test]
    fn read_personality_description_returns_sidecar_contents() {
        let tmp = tempdir().expect("tempdir");
        fs::create_dir_all(tmp.path().join(PERSONALITIES_DIRNAME)).expect("dir");
        fs::write(
            personality_description_file_path(tmp.path(), "caveman"),
            "Terse, high-signal.\n",
        )
        .expect("write description");

        assert_eq!(
            read_personality_description(tmp.path(), "caveman"),
            Some("Terse, high-signal.".to_string())
        );
    }

    #[test]
    fn builtins_are_not_discovered_as_custom_personalities() {
        let tmp = tempdir().expect("tempdir");
        let personalities_dir = tmp.path().join(PERSONALITIES_DIRNAME);
        fs::create_dir_all(&personalities_dir).expect("dir");
        fs::write(personality_file_path(tmp.path(), "friendly"), "ignored").expect("friendly");
        fs::write(personality_file_path(tmp.path(), "pragmatic"), "ignored").expect("pragmatic");
        fs::write(personality_file_path(tmp.path(), "none"), "ignored").expect("none");

        let personalities = discover_custom_personalities(tmp.path());
        assert!(personalities.is_empty());
    }

    #[test]
    fn description_sidecars_are_not_discovered_as_personalities() {
        let tmp = tempdir().expect("tempdir");
        let personalities_dir = tmp.path().join(PERSONALITIES_DIRNAME);
        fs::create_dir_all(&personalities_dir).expect("dir");
        fs::write(personality_file_path(tmp.path(), "caveman"), "caveman").expect("caveman");
        fs::write(
            personality_description_file_path(tmp.path(), "caveman"),
            "description",
        )
        .expect("caveman description");

        let personalities = discover_custom_personalities(tmp.path());
        assert_eq!(personalities, vec![Personality::Custom("caveman".to_string())]);
    }
}
