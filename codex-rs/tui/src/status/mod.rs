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
use crate::history_cell::HistoryCell;
use crate::insert_history::write_spans;
use chrono::Local;
use chrono::Utc;
use codex_backend_client::Client as BackendClient;
use codex_core::CodexAuth;
use codex_core::WireApi;
use codex_core::config::Config;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CreditsSnapshot;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;
use codex_state::AccountUsageSnapshot;
use codex_state::AccountUsageStore;
use codex_state::account_usage_display;
use codex_state::account_usage_key;
pub(crate) use helpers::format_directory_display;
pub(crate) use helpers::format_tokens_compact;
pub(crate) use helpers::plan_type_display_name;
pub(crate) use rate_limits::RateLimitSnapshotDisplay;
pub(crate) use rate_limits::RateLimitWindowDisplay;
#[cfg(test)]
pub(crate) use rate_limits::rate_limit_snapshot_display;
pub(crate) use rate_limits::rate_limit_snapshot_display_for_limit;
use std::time::Duration;
use tokio::time::timeout;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub(crate) struct AccountUsageDisplay {
    pub total_tokens: i64,
    pub estimated_percent: Option<f64>,
    pub sample_count: Option<i64>,
}

const CLI_RATE_LIMIT_REFRESH_TTL_SECS: i64 = 120;
const CLI_RATE_LIMIT_FETCH_TIMEOUT_SECS: u64 = 8;

#[derive(Debug)]
enum CliRateLimitFetchOutcome {
    Skipped,
    Success(Vec<RateLimitSnapshot>),
    Timeout,
    BackendUnavailable(String),
    Other(String),
}

