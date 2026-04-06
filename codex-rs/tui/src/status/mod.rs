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
use chrono::TimeZone;
use chrono::Utc;
use codex_core::config::Config;
use codex_login::CodexAuth;
use codex_login::token_data::parse_jwt_expiration;
use codex_model_provider_info::WireApi;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::TokenUsage;

pub(crate) use account::StatusAccountDisplay;
use account::truncate_status_email_local_part;
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
    let account_display = status_account_display_for_cli(
        auth.as_ref(),
        plan_type,
        cli_status_email_prefix_emoji(auth.as_ref()).map(str::to_string),
    );

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

pub(crate) async fn render_compact_status_for_cli(
    config: &Config,
    auth: Option<&CodexAuth>,
    use_utc: bool,
) -> String {
    let compact_usage = compact_status_usage(config, auth).await;
    let timestamp_with_timezone =
        compact_status_timestamp_with_timezone(compact_usage.reset_at_unix, use_utc);
    let email = truncate_status_email_local_part(auth.and_then(CodexAuth::get_account_email))
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{timestamp_with_timezone} {email} {}%",
        compact_usage.percent_left
    )
}

fn compact_status_timestamp_with_timezone(reset_at_unix: Option<i64>, use_utc: bool) -> String {
    let utc_dt = reset_at_unix
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now);
    if use_utc {
        return utc_dt.format("%Y%m%d%H%M").to_string();
    }

    let local_dt = utc_dt.with_timezone(&Local);
    let offset_seconds = local_dt.offset().local_minus_utc();
    let timezone_suffix = if offset_seconds == 0 {
        String::new()
    } else if offset_seconds % 3600 == 0 {
        let hours = offset_seconds / 3600;
        if hours > 0 {
            format!("+{hours}")
        } else {
            hours.to_string()
        }
    } else {
        let sign = if offset_seconds.is_negative() { '-' } else { '+' };
        let total_minutes = offset_seconds.unsigned_abs() / 60;
        let hours = total_minutes / 60;
        let minutes = total_minutes % 60;
        format!("{sign}{hours:02}{minutes:02}")
    };
    format!("{}{timezone_suffix}", local_dt.format("%Y%m%d%H%M"))
}

#[derive(Debug, Clone, Copy)]
struct CompactStatusUsage {
    percent_left: i64,
    reset_at_unix: Option<i64>,
}

async fn compact_status_usage(config: &Config, auth: Option<&CodexAuth>) -> CompactStatusUsage {
    let Some(auth) = auth else {
        return CompactStatusUsage {
            percent_left: 0,
            reset_at_unix: None,
        };
    };
    let _ = config;
    let token_expiry_reset_at_unix = auth
        .get_token_data()
        .ok()
        .and_then(|token_data| parse_jwt_expiration(&token_data.access_token).ok())
        .flatten()
        .map(|expires_at| expires_at.timestamp());
    CompactStatusUsage {
        percent_left: 0,
        reset_at_unix: token_expiry_reset_at_unix,
    }
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
    email_prefix_emoji: Option<String>,
) -> Option<StatusAccountDisplay> {
    match auth {
        Some(auth) if auth.is_api_key_auth() => Some(StatusAccountDisplay::ApiKey),
        Some(auth) => Some(StatusAccountDisplay::ChatGpt {
            email_prefix_emoji,
            email: truncate_status_email_local_part(auth.get_account_email()),
            plan: plan_type.map(plan_type_display_name),
        }),
        None => None,
    }
}

fn cli_status_email_prefix_emoji(auth: Option<&CodexAuth>) -> Option<&'static str> {
    let auth = auth?;
    if auth.is_api_key_auth() {
        return None;
    }
    match auth_access_token_state(auth) {
        AuthAccessTokenState::Healthy => Some("✅"),
        AuthAccessTokenState::Expired => Some("⏰"),
        AuthAccessTokenState::Unknown => Some("❔"),
    }
}

enum AuthAccessTokenState {
    Healthy,
    Expired,
    Unknown,
}

fn auth_access_token_state(auth: &CodexAuth) -> AuthAccessTokenState {
    let token_data = match auth.get_token_data() {
        Ok(token_data) => token_data,
        Err(_) => return AuthAccessTokenState::Unknown,
    };
    match parse_jwt_expiration(&token_data.access_token) {
        Ok(Some(exp)) if exp <= Utc::now() => AuthAccessTokenState::Expired,
        Ok(Some(_)) => AuthAccessTokenState::Healthy,
        Ok(None) | Err(_) => AuthAccessTokenState::Unknown,
    }
}

fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
    let mut out = Vec::new();
    let _ = write_spans(&mut out, line.spans.iter());
    String::from_utf8_lossy(&out).into_owned()
}
