use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;

const TOKENS_PER_MILLION: f64 = 1_000_000.0;
pub const MODEL_PRICING_FILENAME: &str = "model_pricing.json";
pub const CREDITS_PER_USD: f64 = 25.0;
const BUNDLED_MODEL_PRICING_JSON: &str = include_str!("default_model_pricing.json");

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct UsagePriceWeights {
    pub(crate) input: f64,
    pub(crate) cached_input: f64,
    pub(crate) output: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelPricingEntry {
    pub input_credits_per_million: f64,
    pub cached_input_credits_per_million: f64,
    pub output_credits_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelPricingFile {
    pub version: u32,
    pub default_model: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default = "default_credits_per_usd")]
    pub credits_per_usd: f64,
    pub models: BTreeMap<String, ModelPricingEntry>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

fn default_credits_per_usd() -> f64 {
    CREDITS_PER_USD
}

impl ModelPricingFile {
    pub fn bundled_default() -> anyhow::Result<Self> {
        serde_json::from_str(BUNDLED_MODEL_PRICING_JSON)
            .context("parse bundled default model pricing")
    }

    pub fn from_pricing_page_html(
        html: &str,
        source_url: &str,
        updated_at: &str,
    ) -> anyhow::Result<Self> {
        let normalized = normalize_html_for_pricing_parse(html);
        let mut models = BTreeMap::new();
        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        for window in tokens.windows(4) {
            let Some(model_token) = window.first() else {
                continue;
            };
            if !looks_like_priced_model_token(model_token) {
                continue;
            }

            let model = (*model_token).to_string();
            if models.contains_key(&model) {
                continue;
            }

            let Some(input_usd) = parse_usd_token(window[1]) else {
                continue;
            };
            let Some(cached_input_usd) = parse_usd_token(window[2]) else {
                continue;
            };
            let Some(output_usd) = parse_usd_token(window[3]) else {
                continue;
            };

            models.insert(
                model,
                ModelPricingEntry {
                    input_credits_per_million: input_usd * CREDITS_PER_USD,
                    cached_input_credits_per_million: cached_input_usd * CREDITS_PER_USD,
                    output_credits_per_million: output_usd * CREDITS_PER_USD,
                },
            );
        }

        if models.is_empty() {
            anyhow::bail!("did not find any pricing rows in pricing page HTML");
        }

        let mut aliases = bundled_aliases();
        if models.contains_key("gpt-5.4-mini") {
            aliases.insert("gpt-5.1-codex-mini".to_string(), "gpt-5.4-mini".to_string());
            aliases.insert("gpt-5-codex-mini".to_string(), "gpt-5.4-mini".to_string());
        }
        if models.contains_key("gpt-5.3-codex") {
            aliases.insert("gpt-5.2-codex".to_string(), "gpt-5.3-codex".to_string());
            aliases.insert("gpt-5.2".to_string(), "gpt-5.3-codex".to_string());
        }

        Ok(Self {
            version: 1,
            default_model: "gpt-5.3-codex".to_string(),
            source_url: Some(source_url.to_string()),
            updated_at: Some(updated_at.to_string()),
            credits_per_usd: CREDITS_PER_USD,
            models,
            aliases,
        })
    }

    pub(crate) fn default_weights(&self) -> UsagePriceWeights {
        self.weights_for_model(Some(self.default_model.as_str()))
    }

    pub(crate) fn weights_for_model(&self, model_slug: Option<&str>) -> UsagePriceWeights {
        let resolved_slug = model_slug
            .and_then(|slug| self.resolve_slug(slug))
            .unwrap_or(self.default_model.as_str());

        self.models
            .get(resolved_slug)
            .or_else(|| self.models.get(self.default_model.as_str()))
            .map(UsagePriceWeights::from_entry)
            .unwrap_or_default()
    }

    fn resolve_slug<'a>(&'a self, model_slug: &'a str) -> Option<&'a str> {
        if self.models.contains_key(model_slug) {
            return Some(model_slug);
        }

        self.aliases.get(model_slug).map(String::as_str)
    }
}

impl Default for UsagePriceWeights {
    fn default() -> Self {
        ModelPricingFile::bundled_default()
            .map(|pricing| pricing.default_weights())
            .unwrap_or(Self {
                input: 0.0,
                cached_input: 0.0,
                output: 0.0,
            })
    }
}

impl UsagePriceWeights {
    fn from_entry(entry: &ModelPricingEntry) -> Self {
        Self {
            input: entry.input_credits_per_million / TOKENS_PER_MILLION,
            cached_input: entry.cached_input_credits_per_million / TOKENS_PER_MILLION,
            output: entry.output_credits_per_million / TOKENS_PER_MILLION,
        }
    }
}

pub fn model_pricing_path(codex_home: &Path) -> PathBuf {
    codex_home.join(MODEL_PRICING_FILENAME)
}

pub fn load_model_pricing(codex_home: &Path) -> anyhow::Result<ModelPricingFile> {
    let pricing_path = model_pricing_path(codex_home);
    match std::fs::read_to_string(&pricing_path) {
        Ok(contents) => serde_json::from_str(&contents)
            .with_context(|| format!("parse model pricing file at {}", pricing_path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ModelPricingFile::bundled_default(),
        Err(err) => Err(err)
            .with_context(|| format!("read model pricing file at {}", pricing_path.display())),
    }
}

pub fn write_model_pricing(path: &Path, pricing: &ModelPricingFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create pricing directory {}", parent.display()))?;
    }
    let serialized = serde_json::to_string_pretty(pricing).context("serialize model pricing")?;
    std::fs::write(path, serialized)
        .with_context(|| format!("write model pricing file at {}", path.display()))
}

fn bundled_aliases() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("gpt-5.2-codex".to_string(), "gpt-5.3-codex".to_string()),
        ("gpt-5.2".to_string(), "gpt-5.3-codex".to_string()),
        ("gpt-5.1-codex-mini".to_string(), "gpt-5.4-mini".to_string()),
        ("gpt-5-codex-mini".to_string(), "gpt-5.4-mini".to_string()),
    ])
}