pub(crate) async fn render_status_lines_for_cli(
    config: &Config,
    auth: Option<CodexAuth>,
    model_name: &str,
    width: u16,
) -> Vec<String> {
    let mut plan_type = auth.as_ref().and_then(CodexAuth::account_plan_type);
    let account_usage_store =
        AccountUsageStore::init(config.sqlite_home.clone(), config.model_provider_id.clone())
            .await
            .ok();
    let account_id = auth.as_ref().and_then(|auth| {
        account_usage_key(
            auth.get_account_id().as_deref(),
            auth.get_account_email().as_deref(),
        )
    });

    let mut usage_snapshot = None;
    if let Some(auth) = auth.as_ref()
        && let Some(account_id) = account_id.as_ref()
        && let Some(store) = account_usage_store.as_ref()
    {
        if let Some(account_display) = account_usage_display(auth.get_account_email().as_deref()) {
            store
                .cache_account_display(account_id.as_str(), account_display)
                .await;
        }
        usage_snapshot = store
            .get_account_usage(account_id.as_str())
            .await
            .ok()
            .flatten();
    }

    let cached_rate_limits = usage_snapshot
        .as_ref()
        .and_then(cached_rate_limit_snapshot_from_usage);
    let should_refresh_rate_limits =
        should_refresh_cli_rate_limits(usage_snapshot.as_ref(), Utc::now().timestamp());

    let fetch_outcome = if let Some(auth) = auth.as_ref() {
        if should_refresh_rate_limits {
            fetch_rate_limits_for_cli(config.chatgpt_base_url.clone(), auth.clone()).await
        } else {
            CliRateLimitFetchOutcome::Skipped
        }
    } else {
        CliRateLimitFetchOutcome::Skipped
    };

    let fetched_rate_limits = match &fetch_outcome {
        CliRateLimitFetchOutcome::Success(snapshots) => snapshots.clone(),
        CliRateLimitFetchOutcome::Skipped
        | CliRateLimitFetchOutcome::Timeout
        | CliRateLimitFetchOutcome::BackendUnavailable(_)
        | CliRateLimitFetchOutcome::Other(_) => Vec::new(),
    };

    let mut rate_limits = if should_refresh_rate_limits {
        if fetched_rate_limits.is_empty() {
            cached_rate_limits.clone().into_iter().collect()
        } else {
            fetched_rate_limits.clone()
        }
    } else {
        cached_rate_limits.clone().into_iter().collect()
    };

    if rate_limits.is_empty()
        && let Some(cached) = cached_rate_limits.clone()
    {
        rate_limits.push(cached);
    }

    let limits_unavailable_reason = if rate_limits.is_empty() {
        Some(
            match (&auth, &fetch_outcome, cached_rate_limits.is_some()) {
                (None, _, _) => "not available: not logged in".to_string(),
                (_, CliRateLimitFetchOutcome::Timeout, _) => {
                    format!(
                        "not available: backend unavailable (timed out after {CLI_RATE_LIMIT_FETCH_TIMEOUT_SECS}s)"
                    )
                }
                (_, CliRateLimitFetchOutcome::BackendUnavailable(err), _) => {
                    format!("not available: backend unavailable ({err})")
                }
                (_, CliRateLimitFetchOutcome::Other(err), _) => {
                    format!("not available: other error ({err})")
                }
                (_, _, false) => "not available: no cached limits".to_string(),
                (_, _, true) => "not available: other reason".to_string(),
            },
        )
    } else {
        None
    };

    if let Some(account_id) = account_id.as_ref()
        && let Some(store) = account_usage_store.as_ref()
    {
        for snapshot in &fetched_rate_limits {
            let _ = store
                .record_account_backend_rate_limit(account_id.as_str(), snapshot)
                .await;
        }
    }
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

    let reasoning_effort_override = (config.model_provider.wire_api == WireApi::Responses).then(
        || {
            config
                .model_reasoning_effort
                .or_else(|| default_reasoning_effort_from_catalog(config, model_name))
        },
    );

    let total_usage = TokenUsage::default();
    let account_usage =
        fetch_account_usage_display(account_usage_store.as_deref(), auth.as_ref()).await;
    let output = card::new_status_output_with_rate_limits_variant(
        config,
        account_display.as_ref(),
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
        card::StatusCardVariant::Cli,
        limits_unavailable_reason,
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

fn should_refresh_cli_rate_limits(usage: Option<&AccountUsageSnapshot>, now_ts: i64) -> bool {
    let Some(usage) = usage else {
        return true;
    };
    let Some(last_seen_at) = usage.last_backend_seen_at else {
        return true;
    };

    if now_ts.saturating_sub(last_seen_at) >= CLI_RATE_LIMIT_REFRESH_TTL_SECS {
        return true;
    }

    let last_snapshot_total = usage
        .last_snapshot_total_tokens
        .unwrap_or(usage.total_tokens);
    usage.total_tokens > last_snapshot_total
}

fn cached_rate_limit_snapshot_from_usage(
    usage: &AccountUsageSnapshot,
) -> Option<RateLimitSnapshot> {
    let used_percent = usage.last_backend_used_percent?;
    let window = codex_protocol::protocol::RateLimitWindow {
        used_percent,
        window_minutes: usage.last_backend_window_minutes,
        resets_at: usage.last_backend_resets_at,
    };
    Some(RateLimitSnapshot {
        limit_id: usage
            .last_backend_limit_id
            .clone()
            .or(Some("codex".to_string())),
        limit_name: usage
            .last_backend_limit_name
            .clone()
            .or(Some("codex".to_string())),
        primary: None,
        secondary: Some(window),
        credits: None,
        plan_type: None,
    })
}

async fn fetch_rate_limits_for_cli(base_url: String, auth: CodexAuth) -> CliRateLimitFetchOutcome {
    let client = match BackendClient::from_auth(base_url, &auth) {
        Ok(client) => client,
        Err(err) => return CliRateLimitFetchOutcome::Other(err.to_string()),
    };
    let result = timeout(
        Duration::from_secs(CLI_RATE_LIMIT_FETCH_TIMEOUT_SECS),
        client.get_rate_limits_many(),
    )
    .await;
    match result {
        Ok(Ok(snapshots)) => CliRateLimitFetchOutcome::Success(snapshots),
        Ok(Err(err)) => CliRateLimitFetchOutcome::BackendUnavailable(err.to_string()),
        Err(_) => CliRateLimitFetchOutcome::Timeout,
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
