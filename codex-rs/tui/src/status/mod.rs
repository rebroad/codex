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

use std::sync::Arc;

use crate::history_cell::HistoryCell;
use crate::insert_history::write_spans;
use chrono::Local;
use codex_backend_client::Client as BackendClient;
use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::WireApi;
use codex_core::config::Config;
use codex_core::models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_core::models_manager::manager::ModelsManager;
use codex_protocol::ThreadId;
use codex_protocol::protocol::CreditsSnapshot;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;
use codex_state::AccountUsageSnapshot;
use codex_state::AccountUsageStore;
use codex_state::account_usage_display;
use codex_state::account_usage_key;

#[cfg(test)]
pub(crate) use card::new_status_output;
pub(crate) use card::new_status_output_with_rate_limits;
pub(crate) use helpers::format_directory_display;
pub(crate) use helpers::format_tokens_compact;
pub(crate) use rate_limits::RateLimitSnapshotDisplay;
pub(crate) use rate_limits::RateLimitWindowDisplay;
#[cfg(test)]
pub(crate) use rate_limits::rate_limit_snapshot_display;
pub(crate) use rate_limits::rate_limit_snapshot_display_for_limit;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub(crate) struct AccountUsageDisplay {
    pub total_tokens: i64,
    pub estimated_percent: Option<f64>,
    pub sample_count: Option<i64>,
}

pub(crate) async fn render_status_lines_for_cli(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    model_name: &str,
    width: u16,
) -> Vec<String> {
    let auth = auth_manager.auth().await;
    let mut plan_type = auth.as_ref().and_then(CodexAuth::account_plan_type);
    let account_usage_store =
        AccountUsageStore::init(config.sqlite_home.clone(), config.model_provider_id.clone())
            .await
            .ok();
    let rate_limits = match auth.as_ref() {
        Some(auth) => fetch_rate_limits(config.chatgpt_base_url.clone(), auth.clone()).await,
        None => Vec::new(),
    };
    if let Some(auth) = auth.as_ref()
        && let Some(account_id) = account_usage_key(
            auth.get_account_id().as_deref(),
            auth.get_account_email().as_deref(),
        )
        && let Some(store) = account_usage_store.as_ref()
    {
        if let Some(account_display) = account_usage_display(auth.get_account_email().as_deref()) {
            store
                .cache_account_display(account_id.as_str(), account_display)
                .await;
        }
        for snapshot in &rate_limits {
            let _ = store
                .record_account_backend_rate_limit(account_id.as_str(), snapshot)
                .await;
        }
    }
    if let Some(rate_limit_plan) = rate_limits.iter().find_map(|snapshot| snapshot.plan_type) {
        plan_type = Some(rate_limit_plan);
    }
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
        let effective = match config.model_reasoning_effort {
            Some(effort) => Some(effort),
            None => {
                let models_manager = ModelsManager::new(
                    config.codex_home.clone(),
                    Arc::clone(&auth_manager),
                    config.model_catalog.clone(),
                    CollaborationModesConfig::default(),
                );
                models_manager
                    .get_model_info(model_name, config)
                    .await
                    .default_reasoning_level
            }
        };
        Some(effective)
    } else {
        None
    };

    let total_usage = TokenUsage::default();
    let account_usage =
        fetch_account_usage_display(account_usage_store.as_deref(), auth.as_ref()).await;
    let output = new_status_output_with_rate_limits(
        config,
        auth_manager.as_ref(),
        /*token_info*/ None,
        &total_usage,
        &Option::<ThreadId>::None,
        /*thread_name*/ None,
        /*forked_from*/ None,
        account_usage.as_ref(),
        rate_limit_displays.as_slice(),
        plan_type,
        Local::now(),
        model_name,
        /*collaboration_mode*/ None,
        reasoning_effort_override,
    );
    output
        .display_lines(width)
        .into_iter()
        .map(|line| line_to_ansi(&line))
        .collect()
}

pub(crate) async fn fetch_rate_limits(base_url: String, auth: CodexAuth) -> Vec<RateLimitSnapshot> {
    match BackendClient::from_auth(base_url, &auth) {
        Ok(client) => client.get_rate_limits_many().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn fetch_account_usage_display(
    store: Option<&AccountUsageStore>,
    auth: Option<&CodexAuth>,
) -> Option<AccountUsageDisplay> {
    let account_id = match auth.and_then(|auth| {
        account_usage_key(
            auth.get_account_id().as_deref(),
            auth.get_account_email().as_deref(),
        )
    }) {
        Some(account_id) => account_id,
        None => return None,
    };
    let store = store?;
    let usage = store.get_account_usage(account_id.as_str()).await.ok();
    let usage = match usage {
        Some(Some(usage)) => usage,
        Some(None) | None => return None,
    };

    let estimated_limit = store
        .estimate_account_limit_tokens(account_id.as_str())
        .await
        .ok()
        .unwrap_or((None, 0));
    Some(build_account_usage_display(&usage, estimated_limit))
}

fn build_account_usage_display(
    usage: &AccountUsageSnapshot,
    estimated_limit: (Option<f64>, i64),
) -> AccountUsageDisplay {
    let (limit, sample_count) = estimated_limit;
    let estimated_percent = estimate_account_usage_percent(usage, limit);
    AccountUsageDisplay {
        total_tokens: usage.total_tokens,
        estimated_percent,
        sample_count: Some(sample_count).filter(|count| *count > 0),
    }
}

fn estimate_account_usage_percent(
    usage: &AccountUsageSnapshot,
    estimated_limit: Option<f64>,
) -> Option<f64> {
    let estimated_limit = estimated_limit?;
    if estimated_limit <= 0.0 || !estimated_limit.is_finite() {
        return None;
    }
    let base_percent = usage.window_start_percent_int.unwrap_or(0) as f64;
    let window_start_total_tokens = usage.window_start_total_tokens.unwrap_or(0).max(0) as f64;
    let total_tokens = usage.total_tokens.max(0) as f64;
    let delta_tokens = (total_tokens - window_start_total_tokens).max(0.0);
    let avg_tokens_per_pct = estimated_limit / 100.0;
    if avg_tokens_per_pct <= 0.0 || !avg_tokens_per_pct.is_finite() {
        return None;
    }
    let percent = delta_tokens / avg_tokens_per_pct;
    Some(base_percent + percent)
}

fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
    let mut out = Vec::new();
    let _ = write_spans(&mut out, line.spans.iter());
    String::from_utf8_lossy(&out).into_owned()
}
