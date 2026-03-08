//! Status output formatting and display adapters for the TUI.
//!
//! This module turns protocol-level snapshots into stable display structures used by `/status`
//! output and footer/status-line helpers, while keeping rendering concerns out of transport-facing
//! code.
//!
//! `rate_limits` is the main integration point for status-line usage-limit items: it converts raw
//! window snapshots into local-time labels and classifies data as available, stale, or missing.
mod account;
mod card;
mod format;
mod helpers;
mod rate_limits;

pub(crate) use account::StatusAccountDisplay;
#[cfg(test)]
pub(crate) use card::new_status_output;
pub(crate) use card::new_status_output_with_rate_limits;
pub(crate) use helpers::format_directory_display;
pub(crate) use helpers::format_tokens_compact;
pub(crate) use helpers::plan_type_display_name;
pub(crate) use rate_limits::RateLimitSnapshotDisplay;
pub(crate) use rate_limits::RateLimitWindowDisplay;
#[cfg(test)]
pub(crate) use rate_limits::rate_limit_snapshot_display;
pub(crate) use rate_limits::rate_limit_snapshot_display_for_limit;

use crate::history_cell::HistoryCell;
use crate::insert_history::write_spans;
use chrono::Local;
use codex_core::CodexAuth;
use codex_core::WireApi;
use codex_core::config::Config;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CreditsSnapshot;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;

#[cfg(test)]
mod tests;

pub(crate) async fn render_status_lines_for_cli(
    config: &Config,
    auth: Option<CodexAuth>,
    model_name: &str,
    width: u16,
) -> Vec<String> {
    let mut plan_type = auth.as_ref().and_then(CodexAuth::account_plan_type);
    let rate_limits = match auth.as_ref() {
        Some(auth) => fetch_rate_limits(config.chatgpt_base_url.clone(), auth.clone()).await,
        None => Vec::new(),
    };
    if let Some(rate_limit_plan) = rate_limits.iter().find_map(|snapshot| snapshot.plan_type) {
        plan_type = Some(rate_limit_plan);
    }

    let account_display = status_account_display_for_cli(auth.as_ref(), plan_type.clone());

    let mut credits_by_limit_id = std::collections::BTreeMap::<String, CreditsSnapshot>::new();
    let mut rate_limit_displays = Vec::with_capacity(rate_limits.len());
    for mut snapshot in rate_limits {
        let limit_id = snapshot
            .limit_id
            .clone()
            .unwrap_or_else(|| "codex".to_string());
        if snapshot.credits.is_none()
            && let Some(credits) = credits_by_limit_id.get(&limit_id).cloned()
        {
            snapshot.credits = Some(credits);
        }
        if let Some(credits) = snapshot.credits.clone() {
            credits_by_limit_id.insert(limit_id.clone(), credits);
        }
        let limit_name = snapshot
            .limit_name
            .clone()
            .unwrap_or_else(|| limit_id.clone());
        rate_limit_displays.push(rate_limit_snapshot_display_for_limit(
            &snapshot,
            limit_name,
            Local::now(),
        ));
    }

    let reasoning_effort_override = if config.model_provider.wire_api == WireApi::Responses {
        Some(
            config
                .model_reasoning_effort
                .or_else(|| default_reasoning_effort_from_catalog(config, model_name)),
        )
    } else {
        None
    };

    let total_usage = TokenUsage::default();
    let output = new_status_output_with_rate_limits(
        config,
        account_display.as_ref(),
        None,
        &total_usage,
        &Option::<ThreadId>::None,
        None,
        None,
        rate_limit_displays.as_slice(),
        plan_type,
        Local::now(),
        model_name,
        None,
        reasoning_effort_override,
    );
    output
        .display_lines(width)
        .into_iter()
        .map(|line| line_to_ansi(&line))
        .collect()
}

fn default_reasoning_effort_from_catalog(
    config: &Config,
    model_name: &str,
) -> Option<ReasoningEffort> {
    config.model_catalog.as_ref().and_then(|catalog| {
        catalog
            .models
            .iter()
            .find(|model| model.slug == model_name)
            .and_then(|model| model.default_reasoning_level)
    })
}

fn status_account_display_for_cli(
    auth: Option<&CodexAuth>,
    plan_type: Option<PlanType>,
) -> Option<StatusAccountDisplay> {
    match auth {
        Some(auth) if auth.is_api_key_auth() => Some(StatusAccountDisplay::ApiKey),
        Some(auth) => Some(StatusAccountDisplay::ChatGpt {
            email: auth.get_account_email(),
            plan: plan_type.map(plan_type_display_name),
        }),
        None => None,
    }
}

pub(crate) async fn fetch_rate_limits(base_url: String, auth: CodexAuth) -> Vec<RateLimitSnapshot> {
    let _ = (base_url, auth);
    Vec::new()
}

fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
    let mut out = Vec::new();
    let _ = write_spans(&mut out, line.spans.iter());
    String::from_utf8_lossy(&out).into_owned()
}