fn normalize_html_for_pricing_parse(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if starts_with_ascii_case_insensitive(bytes, i, b"<script") {
            i = skip_html_element(bytes, i, b"script");
            out.push(' ');
            continue;
        }
        if starts_with_ascii_case_insensitive(bytes, i, b"<style") {
            i = skip_html_element(bytes, i, b"style");
            out.push(' ');
            continue;
        }
        if bytes[i] == b'<' {
            i = skip_html_tag(bytes, i);
            out.push(' ');
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    collapse_whitespace(
        &out.replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&#36;", "$"),
    )
}

fn looks_like_priced_model_token(token: &str) -> bool {
    token.starts_with("gpt-") && token.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
}

fn parse_usd_token(token: &str) -> Option<f64> {
    let trimmed = token.strip_prefix('$')?;
    trimmed.parse().ok()
}

fn starts_with_ascii_case_insensitive(bytes: &[u8], index: usize, prefix: &[u8]) -> bool {
    bytes
        .get(index..index.saturating_add(prefix.len()))
        .is_some_and(|slice| slice.eq_ignore_ascii_case(prefix))
}

fn skip_html_tag(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] != b'>' {
        index += 1;
    }
    index.saturating_add(1)
}

fn skip_html_element(bytes: &[u8], start: usize, element_name: &[u8]) -> usize {
    let after_open_tag = skip_html_tag(bytes, start);
    let mut close_marker = Vec::with_capacity(element_name.len() + 3);
    close_marker.extend_from_slice(b"</");
    close_marker.extend_from_slice(element_name);

    let mut index = after_open_tag;
    while index < bytes.len() {
        if starts_with_ascii_case_insensitive(bytes, index, close_marker.as_slice()) {
            return skip_html_tag(bytes, index);
        }
        index += 1;
    }

    bytes.len()
}

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_whitespace = true;

    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_whitespace {
                out.push(' ');
                last_was_whitespace = true;
            }
            continue;
        }

        out.push(ch);
        last_was_whitespace = false;
    }

    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_page_html_is_parsed_into_credit_rows() {
        let html = r#"
            <section>
              <h2>Pricing</h2>
              <table>
                <tr><td>gpt-5.4</td><td>$2.50</td><td>$0.25</td><td>$15.00</td></tr>
                <tr><td>gpt-5.4-mini</td><td>$0.75</td><td>$0.075</td><td>$4.50</td></tr>
                <tr><td>gpt-5.3-codex</td><td>$1.75</td><td>$0.175</td><td>$14.00</td></tr>
                <tr><td>gpt-5.3-codex</td><td>$3.50</td><td>$0.35</td><td>$28.00</td></tr>
              </table>
            </section>
        "#;

        let pricing = ModelPricingFile::from_pricing_page_html(
            html,
            "https://developers.openai.com/api/docs/pricing",
            "2026-04-23T00:00:00Z",
        )
        .expect("parse pricing HTML");

        assert_eq!(pricing.models["gpt-5.4"].input_credits_per_million, 62.5);
        assert_eq!(pricing.models["gpt-5.4-mini"].output_credits_per_million, 112.5);
        assert_eq!(pricing.models["gpt-5.3-codex"].output_credits_per_million, 350.0);
        assert_eq!(pricing.aliases["gpt-5.2-codex"], "gpt-5.3-codex");
        assert_eq!(pricing.aliases["gpt-5.1-codex-mini"], "gpt-5.4-mini");
    }

    #[test]
    fn load_model_pricing_uses_external_file_when_present() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let pricing_path = model_pricing_path(tempdir.path());
        let pricing = ModelPricingFile {
            version: 1,
            default_model: "gpt-5.4".to_string(),
            source_url: None,
            updated_at: None,
            credits_per_usd: CREDITS_PER_USD,
            models: BTreeMap::from([(
                "gpt-5.4".to_string(),
                ModelPricingEntry {
                    input_credits_per_million: 99.0,
                    cached_input_credits_per_million: 9.0,
                    output_credits_per_million: 999.0,
                },
            )]),
            aliases: BTreeMap::new(),
        };
        write_model_pricing(&pricing_path, &pricing).expect("write pricing");

        let loaded = load_model_pricing(tempdir.path()).expect("load pricing");
        let weights = loaded.weights_for_model(Some("gpt-5.4"));

        assert_eq!(weights.input, 99.0 / TOKENS_PER_MILLION);
        assert_eq!(weights.cached_input, 9.0 / TOKENS_PER_MILLION);
        assert_eq!(weights.output, 999.0 / TOKENS_PER_MILLION);
    }
}
