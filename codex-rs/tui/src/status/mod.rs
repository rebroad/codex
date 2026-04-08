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

use crate::history_cell::HistoryCell;
use crate::insert_history::write_spans;
pub(crate) use account::StatusAccountDisplay;
pub(crate) use card::StatusCardVariant;
use account::truncate_status_email_local_part;
#[cfg(test)]
pub(crate) use card::new_status_output;
pub(crate) use card::new_status_output_with_rate_limits;
use chrono::Local;
use chrono::TimeZone;
use chrono::Utc;
use codex_backend_client::Client as BackendClient;
use codex_core::CodexAuth;
use codex_core::WireApi;
use codex_core::config::Config;
use codex_login::token_data::parse_jwt_expiration;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CreditsSnapshot;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;
use codex_state::AccountUsageSnapshot;
use codex_state::AccountUsageEstimatorConfig;
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

const CLI_RATE_LIMIT_FETCH_TIMEOUT_SECS: u64 = 8;
const BACKEND_PERCENT_EPSILON: f64 = 0.0001;
const COMPOSITE_Q_INPUT_WEIGHT: f64 = 0.006;
const COMPOSITE_Q_CACHED_INPUT_WEIGHT: f64 = 0.003;

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
    let account_usage_store = AccountUsageStore::init_with_estimator_config(
        config.sqlite_home.clone(),
        config.model_provider_id.clone(),
        AccountUsageEstimatorConfig {
            min_usage_pct_sample_count: config.account_usage_estimator.min_usage_pct_sample_count,
            max_usage_pct_display_percent_before_full: config
                .account_usage_estimator
                .max_usage_pct_display_percent_before_full,
            stable_backend_percent_window: config
                .account_usage_estimator
                .stable_backend_percent_window,
        },
    )
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
    let should_refresh_rate_limits = should_refresh_cli_rate_limits(config, usage_snapshot.as_ref());

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
    let account_display = status_account_display_for_cli(
        auth.as_ref(),
        plan_type.clone(),
        cli_status_email_prefix_emoji(auth.as_ref(), &fetch_outcome).map(str::to_string),
    );
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

    let reasoning_effort_override =
        (config.model_provider.wire_api == WireApi::Responses).then(|| {
            config
                .model_reasoning_effort
                .or_else(|| default_reasoning_effort_from_catalog(config, model_name))
        });

    let total_usage = TokenUsage::default();
    let account_usage = fetch_account_usage_display(
        account_usage_store.as_deref(),
        auth.as_ref(),
        config.account_usage_estimator.stable_backend_percent_window,
    )
    .await;
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
        let sign = if offset_seconds.is_negative() {
            '-'
        } else {
            '+'
        };
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
    let Some(account_id) = account_usage_key(
        auth.get_account_id().as_deref(),
        auth.get_account_email().as_deref(),
    ) else {
        return CompactStatusUsage {
            percent_left: 0,
            reset_at_unix: None,
        };
    };

    let account_usage_store = AccountUsageStore::init_with_estimator_config(
        config.sqlite_home.clone(),
        config.model_provider_id.clone(),
        AccountUsageEstimatorConfig {
            min_usage_pct_sample_count: config.account_usage_estimator.min_usage_pct_sample_count,
            max_usage_pct_display_percent_before_full: config
                .account_usage_estimator
                .max_usage_pct_display_percent_before_full,
            stable_backend_percent_window: config
                .account_usage_estimator
                .stable_backend_percent_window,
        },
    )
    .await
    .ok();
    let Some(account_usage_store) = account_usage_store else {
        return CompactStatusUsage {
            percent_left: 0,
            reset_at_unix: None,
        };
    };
    if let Some(account_display) = account_usage_display(auth.get_account_email().as_deref()) {
        account_usage_store
            .cache_account_display(account_id.as_str(), account_display)
            .await;
    }
    let usage = account_usage_store
        .get_account_usage(account_id.as_str())
        .await
        .ok()
        .flatten();
    CompactStatusUsage {
        percent_left: usage
            .as_ref()
            .and_then(|u| u.last_backend_used_percent)
            .map(|used_percent| (100.0 - used_percent).round() as i64)
            .map(|percent_left| percent_left.clamp(0, 100))
            .unwrap_or(0),
        reset_at_unix: usage.and_then(|u| u.last_backend_resets_at),
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

fn cli_status_email_prefix_emoji(
    auth: Option<&CodexAuth>,
    fetch_outcome: &CliRateLimitFetchOutcome,
) -> Option<&'static str> {
    let auth = auth?;
    if auth.is_api_key_auth() {
        return None;
    }

    match classify_cli_auth_health(auth, fetch_outcome) {
        CliAuthHealth::Healthy => Some("✅"),
        CliAuthHealth::AccessTokenExpired => Some("⏰"),
        CliAuthHealth::RefreshTokenReused => Some("🔁"),
        CliAuthHealth::UnauthorizedUnknown => Some("🚫"),
        CliAuthHealth::UnknownLocalTokenState => Some("❔"),
    }
}

enum CliAuthHealth {
    Healthy,
    AccessTokenExpired,
    RefreshTokenReused,
    UnauthorizedUnknown,
    UnknownLocalTokenState,
}

