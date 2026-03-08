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

use crate::insert_history::write_spans;
use crate::history_cell::HistoryCell;
use chrono::Local;
use codex_core::config::Config;
use codex_login::CodexAuth;
use codex_model_provider_info::WireApi;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::TokenUsage;

pub(crate) use account::StatusAccountDisplay;
pub(crate) use card::StatusHistoryHandle;
#[cfg(test)]
pub(crate) use card::new_status_output;
#[cfg(test)]
pub(crate) use card::new_status_output_with_rate_limits;
pub(crate) use card::new_status_output_with_rate_limits_handle;
pub(crate) use helpers::format_directory_display;
pub(crate) use helpers::format_tokens_compact;
pub(crate) use helpers::plan_type_display_name;
pub(crate) use rate_limits::RateLimitSnapshotDisplay;
pub(crate) use rate_limits::RateLimitWindowDisplay;
#[cfg(test)]
pub(crate) use rate_limits::rate_limit_snapshot_display;
pub(crate) use rate_limits::rate_limit_snapshot_display_for_limit;

#[cfg(test)]
mod tests;

pub(crate) async fn render_status_lines_for_cli(
    config: &Config,
    auth: Option<CodexAuth>,
    model_name: &str,
    width: u16,
) -> Vec<String> {
    let plan_type = auth.as_ref().and_then(CodexAuth::account_plan_type);
    let account_display = status_account_display_for_cli(auth.as_ref(), plan_type);

    let reasoning_effort_override = (config.model_provider.wire_api == WireApi::Responses).then(
        || {
            config
                .model_reasoning_effort
                .or_else(|| default_reasoning_effort_from_catalog(config, model_name))
        },
    );

    let total_usage = TokenUsage::default();
    let (output, _handle) = card::new_status_output_with_rate_limits_handle(
        config,
        account_display.as_ref(),
        /*token_info*/ None,
        &total_usage,
        &Option::<ThreadId>::None,
        /*thread_name*/ None,
        /*forked_from*/ None,
        &[],
        plan_type,
        Local::now(),
        model_name,
        /*collaboration_mode*/ None,
        reasoning_effort_override,
        /*refreshing_rate_limits*/ false,
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

fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
    let mut out = Vec::new();
    let _ = write_spans(&mut out, line.spans.iter());
    String::from_utf8_lossy(&out).into_owned()
}