fn classify_cli_auth_health(
    auth: &CodexAuth,
    fetch_outcome: &CliRateLimitFetchOutcome,
) -> CliAuthHealth {
    if let Some(result) = classify_from_fetch_outcome(fetch_outcome) {
        return result;
    }

    match auth_access_token_state(auth) {
        AuthAccessTokenState::Expired => CliAuthHealth::AccessTokenExpired,
        AuthAccessTokenState::Unknown => CliAuthHealth::UnknownLocalTokenState,
        AuthAccessTokenState::Healthy => CliAuthHealth::Healthy,
    }
}

fn classify_from_fetch_outcome(fetch_outcome: &CliRateLimitFetchOutcome) -> Option<CliAuthHealth> {
    let err = match fetch_outcome {
        CliRateLimitFetchOutcome::BackendUnavailable(err)
        | CliRateLimitFetchOutcome::Other(err) => err,
        _ => return None,
    };
    let err = err.to_ascii_lowercase();
    if err.contains("refresh token was already used") {
        return Some(CliAuthHealth::RefreshTokenReused);
    }
    if err.contains("token_expired") {
        return Some(CliAuthHealth::AccessTokenExpired);
    }
    if err.contains("unauthorized") {
        return Some(CliAuthHealth::UnauthorizedUnknown);
    }
    None
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

fn should_refresh_cli_rate_limits(config: &Config, usage: Option<&AccountUsageSnapshot>) -> bool {
    let Some(usage) = usage else {
        return true;
    };
    !is_backend_percent_stable_for_cli(usage, config.account_usage_estimator.stable_backend_percent_window)
}

fn is_backend_percent_stable_for_cli(
    usage: &AccountUsageSnapshot,
    stable_distinct_window: i64,
) -> bool {
    let window_size = stable_distinct_window.max(1) as usize;
    let mut values = usage
        .backend_percent_history
        .as_deref()
        .map(parse_backend_percent_history)
        .unwrap_or_default();
    let Some(current_backend_percent) = usage.last_backend_used_percent else {
        return false;
    };
    if !current_backend_percent.is_finite() || current_backend_percent < 0.0 {
        return false;
    }
    if values
        .last()
        .is_none_or(|last| (last - current_backend_percent).abs() > BACKEND_PERCENT_EPSILON)
    {
        values.push(current_backend_percent);
    }
    let recent = take_last_distinct_backend_percents(&values, window_size);
    recent.len() == window_size
        && recent
            .windows(2)
            .all(|pair| pair[1] > pair[0] + BACKEND_PERCENT_EPSILON)
}

fn parse_backend_percent_history(raw: &str) -> Vec<f64> {
    raw.split(',')
        .filter_map(|item| item.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect()
}

fn take_last_distinct_backend_percents(values: &[f64], window_size: usize) -> Vec<f64> {
    if window_size == 0 {
        return Vec::new();
    }
    let mut distinct_reversed = Vec::with_capacity(window_size);
    for value in values.iter().rev() {
        if distinct_reversed
            .last()
            .is_none_or(|last: &f64| (*last - *value).abs() > BACKEND_PERCENT_EPSILON)
        {
            distinct_reversed.push(*value);
            if distinct_reversed.len() >= window_size {
                break;
            }
        }
    }
    distinct_reversed.reverse();
    distinct_reversed
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
    stable_distinct_window: i64,
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
        .estimate_account_limit_tokens_q_cached(account_id.as_str(), &usage)
        .await
        .ok()
        .unwrap_or((None, 0));
    Some(build_account_usage_display(
        &usage,
        estimated_limit,
        is_backend_percent_stable_for_cli(&usage, stable_distinct_window),
    ))
}

fn build_account_usage_display(
    usage: &AccountUsageSnapshot,
    estimated_limit: (Option<f64>, i64),
    backend_percent_stable: bool,
) -> AccountUsageDisplay {
    let (limit, sample_count) = estimated_limit;
    let estimated_percent = if backend_percent_stable {
        estimate_account_usage_percent(usage, limit)
    } else {
        None
    };
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
    let window_start_q_tokens = composite_q_tokens(
        usage.window_start_input_tokens.unwrap_or(0),
        usage.window_start_cached_input_tokens.unwrap_or(0),
        usage.window_start_output_tokens.unwrap_or(0),
    );
    let q_tokens = composite_q_tokens(
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
    );
    let delta_tokens = (q_tokens - window_start_q_tokens).max(0.0);
    let avg_tokens_per_pct = estimated_limit / 100.0;
    if avg_tokens_per_pct <= 0.0 || !avg_tokens_per_pct.is_finite() {
        return None;
    }
    let percent = delta_tokens / avg_tokens_per_pct;
    Some(base_percent + percent)
}

fn composite_q_tokens(input_tokens: i64, cached_input_tokens: i64, output_tokens: i64) -> f64 {
    output_tokens.max(0) as f64
        + COMPOSITE_Q_INPUT_WEIGHT * input_tokens.max(0) as f64
        + COMPOSITE_Q_CACHED_INPUT_WEIGHT * cached_input_tokens.max(0) as f64
}

fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
    let mut out = Vec::new();
    let _ = write_spans(&mut out, line.spans.iter());
    String::from_utf8_lossy(&out).into_owned()
}
