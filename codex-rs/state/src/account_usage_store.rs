use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::SecondsFormat;
use chrono::TimeZone;
use chrono::Utc;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::TokenUsage;
use codex_utils_home_dir::find_codex_home;
use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use tokio::sync::Mutex;

pub(crate) static USAGE_MIGRATOR: Migrator = sqlx::migrate!("./usage_migrations");

const USAGE_DB_FILENAME: &str = "usage";
const USAGE_DB_VERSION: u32 = 1;
const USED_PERCENT_REFUND_EPSILON: f64 = 0.0001;
const SUSPICIOUS_PERCENT_JUMP_THRESHOLD: f64 = 2.0;
const BACKEND_CHANGE_CONFIRMATIONS_REQUIRED: u8 = 2;
const BACKEND_CHANGE_PENDING_TTL_SECS: i64 = 120;
const PLAUSIBLE_JUMP_MIN_MATCH_RATIO: f64 = 0.75;
const PLAUSIBLE_JUMP_ABS_SLACK_PERCENT: f64 = 0.5;
const USAGE_LOG_DIRNAME: &str = "log";
const USAGE_LOG_FILENAME_PREFIX: &str = "usage-";
const USAGE_LOG_FILENAME_SUFFIX: &str = ".log";

pub fn account_usage_key(account_id: Option<&str>, account_email: Option<&str>) -> Option<String> {
    account_id
        .map(str::to_owned)
        .or_else(|| account_email.map(|email| format!("email:{email}")))
}

pub fn account_usage_display(account_email: Option<&str>) -> Option<String> {
    account_email.map(str::to_owned)
}

#[derive(Debug, Clone)]
pub struct AccountUsageSnapshot {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub updated_at: i64,
    pub last_backend_limit_id: Option<String>,
    pub last_backend_limit_name: Option<String>,
    pub last_backend_used_percent: Option<f64>,
    pub last_snapshot_total_tokens: Option<i64>,
    pub last_snapshot_percent_int: Option<i64>,
    pub window_start_percent_int: Option<i64>,
    pub window_start_total_tokens: Option<i64>,
    pub window_start_input_tokens: Option<i64>,
    pub window_start_cached_input_tokens: Option<i64>,
    pub window_start_output_tokens: Option<i64>,
    pub last_backend_resets_at: Option<i64>,
    pub last_backend_window_minutes: Option<i64>,
    pub last_backend_seen_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AccountUsageEventMeta<'a> {
    pub query_id: Option<&'a str>,
    pub sent_bytes: Option<i64>,
    pub recv_bytes: Option<i64>,
}

#[derive(Clone)]
pub struct AccountUsageStore {
    sqlite_home: PathBuf,
    default_provider: String,
    pool: Arc<SqlitePool>,
    pending_backend_updates:
        Arc<Mutex<std::collections::HashMap<(String, String), PendingBackendRateLimitUpdate>>>,
    account_displays: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

#[derive(Debug, Clone)]
struct PendingBackendRateLimitUpdate {
    used_percent: f64,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
    confirmations: u8,
    last_seen_at: i64,
}

#[derive(Debug, Clone)]
struct AccountLimitEstimates {
    composite_q_limit: Option<f64>,
    blended_limit: Option<f64>,
    cached_input_limit: Option<f64>,
    output_limit: Option<f64>,
    context_total_limit: Option<f64>,
    min_total_cached_output_limit: Option<f64>,
    min_input_cached_output_limit: Option<f64>,
    sent_limit: Option<f64>,
    recv_limit: Option<f64>,
    sent_recv_limit: Option<f64>,
    sample_count: i64,
}

#[derive(Debug, Clone)]
struct SampleTokenDeltas {
    blended_tokens: i64,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    context_total_tokens: i64,
    min_total_cached_output_tokens: i64,
    sent_bytes: i64,
    recv_bytes: i64,
    sent_recv_bytes: i64,
}

// Composite usage model calibrated from local/backend usage logs.
const COMPOSITE_Q_INPUT_WEIGHT: f64 = 0.006;
const COMPOSITE_Q_CACHED_INPUT_WEIGHT: f64 = 0.003;

impl AccountUsageStore {
    pub async fn init(sqlite_home: PathBuf, default_provider: String) -> anyhow::Result<Arc<Self>> {
        tokio::fs::create_dir_all(&sqlite_home).await?;
        let usage_path = usage_db_path(sqlite_home.as_path());
        let pool = open_sqlite(&usage_path).await?;
        Ok(Arc::new(Self {
            sqlite_home,
            default_provider,
            pool: Arc::new(pool),
            pending_backend_updates: Arc::new(Mutex::new(std::collections::HashMap::new())),
            account_displays: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }))
    }

    pub fn sqlite_home(&self) -> &Path {
        self.sqlite_home.as_path()
    }

    pub async fn clear_usage_for_account(&self, account_id: &str) -> anyhow::Result<(u64, u64)> {
        let mut tx = self.pool.begin().await?;
        let sample_rows_deleted = sqlx::query(
            r#"
DELETE FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let usage_rows_deleted = sqlx::query(
            r#"
DELETE FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;

        {
            let mut account_displays = self.account_displays.lock().await;
            account_displays.remove(account_id);
        }

        Ok((usage_rows_deleted, sample_rows_deleted))
    }

    pub async fn clear_usage_for_all_accounts(&self) -> anyhow::Result<(u64, u64)> {
        let mut tx = self.pool.begin().await?;
        let sample_rows_deleted = sqlx::query(
            r#"
DELETE FROM account_usage_samples
WHERE provider = ?
            "#,
        )
        .bind(self.default_provider.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let usage_rows_deleted = sqlx::query(
            r#"
DELETE FROM account_usage
WHERE provider = ?
            "#,
        )
        .bind(self.default_provider.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;

        {
            let mut account_displays = self.account_displays.lock().await;
            account_displays.clear();
        }

        Ok((usage_rows_deleted, sample_rows_deleted))
    }

    pub async fn get_account_usage(
        &self,
        account_id: &str,
    ) -> anyhow::Result<Option<AccountUsageSnapshot>> {
        let row = sqlx::query(
            r#"
SELECT
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    reasoning_output_tokens,
    updated_at,
    last_backend_limit_id,
    last_backend_limit_name,
    last_backend_used_percent,
    last_snapshot_total_tokens,
    last_snapshot_percent_int,
    window_start_percent_int,
    window_start_total_tokens,
    window_start_input_tokens,
    window_start_cached_input_tokens,
    window_start_output_tokens,
    window_start_context_total_tokens,
    window_start_min_total_cached_output_tokens,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .fetch_optional(self.pool.as_ref())
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(AccountUsageSnapshot {
            total_tokens: row.try_get("total_tokens")?,
            input_tokens: row.try_get("input_tokens")?,
            cached_input_tokens: row.try_get("cached_input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            reasoning_output_tokens: row.try_get("reasoning_output_tokens")?,
            updated_at: row.try_get("updated_at")?,
            last_backend_limit_id: row.try_get("last_backend_limit_id")?,
            last_backend_limit_name: row.try_get("last_backend_limit_name")?,
            last_backend_used_percent: row.try_get("last_backend_used_percent")?,
            last_snapshot_total_tokens: row.try_get("last_snapshot_total_tokens")?,
            last_snapshot_percent_int: row.try_get("last_snapshot_percent_int")?,
            window_start_percent_int: row.try_get("window_start_percent_int")?,
            window_start_total_tokens: row.try_get("window_start_total_tokens")?,
            window_start_input_tokens: row.try_get("window_start_input_tokens")?,
            window_start_cached_input_tokens: row.try_get("window_start_cached_input_tokens")?,
            window_start_output_tokens: row.try_get("window_start_output_tokens")?,
            last_backend_resets_at: row.try_get("last_backend_resets_at")?,
            last_backend_window_minutes: row.try_get("last_backend_window_minutes")?,
            last_backend_seen_at: row.try_get("last_backend_seen_at")?,
        }))
    }

    pub async fn estimate_account_limit_tokens(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(Option<f64>, i64)> {
        let estimates = self.estimate_account_limit_tokens_multi(account_id).await?;
        Ok((estimates.blended_limit, estimates.sample_count))
    }

    pub async fn estimate_account_limit_tokens_q(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(Option<f64>, i64)> {
        let estimates = self.estimate_account_limit_tokens_multi(account_id).await?;
        Ok((estimates.composite_q_limit, estimates.sample_count))
    }

    async fn estimate_account_limit_tokens_multi(
        &self,
        account_id: &str,
    ) -> anyhow::Result<AccountLimitEstimates> {
        let query = String::from(
            r#"
SELECT
    SUM(delta_tokens) AS blended_tokens,
    SUM(delta_input_tokens) AS input_tokens,
    SUM(delta_cached_input_tokens) AS cached_input_tokens,
    SUM(delta_output_tokens) AS output_tokens,
    SUM(delta_context_total_tokens) AS context_total_tokens,
    SUM(delta_min_total_cached_output_tokens) AS min_total_cached_output_tokens,
    SUM(delta_output_tokens + MIN(delta_input_tokens, delta_cached_input_tokens)) AS min_input_cached_output_tokens,
    SUM(delta_sent_bytes) AS sent_bytes,
    SUM(delta_recv_bytes) AS recv_bytes,
    SUM(delta_sent_recv_bytes) AS sent_recv_bytes,
    SUM(delta_percent_int) AS total_percent,
    COUNT(*) AS sample_count
FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        );
        let row = sqlx::query(query.as_str())
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .fetch_one(self.pool.as_ref())
            .await?;

        let blended_tokens = row
            .try_get::<Option<i64>, _>("blended_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let input_tokens = row
            .try_get::<Option<i64>, _>("input_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let cached_input_tokens = row
            .try_get::<Option<i64>, _>("cached_input_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let output_tokens = row
            .try_get::<Option<i64>, _>("output_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let context_total_tokens = row
            .try_get::<Option<i64>, _>("context_total_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let min_total_cached_output_tokens = row
            .try_get::<Option<i64>, _>("min_total_cached_output_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let min_input_cached_output_tokens = row
            .try_get::<Option<i64>, _>("min_input_cached_output_tokens")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let sent_bytes = row
            .try_get::<Option<i64>, _>("sent_bytes")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let recv_bytes = row
            .try_get::<Option<i64>, _>("recv_bytes")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let sent_recv_bytes = row
            .try_get::<Option<i64>, _>("sent_recv_bytes")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let total_percent: i64 = row
            .try_get::<Option<i64>, _>("total_percent")
            .unwrap_or(Some(0))
            .unwrap_or(0);
        let sample_count: i64 = row
            .try_get::<Option<i64>, _>("sample_count")
            .unwrap_or(Some(0))
            .unwrap_or(0);

        if total_percent <= 0 || sample_count <= 0 {
            return Ok(AccountLimitEstimates {
                composite_q_limit: None,
                blended_limit: None,
                cached_input_limit: None,
                output_limit: None,
                context_total_limit: None,
                min_total_cached_output_limit: None,
                min_input_cached_output_limit: None,
                sent_limit: None,
                recv_limit: None,
                sent_recv_limit: None,
                sample_count,
            });
        }

        let percent = total_percent as f64 / 100.0;
        let estimate = |tokens: f64| {
            if tokens <= 0.0 || !tokens.is_finite() {
                return None;
            }
            let estimated_limit = tokens / percent;
            if !estimated_limit.is_finite() || estimated_limit <= 0.0 {
                None
            } else {
                Some(estimated_limit)
            }
        };

        Ok(AccountLimitEstimates {
            composite_q_limit: estimate(composite_q_tokens(
                input_tokens,
                cached_input_tokens,
                output_tokens,
            )),
            blended_limit: estimate(blended_tokens as f64),
            cached_input_limit: estimate(cached_input_tokens as f64),
            output_limit: estimate(output_tokens as f64),
            context_total_limit: estimate(context_total_tokens as f64),
            min_total_cached_output_limit: estimate(min_total_cached_output_tokens as f64),
            min_input_cached_output_limit: estimate(min_input_cached_output_tokens as f64),
            sent_limit: estimate(sent_bytes as f64),
            recv_limit: estimate(recv_bytes as f64),
            sent_recv_limit: estimate(sent_recv_bytes as f64),
            sample_count,
        })
    }

    pub async fn record_account_token_usage(
        &self,
        account_id: &str,
        usage: &TokenUsage,
        meta: AccountUsageEventMeta<'_>,
    ) -> anyhow::Result<()> {
        let normalized_usage = normalize_usage_for_accounting(usage);
        let context_total_tokens = usage.total_tokens.max(0);
        let total_tokens = normalized_usage.total_tokens.max(0);
        let input_tokens = normalized_usage.input_tokens.max(0);
        let cached_input_tokens = normalized_usage.cached_input_tokens.max(0);
        let output_tokens = normalized_usage.output_tokens.max(0);
        let reasoning_output_tokens = normalized_usage.reasoning_output_tokens.max(0);
        let min_total_cached_output_tokens = total_tokens.min(cached_input_tokens + output_tokens);
        let sent = meta.sent_bytes.unwrap_or(0).max(0);
        let recv = meta.recv_bytes.unwrap_or(0).max(0);
        let sent_recv = sent.saturating_add(recv);
        if total_tokens == 0
            && input_tokens == 0
            && cached_input_tokens == 0
            && output_tokens == 0
            && reasoning_output_tokens == 0
        {
            return Ok(());
        }

        let updated_at = Utc::now().timestamp();
        sqlx::query(
            r#"
INSERT INTO account_usage (
    account_id,
    provider,
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    reasoning_output_tokens,
    context_total_tokens,
    min_total_cached_output_tokens,
    sent_bytes,
    recv_bytes,
    sent_recv_bytes,
    updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(account_id, provider) DO UPDATE SET
    total_tokens = total_tokens + excluded.total_tokens,
    input_tokens = input_tokens + excluded.input_tokens,
    cached_input_tokens = cached_input_tokens + excluded.cached_input_tokens,
    output_tokens = output_tokens + excluded.output_tokens,
    reasoning_output_tokens = reasoning_output_tokens + excluded.reasoning_output_tokens,
    context_total_tokens = context_total_tokens + excluded.context_total_tokens,
    min_total_cached_output_tokens = min_total_cached_output_tokens + excluded.min_total_cached_output_tokens,
    sent_bytes = sent_bytes + excluded.sent_bytes,
    recv_bytes = recv_bytes + excluded.recv_bytes,
    sent_recv_bytes = sent_recv_bytes + excluded.sent_recv_bytes,
    updated_at = excluded.updated_at
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .bind(total_tokens)
        .bind(input_tokens)
        .bind(cached_input_tokens)
        .bind(output_tokens)
        .bind(reasoning_output_tokens)
        .bind(context_total_tokens)
        .bind(min_total_cached_output_tokens)
        .bind(sent)
        .bind(recv)
        .bind(sent_recv)
        .bind(updated_at)
        .execute(self.pool.as_ref())
        .await?;

        let query_id_suffix = meta
            .query_id
            .map(|value| format!(" query_id={value}"))
            .unwrap_or_default();
        self.log_usage_event(
            account_id,
            /*used_percent*/ None,
            /*previous_percent*/ None,
            format!(
                "total={total_tokens}, input={input_tokens}, cached_input={cached_input_tokens}, output={output_tokens}, reasoning={reasoning_output_tokens}, context_total={context_total_tokens}, sent={sent}, recv={recv}, sent_recv={sent_recv}{query_id_suffix}"
            ),
        )
        .await;

        Ok(())
    }

    pub async fn cache_account_display(&self, account_id: &str, display: String) {
        let mut displays = self.account_displays.lock().await;
        displays.insert(account_id.to_string(), display);
    }

    pub async fn record_account_backend_rate_limit(
        &self,
        account_id: &str,
        snapshot: &RateLimitSnapshot,
    ) -> anyhow::Result<()> {
        if snapshot.limit_id.as_deref() != Some("codex") {
            return Ok(());
        }

        let window = snapshot.secondary.as_ref().or(snapshot.primary.as_ref());
        let Some(window) = window else {
            return Ok(());
        };

        let used_percent = window.used_percent;
        let window_minutes = window.window_minutes.map(|minutes| minutes.max(0));
        let resets_at = window.resets_at.map(|ts| ts.max(0));
        let seen_at = Some(Utc::now().timestamp());
        let now = Utc::now().timestamp();

        let prior_usage = sqlx::query(
            r#"
SELECT
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    reasoning_output_tokens,
    context_total_tokens,
    min_total_cached_output_tokens,
    sent_bytes,
    recv_bytes,
    sent_recv_bytes,
    last_backend_used_percent,
    last_snapshot_total_tokens,
    last_snapshot_input_tokens,
    last_snapshot_cached_input_tokens,
    last_snapshot_output_tokens,
    last_snapshot_context_total_tokens,
    last_snapshot_min_total_cached_output_tokens,
    last_snapshot_sent_bytes,
    last_snapshot_recv_bytes,
    last_snapshot_sent_recv_bytes,
    last_snapshot_percent_int,
    window_start_percent_int,
    window_start_total_tokens,
    window_start_input_tokens,
    window_start_cached_input_tokens,
    window_start_output_tokens,
    window_start_context_total_tokens,
    window_start_min_total_cached_output_tokens,
    window_start_sent_bytes,
    window_start_recv_bytes,
    window_start_sent_recv_bytes,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .fetch_optional(self.pool.as_ref())
        .await?;

        let previous_backend_percent = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<f64, _>("last_backend_used_percent").ok());
        let previous_snapshot_percent = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_percent_int").ok())
            .map(|percent| percent as f64);
        let previous_percent_for_jump = previous_snapshot_percent.or(previous_backend_percent);
        let (window_changed, reset_time_changed) = prior_usage
            .as_ref()
            .map(|row| {
                let previous_resets_at: Option<i64> = row.try_get("last_backend_resets_at").ok();
                let previous_window: Option<i64> = row.try_get("last_backend_window_minutes").ok();
                let window_changed = previous_window.is_some()
                    && previous_window != window_minutes
                    && window_minutes.is_some();
                let reset_time_changed = match (previous_resets_at, resets_at) {
                    (Some(previous), Some(current)) => (previous - current).abs() > 3,
                    _ => false,
                };
                (window_changed, reset_time_changed)
            })
            .unwrap_or((false, false));

        if window_changed || reset_time_changed {
            let previous_window = prior_usage
                .as_ref()
                .and_then(|row| row.try_get::<i64, _>("last_backend_window_minutes").ok());
            let previous_resets_at = prior_usage
                .as_ref()
                .and_then(|row| row.try_get::<i64, _>("last_backend_resets_at").ok());
            let format_epoch = |value: Option<i64>| {
                if let Some(ts) = value {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
                        .unwrap_or_else(|| ts.to_string())
                } else {
                    "none".to_string()
                }
            };
            let window_log = if window_changed {
                format!(
                    "window_minutes={}->{}",
                    previous_window
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    window_minutes
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string())
                )
            } else {
                String::new()
            };
            let reset_log = if reset_time_changed {
                format!(
                    "resets_at={}->{}",
                    format_epoch(previous_resets_at),
                    format_epoch(resets_at)
                )
            } else {
                String::new()
            };
            let sep = if !window_log.is_empty() && !reset_log.is_empty() {
                " "
            } else {
                ""
            };
            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                format!("{window_log}{sep}{reset_log}"),
            )
            .await;
        }

        let total_tokens_now_for_jump = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("total_tokens").ok())
            .unwrap_or(0);
        let last_snapshot_total_tokens_for_jump = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_total_tokens").ok())
            .unwrap_or(total_tokens_now_for_jump);
        let delta_tokens_since_snapshot =
            (total_tokens_now_for_jump - last_snapshot_total_tokens_for_jump).max(0);
        let delta_percent = previous_percent_for_jump.map(|previous| used_percent - previous);
        let large_jump =
            delta_percent.is_some_and(|delta| delta.abs() > SUSPICIOUS_PERCENT_JUMP_THRESHOLD);
        let negative_jump = delta_percent.is_some_and(|delta| delta < -USED_PERCENT_REFUND_EPSILON);
        let large_positive_jump_plausible =
            if let Some(delta) = delta_percent.filter(|delta| *delta > 0.0) {
                if !large_jump {
                    true
                } else {
                    let estimates = self
                        .estimate_account_limit_tokens_multi(account_id)
                        .await
                        .unwrap_or(AccountLimitEstimates {
                            composite_q_limit: None,
                            blended_limit: None,
                            cached_input_limit: None,
                            output_limit: None,
                            context_total_limit: None,
                            min_total_cached_output_limit: None,
                            min_input_cached_output_limit: None,
                            sent_limit: None,
                            recv_limit: None,
                            sent_recv_limit: None,
                            sample_count: 0,
                        });
                    let expected_from_growth = estimates.blended_limit.map(|limit| {
                        if limit.is_finite() && limit > 0.0 {
                            delta_tokens_since_snapshot as f64 * 100.0 / limit
                        } else {
                            0.0
                        }
                    });
                    expected_from_growth.is_some_and(|expected| {
                        expected + PLAUSIBLE_JUMP_ABS_SLACK_PERCENT
                            >= delta * PLAUSIBLE_JUMP_MIN_MATCH_RATIO
                    })
                }
            } else {
                false
            };
        let suspicious_change = prior_usage.is_some()
            && (window_changed
                || reset_time_changed
                || negative_jump
                || (large_jump && !large_positive_jump_plausible));
        if suspicious_change {
            let backend_candidate_confirmed_from_db = prior_usage.as_ref().is_some_and(|row| {
                let previous_seen_at: Option<i64> = row.try_get("last_backend_seen_at").ok();
                let previous_resets_at: Option<i64> = row.try_get("last_backend_resets_at").ok();
                let previous_window: Option<i64> = row.try_get("last_backend_window_minutes").ok();
                previous_seen_at
                    .zip(seen_at)
                    .is_some_and(|(previous, current)| {
                        current.saturating_sub(previous) <= BACKEND_CHANGE_PENDING_TTL_SECS
                    })
                    && previous_window == window_minutes
                    && previous_resets_at == resets_at
                    && previous_backend_percent.is_some_and(|previous| {
                        (previous - used_percent).abs() <= USED_PERCENT_REFUND_EPSILON
                    })
            });
            let key = (account_id.to_string(), self.default_provider.clone());
            let seen_ts = seen_at.unwrap_or(now);
            let mut pending_updates = self.pending_backend_updates.lock().await;
            let mut should_remove_pending = false;
            let confirmation_state = if backend_candidate_confirmed_from_db {
                should_remove_pending = true;
                (true, BACKEND_CHANGE_CONFIRMATIONS_REQUIRED)
            } else if let Some(pending) = pending_updates.get_mut(&key) {
                let same_candidate = seen_ts.saturating_sub(pending.last_seen_at)
                    <= BACKEND_CHANGE_PENDING_TTL_SECS
                    && (pending.used_percent - used_percent).abs() <= USED_PERCENT_REFUND_EPSILON
                    && pending.window_minutes == window_minutes
                    && pending.resets_at == resets_at;
                if same_candidate {
                    pending.confirmations = pending.confirmations.saturating_add(1);
                    pending.last_seen_at = seen_ts;
                    if pending.confirmations >= BACKEND_CHANGE_CONFIRMATIONS_REQUIRED {
                        should_remove_pending = true;
                        (true, BACKEND_CHANGE_CONFIRMATIONS_REQUIRED)
                    } else {
                        (false, pending.confirmations)
                    }
                } else {
                    pending_updates.insert(
                        key.clone(),
                        PendingBackendRateLimitUpdate {
                            used_percent,
                            window_minutes,
                            resets_at,
                            confirmations: 1,
                            last_seen_at: seen_ts,
                        },
                    );
                    (false, 1)
                }
            } else {
                pending_updates.insert(
                    key.clone(),
                    PendingBackendRateLimitUpdate {
                        used_percent,
                        window_minutes,
                        resets_at,
                        confirmations: 1,
                        last_seen_at: seen_ts,
                    },
                );
                (false, 1)
            };
            if should_remove_pending {
                pending_updates.remove(&key);
            }
            drop(pending_updates);

            let (confirmed, confirmations) = confirmation_state;
            if !confirmed {
                sqlx::query(
                    r#"
UPDATE account_usage
SET
    updated_at = ?,
    last_backend_used_percent = ?,
    last_backend_resets_at = ?,
    last_backend_window_minutes = ?,
    last_backend_seen_at = ?
WHERE account_id = ? AND provider = ?
                    "#,
                )
                .bind(now)
                .bind(used_percent)
                .bind(resets_at)
                .bind(window_minutes)
                .bind(seen_at)
                .bind(account_id)
                .bind(self.default_provider.as_str())
                .execute(self.pool.as_ref())
                .await?;
                self.log_usage_event(
                    account_id,
                    Some(used_percent),
                    previous_backend_percent,
                    format!(
                        "backend_change_pending=1 confirmations={confirmations} delta_percent={}",
                        delta_percent.unwrap_or(0.0)
                    ),
                )
                .await;
                return Ok(());
            }
            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                format!(
                    "backend_change_confirmed=1 confirmations={} delta_percent={}",
                    BACKEND_CHANGE_CONFIRMATIONS_REQUIRED,
                    delta_percent.unwrap_or(0.0)
                ),
            )
            .await;
        } else {
            let mut pending_updates = self.pending_backend_updates.lock().await;
            pending_updates.remove(&(account_id.to_string(), self.default_provider.clone()));
        }

        let should_reset = prior_usage.as_ref().is_some_and(|row| {
            let previous_percent: Option<f64> = row.try_get("last_backend_used_percent").ok();
            let previous_seen_at: Option<i64> = row.try_get("last_backend_seen_at").ok();
            let was_positive = previous_percent.unwrap_or(0.0) > 0.0;
            let now_zero = used_percent <= 0.0;

            let new_snapshot = previous_seen_at
                .zip(seen_at)
                .map(|(previous, current)| current >= previous)
                .unwrap_or(true);

            (new_snapshot || reset_time_changed) && was_positive && now_zero
        });

        let current_percent_int = used_percent.floor().max(0.0) as i64;
        if negative_jump {
            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                format!(
                    "refund_rewind_disabled=1 delta_percent={}",
                    delta_percent.unwrap_or(0.0)
                ),
            )
            .await;
        }
        let (
            total_tokens_now,
            input_tokens_now,
            cached_input_tokens_now,
            output_tokens_now,
            context_total_tokens_now,
            min_total_cached_output_tokens_now,
            sent_bytes_now,
            recv_bytes_now,
            sent_recv_bytes_now,
            last_sample_tokens,
            last_sample_input_tokens,
            last_sample_cached_input_tokens,
            last_sample_output_tokens,
            last_sample_context_total_tokens,
            last_sample_min_total_cached_output_tokens,
            last_sample_sent_bytes,
            last_sample_recv_bytes,
            last_sample_sent_recv_bytes,
            last_sample_percent,
            window_start_percent,
            window_start_tokens,
            window_start_input_tokens,
            window_start_cached_input_tokens,
            window_start_output_tokens,
            window_start_context_total_tokens,
            window_start_min_total_cached_output_tokens,
            window_start_sent_bytes,
            window_start_recv_bytes,
            window_start_sent_recv_bytes,
        ) = if let Some(row) = prior_usage.as_ref() {
            (
                row.try_get("total_tokens")?,
                row.try_get("input_tokens")?,
                row.try_get("cached_input_tokens")?,
                row.try_get("output_tokens")?,
                row.try_get("context_total_tokens")?,
                row.try_get("min_total_cached_output_tokens")?,
                row.try_get::<i64, _>("sent_bytes").unwrap_or(0),
                row.try_get::<i64, _>("recv_bytes").unwrap_or(0),
                row.try_get::<i64, _>("sent_recv_bytes").unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_total_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_input_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_cached_input_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_output_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_context_total_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_min_total_cached_output_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_sent_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_recv_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_sent_recv_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_percent_int")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_percent_int")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_total_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_input_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_cached_input_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_output_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_context_total_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_min_total_cached_output_tokens")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_sent_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_recv_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_sent_recv_bytes")
                    .unwrap_or(0),
            )
        } else {
            (
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
            )
        };

        if prior_usage.is_some() && should_reset {
            let reached_full_window = last_sample_percent >= 100;
            let cleared_samples = if reached_full_window {
                sqlx::query(
                    r#"
DELETE FROM account_usage_samples
WHERE account_id = ? AND provider = ?
                    "#,
                )
                .bind(account_id)
                .bind(self.default_provider.as_str())
                .execute(self.pool.as_ref())
                .await?
                .rows_affected()
            } else {
                0
            };
            prune_account_usage_samples(
                self.pool.as_ref(),
                account_id,
                self.default_provider.as_str(),
            )
            .await?;

            sqlx::query(
                r#"
UPDATE account_usage
SET
    total_tokens = 0,
    input_tokens = 0,
    cached_input_tokens = 0,
    output_tokens = 0,
    reasoning_output_tokens = 0,
    context_total_tokens = 0,
    min_total_cached_output_tokens = 0,
    sent_bytes = 0,
    recv_bytes = 0,
    sent_recv_bytes = 0,
    updated_at = ?,
    last_backend_limit_id = ?,
    last_backend_limit_name = ?,
    last_backend_used_percent = ?,
    last_snapshot_total_tokens = ?,
    last_snapshot_input_tokens = ?,
    last_snapshot_cached_input_tokens = ?,
    last_snapshot_output_tokens = ?,
    last_snapshot_context_total_tokens = ?,
    last_snapshot_min_total_cached_output_tokens = ?,
    last_snapshot_sent_bytes = ?,
    last_snapshot_recv_bytes = ?,
    last_snapshot_sent_recv_bytes = ?,
    last_snapshot_percent_int = ?,
    window_start_percent_int = ?,
    window_start_total_tokens = ?,
    window_start_input_tokens = ?,
    window_start_cached_input_tokens = ?,
    window_start_output_tokens = ?,
    window_start_context_total_tokens = ?,
    window_start_min_total_cached_output_tokens = ?,
    window_start_sent_bytes = ?,
    window_start_recv_bytes = ?,
    window_start_sent_recv_bytes = ?,
    last_backend_resets_at = ?,
    last_backend_window_minutes = ?,
    last_backend_seen_at = ?
WHERE account_id = ? AND provider = ?
                    "#,
            )
            .bind(now)
            .bind(snapshot.limit_id.as_deref())
            .bind(snapshot.limit_name.as_deref())
            .bind(used_percent)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(resets_at)
            .bind(window_minutes)
            .bind(seen_at)
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .execute(self.pool.as_ref())
            .await?;

            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                format!(
                    "reset=1 reached_full_window={} samples_cleared={cleared_samples}",
                    if reached_full_window { 1 } else { 0 }
                ),
            )
            .await;

            return Ok(());
        }

        let mut snapshot_total_tokens = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_total_tokens").ok())
            .unwrap_or(0);
        let mut snapshot_input_tokens = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_input_tokens").ok())
            .unwrap_or(0);
        let mut snapshot_cached_input_tokens = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_cached_input_tokens")
                    .ok()
            })
            .unwrap_or(0);
        let mut snapshot_output_tokens = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_output_tokens").ok())
            .unwrap_or(0);
        let mut snapshot_context_total_tokens = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_context_total_tokens")
                    .ok()
            })
            .unwrap_or(0);
        let mut snapshot_min_total_cached_output_tokens = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_min_total_cached_output_tokens")
                    .ok()
            })
            .unwrap_or(0);
        let mut snapshot_sent_bytes = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_sent_bytes").ok())
            .unwrap_or(0);
        let mut snapshot_recv_bytes = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_recv_bytes").ok())
            .unwrap_or(0);
        let mut snapshot_sent_recv_bytes = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_sent_recv_bytes").ok())
            .unwrap_or(0);
        let mut snapshot_percent_int = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("last_snapshot_percent_int").ok())
            .unwrap_or(current_percent_int);

        let last_sample_percent = if previous_backend_percent.is_none() {
            0
        } else {
            last_sample_percent
        };

        if current_percent_int != last_sample_percent {
            let delta_percent = (current_percent_int - last_sample_percent).max(0);
            let sample_deltas = SampleTokenDeltas {
                blended_tokens: (total_tokens_now - last_sample_tokens).max(0),
                input_tokens: (input_tokens_now - last_sample_input_tokens).max(0),
                cached_input_tokens: (cached_input_tokens_now - last_sample_cached_input_tokens)
                    .max(0),
                output_tokens: (output_tokens_now - last_sample_output_tokens).max(0),
                context_total_tokens: (context_total_tokens_now - last_sample_context_total_tokens)
                    .max(0),
                min_total_cached_output_tokens: (min_total_cached_output_tokens_now
                    - last_sample_min_total_cached_output_tokens)
                    .max(0),
                sent_bytes: (sent_bytes_now - last_sample_sent_bytes).max(0),
                recv_bytes: (recv_bytes_now - last_sample_recv_bytes).max(0),
                sent_recv_bytes: (sent_recv_bytes_now - last_sample_sent_recv_bytes).max(0),
            };
            if delta_percent > 0 && total_tokens_now > 0 {
                insert_account_usage_sample(
                    self.pool.as_ref(),
                    account_id,
                    self.default_provider.as_str(),
                    now,
                    last_sample_percent,
                    current_percent_int,
                    delta_percent,
                    &sample_deltas,
                    window_minutes,
                    resets_at,
                )
                .await?;
            }

            let updated_window_start_percent = window_start_percent;
            let updated_window_start_tokens = window_start_tokens;
            let updated_window_start_input_tokens = window_start_input_tokens;
            let updated_window_start_cached_input_tokens = window_start_cached_input_tokens;
            let updated_window_start_output_tokens = window_start_output_tokens;
            let updated_window_start_context_total_tokens = window_start_context_total_tokens;
            let updated_window_start_min_total_cached_output_tokens =
                window_start_min_total_cached_output_tokens;
            let updated_window_start_sent_bytes = window_start_sent_bytes;
            let updated_window_start_recv_bytes = window_start_recv_bytes;
            let updated_window_start_sent_recv_bytes = window_start_sent_recv_bytes;
            let estimates = self
                .estimate_account_limit_tokens_multi(account_id)
                .await
                .unwrap_or(AccountLimitEstimates {
                    composite_q_limit: None,
                    blended_limit: None,
                    cached_input_limit: None,
                    output_limit: None,
                    context_total_limit: None,
                    min_total_cached_output_limit: None,
                    min_input_cached_output_limit: None,
                    sent_limit: None,
                    recv_limit: None,
                    sent_recv_limit: None,
                    sample_count: 0,
                });
            let log_message = format_account_limit_estimates(&estimates);
            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                log_message,
            )
            .await;

            snapshot_total_tokens = total_tokens_now;
            snapshot_input_tokens = input_tokens_now;
            snapshot_cached_input_tokens = cached_input_tokens_now;
            snapshot_output_tokens = output_tokens_now;
            snapshot_context_total_tokens = context_total_tokens_now;
            snapshot_min_total_cached_output_tokens = min_total_cached_output_tokens_now;
            snapshot_sent_bytes = sent_bytes_now;
            snapshot_recv_bytes = recv_bytes_now;
            snapshot_sent_recv_bytes = sent_recv_bytes_now;
            snapshot_percent_int = current_percent_int;

            sqlx::query(
                r#"
UPDATE account_usage
SET
    last_snapshot_total_tokens = ?,
    last_snapshot_input_tokens = ?,
    last_snapshot_cached_input_tokens = ?,
    last_snapshot_output_tokens = ?,
    last_snapshot_context_total_tokens = ?,
    last_snapshot_min_total_cached_output_tokens = ?,
    last_snapshot_sent_bytes = ?,
    last_snapshot_recv_bytes = ?,
    last_snapshot_sent_recv_bytes = ?,
    last_snapshot_percent_int = ?,
    window_start_percent_int = ?,
    window_start_total_tokens = ?,
    window_start_input_tokens = ?,
    window_start_cached_input_tokens = ?,
    window_start_output_tokens = ?,
    window_start_context_total_tokens = ?,
    window_start_min_total_cached_output_tokens = ?,
    window_start_sent_bytes = ?,
    window_start_recv_bytes = ?,
    window_start_sent_recv_bytes = ?
WHERE account_id = ? AND provider = ?
                "#,
            )
            .bind(total_tokens_now)
            .bind(input_tokens_now)
            .bind(cached_input_tokens_now)
            .bind(output_tokens_now)
            .bind(context_total_tokens_now)
            .bind(min_total_cached_output_tokens_now)
            .bind(sent_bytes_now)
            .bind(recv_bytes_now)
            .bind(sent_recv_bytes_now)
            .bind(current_percent_int)
            .bind(updated_window_start_percent)
            .bind(updated_window_start_tokens)
            .bind(updated_window_start_input_tokens)
            .bind(updated_window_start_cached_input_tokens)
            .bind(updated_window_start_output_tokens)
            .bind(updated_window_start_context_total_tokens)
            .bind(updated_window_start_min_total_cached_output_tokens)
            .bind(updated_window_start_sent_bytes)
            .bind(updated_window_start_recv_bytes)
            .bind(updated_window_start_sent_recv_bytes)
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .execute(self.pool.as_ref())
            .await?;
        }

        // Keep the usage_pct predictor anchored within each integer-percent window.
        // This prevents regressions between visible backend percent jumps.
        let (
            anchor_percent_int,
            anchor_total_tokens,
            anchor_input_tokens,
            anchor_cached_input_tokens,
            anchor_output_tokens,
            anchor_context_total_tokens,
            anchor_min_total_cached_output_tokens,
            anchor_sent_bytes,
            anchor_recv_bytes,
            anchor_sent_recv_bytes,
        ) = if current_percent_int != last_sample_percent {
            (
                current_percent_int,
                total_tokens_now,
                input_tokens_now,
                cached_input_tokens_now,
                output_tokens_now,
                context_total_tokens_now,
                min_total_cached_output_tokens_now,
                sent_bytes_now,
                recv_bytes_now,
                sent_recv_bytes_now,
            )
        } else {
            (
                window_start_percent,
                window_start_tokens,
                window_start_input_tokens,
                window_start_cached_input_tokens,
                window_start_output_tokens,
                window_start_context_total_tokens,
                window_start_min_total_cached_output_tokens,
                window_start_sent_bytes,
                window_start_recv_bytes,
                window_start_sent_recv_bytes,
            )
        };

        sqlx::query(
            r#"
INSERT INTO account_usage (
    account_id,
    provider,
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    reasoning_output_tokens,
    context_total_tokens,
    min_total_cached_output_tokens,
    sent_bytes,
    recv_bytes,
    sent_recv_bytes,
    updated_at,
    last_backend_limit_id,
    last_backend_limit_name,
    last_backend_used_percent,
    last_snapshot_total_tokens,
    last_snapshot_input_tokens,
    last_snapshot_cached_input_tokens,
    last_snapshot_output_tokens,
    last_snapshot_context_total_tokens,
    last_snapshot_min_total_cached_output_tokens,
    last_snapshot_sent_bytes,
    last_snapshot_recv_bytes,
    last_snapshot_sent_recv_bytes,
    last_snapshot_percent_int,
    window_start_percent_int,
    window_start_total_tokens,
    window_start_input_tokens,
    window_start_cached_input_tokens,
    window_start_output_tokens,
    window_start_context_total_tokens,
    window_start_min_total_cached_output_tokens,
    window_start_sent_bytes,
    window_start_recv_bytes,
    window_start_sent_recv_bytes,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at
 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(account_id, provider) DO UPDATE SET
    updated_at = excluded.updated_at,
    last_backend_limit_id = excluded.last_backend_limit_id,
    last_backend_limit_name = excluded.last_backend_limit_name,
    last_backend_used_percent = excluded.last_backend_used_percent,
    last_snapshot_total_tokens = excluded.last_snapshot_total_tokens,
    last_snapshot_input_tokens = excluded.last_snapshot_input_tokens,
    last_snapshot_cached_input_tokens = excluded.last_snapshot_cached_input_tokens,
    last_snapshot_output_tokens = excluded.last_snapshot_output_tokens,
    last_snapshot_context_total_tokens = excluded.last_snapshot_context_total_tokens,
    last_snapshot_min_total_cached_output_tokens = excluded.last_snapshot_min_total_cached_output_tokens,
    last_snapshot_sent_bytes = excluded.last_snapshot_sent_bytes,
    last_snapshot_recv_bytes = excluded.last_snapshot_recv_bytes,
    last_snapshot_sent_recv_bytes = excluded.last_snapshot_sent_recv_bytes,
    last_snapshot_percent_int = excluded.last_snapshot_percent_int,
    window_start_percent_int = excluded.window_start_percent_int,
    window_start_total_tokens = excluded.window_start_total_tokens,
    window_start_input_tokens = excluded.window_start_input_tokens,
    window_start_cached_input_tokens = excluded.window_start_cached_input_tokens,
    window_start_output_tokens = excluded.window_start_output_tokens,
    window_start_context_total_tokens = excluded.window_start_context_total_tokens,
    window_start_min_total_cached_output_tokens = excluded.window_start_min_total_cached_output_tokens,
    window_start_sent_bytes = excluded.window_start_sent_bytes,
    window_start_recv_bytes = excluded.window_start_recv_bytes,
    window_start_sent_recv_bytes = excluded.window_start_sent_recv_bytes,
    last_backend_resets_at = excluded.last_backend_resets_at,
    last_backend_window_minutes = excluded.last_backend_window_minutes,
    last_backend_seen_at = excluded.last_backend_seen_at
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(now)
        .bind(snapshot.limit_id.as_deref())
        .bind(snapshot.limit_name.as_deref())
        .bind(used_percent)
        .bind(snapshot_total_tokens)
        .bind(snapshot_input_tokens)
        .bind(snapshot_cached_input_tokens)
        .bind(snapshot_output_tokens)
        .bind(snapshot_context_total_tokens)
        .bind(snapshot_min_total_cached_output_tokens)
        .bind(snapshot_sent_bytes)
        .bind(snapshot_recv_bytes)
        .bind(snapshot_sent_recv_bytes)
        .bind(snapshot_percent_int)
        .bind(anchor_percent_int)
        .bind(anchor_total_tokens)
        .bind(anchor_input_tokens)
        .bind(anchor_cached_input_tokens)
        .bind(anchor_output_tokens)
        .bind(anchor_context_total_tokens)
        .bind(anchor_min_total_cached_output_tokens)
        .bind(anchor_sent_bytes)
        .bind(anchor_recv_bytes)
        .bind(anchor_sent_recv_bytes)
        .bind(resets_at)
        .bind(window_minutes)
        .bind(seen_at)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn record_usage_limit_reached(&self, account_id: &str) -> anyhow::Result<()> {
        let previous_percent = sqlx::query(
            r#"
SELECT
    last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .fetch_optional(self.pool.as_ref())
        .await?
        .and_then(|row| row.try_get::<f64, _>("last_backend_used_percent").ok());

        self.log_usage_event(
            account_id,
            Some(101.0),
            previous_percent,
            "usage_limit_reached=1 synthetic_used_percent=101".to_string(),
        )
        .await;

        Ok(())
    }

    async fn log_usage_event(
        &self,
        account_id: &str,
        used_percent: Option<f64>,
        previous_percent: Option<f64>,
        message: String,
    ) {
        let is_token_usage_event = message.starts_with("total=");
        let is_backend_delta_event = message.contains("tokens_per_pct=")
            || message.contains("avg_tokens_per_pct=")
            || message.contains("avg_tpp=");
        let used_percent = if used_percent.is_some() {
            used_percent
        } else {
            let row = sqlx::query(
                r#"
SELECT
    last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
                "#,
            )
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .fetch_optional(self.pool.as_ref())
            .await
            .ok()
            .flatten();
            let last_used_percent = row
                .as_ref()
                .and_then(|row| row.try_get::<f64, _>("last_backend_used_percent").ok());
            used_percent.or(last_used_percent)
        };

        let sample_count = if is_token_usage_event {
            None
        } else {
            Some(
                self.estimate_account_limit_tokens(account_id)
                    .await
                    .map(|(_, samples)| samples)
                    .unwrap_or(0),
            )
        };
        let percent_display = if is_token_usage_event {
            None
        } else {
            let used_percent_int = used_percent.map(|value| value.floor() as i64);
            Some(match (previous_percent, used_percent_int) {
                (Some(previous), Some(current)) => {
                    let previous_int = previous.floor() as i64;
                    if previous_int != current {
                        format!("percent={previous_int}->{current}")
                    } else {
                        format!("percent={current}")
                    }
                }
                (None, Some(current)) => {
                    if current > 0 {
                        format!("percent=0->{current}")
                    } else {
                        format!("percent={current}")
                    }
                }
                _ => "percent=unknown".to_string(),
            })
        };
        let account_display = self.resolve_account_display(account_id).await;
        let pid = std::process::id();
        let pid_label = if std::env::args().any(|arg| arg == "status" || arg == "exec") {
            "pid:"
        } else {
            "pid="
        };
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let usage_pct_suffix = if message.contains("usage_pct=") {
            String::new()
        } else {
            let usage_row = sqlx::query(
                r#"
SELECT
    last_backend_used_percent,
    window_start_percent_int,
    window_start_total_tokens,
    total_tokens,
    window_start_input_tokens,
    input_tokens,
    window_start_cached_input_tokens,
    cached_input_tokens,
    window_start_output_tokens,
    output_tokens,
    window_start_context_total_tokens,
    context_total_tokens,
    window_start_min_total_cached_output_tokens,
    min_total_cached_output_tokens,
    window_start_sent_bytes,
    sent_bytes,
    window_start_recv_bytes,
    recv_bytes,
    window_start_sent_recv_bytes,
    sent_recv_bytes
FROM account_usage
WHERE account_id = ? AND provider = ?
                "#,
            )
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .fetch_optional(self.pool.as_ref())
            .await
            .ok()
            .flatten()
            .map(|row| {
                let backend_anchor_percent = row
                    .try_get::<Option<f64>, _>("last_backend_used_percent")
                    .ok()
                    .flatten();
                let window_start_percent = row.try_get::<i64, _>("window_start_percent_int").ok();
                let window_start_total_tokens =
                    row.try_get::<i64, _>("window_start_total_tokens").ok();
                let total_tokens_now = row.try_get::<i64, _>("total_tokens").ok();
                let window_start_input_tokens =
                    row.try_get::<i64, _>("window_start_input_tokens").ok();
                let input_tokens_now = row.try_get::<i64, _>("input_tokens").ok();
                let window_start_cached_input_tokens = row
                    .try_get::<i64, _>("window_start_cached_input_tokens")
                    .ok();
                let cached_input_tokens_now = row.try_get::<i64, _>("cached_input_tokens").ok();
                let window_start_output_tokens =
                    row.try_get::<i64, _>("window_start_output_tokens").ok();
                let output_tokens_now = row.try_get::<i64, _>("output_tokens").ok();
                let window_start_context_total_tokens = row
                    .try_get::<i64, _>("window_start_context_total_tokens")
                    .ok();
                let context_total_tokens_now = row.try_get::<i64, _>("context_total_tokens").ok();
                let window_start_min_total_cached_output_tokens = row
                    .try_get::<i64, _>("window_start_min_total_cached_output_tokens")
                    .ok();
                let min_total_cached_output_tokens_now =
                    row.try_get::<i64, _>("min_total_cached_output_tokens").ok();
                let window_start_sent_bytes = row.try_get::<i64, _>("window_start_sent_bytes").ok();
                let sent_bytes_now = row.try_get::<i64, _>("sent_bytes").ok();
                let window_start_recv_bytes = row.try_get::<i64, _>("window_start_recv_bytes").ok();
                let recv_bytes_now = row.try_get::<i64, _>("recv_bytes").ok();
                let window_start_sent_recv_bytes =
                    row.try_get::<i64, _>("window_start_sent_recv_bytes").ok();
                let sent_recv_bytes_now = row.try_get::<i64, _>("sent_recv_bytes").ok();
                (
                    backend_anchor_percent,
                    window_start_percent,
                    window_start_total_tokens,
                    total_tokens_now,
                    window_start_input_tokens,
                    input_tokens_now,
                    window_start_cached_input_tokens,
                    cached_input_tokens_now,
                    window_start_output_tokens,
                    output_tokens_now,
                    window_start_context_total_tokens,
                    context_total_tokens_now,
                    window_start_min_total_cached_output_tokens,
                    min_total_cached_output_tokens_now,
                    window_start_sent_bytes,
                    sent_bytes_now,
                    window_start_recv_bytes,
                    recv_bytes_now,
                    window_start_sent_recv_bytes,
                    sent_recv_bytes_now,
                )
            });
            if let Some((
                Some(backend_anchor_percent),
                Some(window_start_percent),
                Some(window_start_total_tokens),
                Some(total_tokens_now),
                Some(window_start_input_tokens),
                Some(input_tokens_now),
                Some(window_start_cached_input_tokens),
                Some(cached_input_tokens_now),
                Some(window_start_output_tokens),
                Some(output_tokens_now),
                Some(window_start_context_total_tokens),
                Some(context_total_tokens_now),
                Some(window_start_min_total_cached_output_tokens),
                Some(min_total_cached_output_tokens_now),
                Some(window_start_sent_bytes),
                Some(sent_bytes_now),
                Some(window_start_recv_bytes),
                Some(recv_bytes_now),
                Some(window_start_sent_recv_bytes),
                Some(sent_recv_bytes_now),
            )) = usage_row
            {
                let estimates = self
                    .estimate_account_limit_tokens_multi(account_id)
                    .await
                    .unwrap_or(AccountLimitEstimates {
                        composite_q_limit: None,
                        blended_limit: None,
                        cached_input_limit: None,
                        output_limit: None,
                        context_total_limit: None,
                        min_total_cached_output_limit: None,
                        min_input_cached_output_limit: None,
                        sent_limit: None,
                        recv_limit: None,
                        sent_recv_limit: None,
                        sample_count: 0,
                    });
                let usage_pct_values = format_metric_values(
                    [
                        estimates.composite_q_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log_f64(
                                composite_q_tokens(
                                    input_tokens_now,
                                    cached_input_tokens_now,
                                    output_tokens_now,
                                ),
                                backend_anchor_percent,
                                window_start_percent,
                                composite_q_tokens(
                                    window_start_input_tokens,
                                    window_start_cached_input_tokens,
                                    window_start_output_tokens,
                                ),
                                allowance,
                            )
                        }),
                        estimates.blended_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                total_tokens_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_total_tokens,
                                allowance,
                            )
                        }),
                        estimates.cached_input_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                cached_input_tokens_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_cached_input_tokens,
                                allowance,
                            )
                        }),
                        estimates.output_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                output_tokens_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_output_tokens,
                                allowance,
                            )
                        }),
                        estimates.context_total_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                context_total_tokens_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_context_total_tokens,
                                allowance,
                            )
                        }),
                        estimates
                            .min_total_cached_output_limit
                            .and_then(|allowance| {
                                estimate_account_usage_percent_for_log(
                                    min_total_cached_output_tokens_now,
                                    backend_anchor_percent,
                                    window_start_percent,
                                    window_start_min_total_cached_output_tokens,
                                    allowance,
                                )
                            }),
                        estimates
                            .min_input_cached_output_limit
                            .and_then(|allowance| {
                                estimate_account_usage_percent_for_log(
                                    min_input_cached_output_tokens(
                                        input_tokens_now,
                                        cached_input_tokens_now,
                                        output_tokens_now,
                                    ),
                                    backend_anchor_percent,
                                    window_start_percent,
                                    min_input_cached_output_tokens(
                                        window_start_input_tokens,
                                        window_start_cached_input_tokens,
                                        window_start_output_tokens,
                                    ),
                                    allowance,
                                )
                            }),
                        estimates.sent_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                sent_bytes_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_sent_bytes,
                                allowance,
                            )
                        }),
                        estimates.recv_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                recv_bytes_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_recv_bytes,
                                allowance,
                            )
                        }),
                        estimates.sent_recv_limit.and_then(|allowance| {
                            estimate_account_usage_percent_for_log(
                                sent_recv_bytes_now,
                                backend_anchor_percent,
                                window_start_percent,
                                window_start_sent_recv_bytes,
                                allowance,
                            )
                        }),
                    ],
                    /*precision*/ 2,
                );
                format!(" usage_pct[q/b/c/o/x/m/n/s/r/z]={usage_pct_values}%")
            } else {
                String::new()
            }
        };
        if let Some(mut file) = open_usage_log_file(&account_display) {
            let suffix = if message.is_empty() {
                String::new()
            } else {
                format!(" {message}")
            };
            if is_token_usage_event {
                let _ = writeln!(file, "{ts} {pid_label}{pid}{usage_pct_suffix}{suffix}");
            } else {
                let percent_display =
                    percent_display.unwrap_or_else(|| "percent=unknown".to_string());
                if is_backend_delta_event {
                    let _ = writeln!(
                        file,
                        "{ts} {pid_label}{pid} {percent_display}{usage_pct_suffix}{suffix}"
                    );
                } else {
                    let sample_count = sample_count.unwrap_or(0);
                    let _ = writeln!(
                        file,
                        "{ts} {pid_label}{pid} {percent_display} samples={sample_count}{usage_pct_suffix}{suffix}",
                    );
                }
            }
        }
    }

    async fn resolve_account_display(&self, account_id: &str) -> String {
        if let Some(email) = account_id.strip_prefix("email:") {
            return email.to_string();
        }
        let displays = self.account_displays.lock().await;
        displays
            .get(account_id)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    }
}

async fn open_sqlite(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .log_statements(LevelFilter::Off);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("failed to connect to usage db at {}", path.display()))?;
    USAGE_MIGRATOR
        .run(&pool)
        .await
        .with_context(|| format!("failed to migrate usage db at {}", path.display()))?;
    Ok(pool)
}

fn db_filename(base_name: &str, version: u32) -> String {
    format!("{base_name}_{version}.sqlite")
}

pub fn usage_db_filename() -> String {
    db_filename(USAGE_DB_FILENAME, USAGE_DB_VERSION)
}

pub fn usage_db_path(sqlite_home: &Path) -> PathBuf {
    sqlite_home.join(usage_db_filename())
}

fn usage_log_filename(account_display: &str) -> String {
    format!("{USAGE_LOG_FILENAME_PREFIX}{account_display}{USAGE_LOG_FILENAME_SUFFIX}")
}

fn usage_log_path(account_display: &str) -> Option<PathBuf> {
    let codex_home = find_codex_home().ok()?;
    Some(
        codex_home
            .join(USAGE_LOG_DIRNAME)
            .join(usage_log_filename(account_display)),
    )
}

fn open_usage_log_file(account_display: &str) -> Option<std::fs::File> {
    let path = usage_log_path(account_display)?;
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn estimate_account_usage_percent_for_log(
    total_tokens: i64,
    backend_anchor_percent: f64,
    backend_anchor_percent_int: i64,
    backend_anchor_tokens: i64,
    estimated_limit: f64,
) -> Option<f64> {
    estimate_account_usage_percent_for_log_f64(
        total_tokens as f64,
        backend_anchor_percent,
        backend_anchor_percent_int,
        backend_anchor_tokens as f64,
        estimated_limit,
    )
}

fn estimate_account_usage_percent_for_log_f64(
    total_tokens: f64,
    backend_anchor_percent: f64,
    backend_anchor_percent_int: i64,
    backend_anchor_tokens: f64,
    estimated_limit: f64,
) -> Option<f64> {
    if estimated_limit <= 0.0 || !estimated_limit.is_finite() {
        return None;
    }
    let base_percent = if backend_anchor_percent.is_finite() {
        backend_anchor_percent.max(backend_anchor_percent_int.max(0) as f64)
    } else {
        backend_anchor_percent_int.max(0) as f64
    };
    let delta_tokens = (total_tokens - backend_anchor_tokens).max(0.0);
    let avg_tokens_per_pct = estimated_limit / 100.0;
    if avg_tokens_per_pct <= 0.0 || !avg_tokens_per_pct.is_finite() {
        return None;
    }
    let percent = delta_tokens / avg_tokens_per_pct;
    Some(base_percent + percent)
}

fn normalize_usage_for_accounting(usage: &TokenUsage) -> TokenUsage {
    let cached_input_tokens = usage.cached_input_tokens.max(0);
    let input_tokens = usage.non_cached_input();
    let output_tokens = usage.output_tokens.max(0);
    TokenUsage {
        total_tokens: usage.blended_total(),
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens.max(0),
    }
}

fn min_input_cached_output_tokens(
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
) -> i64 {
    input_tokens.min(cached_input_tokens).max(0) + output_tokens.max(0)
}

fn composite_q_tokens(input_tokens: i64, cached_input_tokens: i64, output_tokens: i64) -> f64 {
    output_tokens.max(0) as f64
        + COMPOSITE_Q_INPUT_WEIGHT * input_tokens.max(0) as f64
        + COMPOSITE_Q_CACHED_INPUT_WEIGHT * cached_input_tokens.max(0) as f64
}

fn format_account_limit_estimates(estimates: &AccountLimitEstimates) -> String {
    let avg = format_metric_values_si([
        estimates.composite_q_limit.map(|value| value / 100.0),
        estimates.blended_limit.map(|value| value / 100.0),
        estimates.cached_input_limit.map(|value| value / 100.0),
        estimates.output_limit.map(|value| value / 100.0),
        estimates.context_total_limit.map(|value| value / 100.0),
        estimates
            .min_total_cached_output_limit
            .map(|value| value / 100.0),
        estimates
            .min_input_cached_output_limit
            .map(|value| value / 100.0),
        estimates.sent_limit.map(|value| value / 100.0),
        estimates.recv_limit.map(|value| value / 100.0),
        estimates.sent_recv_limit.map(|value| value / 100.0),
    ]);
    let allowance = format_metric_values_si([
        estimates.composite_q_limit,
        estimates.blended_limit,
        estimates.cached_input_limit,
        estimates.output_limit,
        estimates.context_total_limit,
        estimates.min_total_cached_output_limit,
        estimates.min_input_cached_output_limit,
        estimates.sent_limit,
        estimates.recv_limit,
        estimates.sent_recv_limit,
    ]);
    format!("avg_tpp={avg} est_allow={allowance}")
}

fn format_metric_values_si(values: [Option<f64>; 10]) -> String {
    let value = |entry: Option<f64>| match entry {
        Some(number) if number.is_finite() && number >= 0.0 => format_si_three_digits(number),
        _ => "-".to_string(),
    };
    values.into_iter().map(value).collect::<Vec<_>>().join("/")
}

fn format_metric_values(values: [Option<f64>; 10], precision: usize) -> String {
    let value = |entry: Option<f64>| match entry {
        Some(number) if number.is_finite() && number >= 0.0 => format!("{number:.precision$}"),
        _ => "-".to_string(),
    };
    values.into_iter().map(value).collect::<Vec<_>>().join("/")
}

fn format_si_three_digits(mut value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let suffixes = ["", "K", "M", "G", "T", "P", "E"];
    let mut suffix_index = 0usize;
    while value >= 1000.0 && suffix_index + 1 < suffixes.len() {
        value /= 1000.0;
        suffix_index += 1;
    }

    let decimals = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.decimals$}{}", suffixes[suffix_index])
}

#[allow(clippy::too_many_arguments)]
async fn insert_account_usage_sample(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    observed_at: i64,
    start_percent_int: i64,
    end_percent_int: i64,
    delta_percent_int: i64,
    deltas: &SampleTokenDeltas,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
INSERT INTO account_usage_samples (
    account_id,
    provider,
    observed_at,
    start_percent_int,
    end_percent_int,
    delta_percent_int,
    delta_tokens,
    delta_input_tokens,
    delta_cached_input_tokens,
    delta_output_tokens,
    delta_context_total_tokens,
    delta_min_total_cached_output_tokens,
    delta_sent_bytes,
    delta_recv_bytes,
    delta_sent_recv_bytes,
    window_minutes,
    resets_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(account_id)
    .bind(provider)
    .bind(observed_at)
    .bind(start_percent_int)
    .bind(end_percent_int)
    .bind(delta_percent_int)
    .bind(deltas.blended_tokens)
    .bind(deltas.input_tokens)
    .bind(deltas.cached_input_tokens)
    .bind(deltas.output_tokens)
    .bind(deltas.context_total_tokens)
    .bind(deltas.min_total_cached_output_tokens)
    .bind(deltas.sent_bytes)
    .bind(deltas.recv_bytes)
    .bind(deltas.sent_recv_bytes)
    .bind(window_minutes)
    .bind(resets_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn prune_account_usage_samples(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
DELETE FROM account_usage_samples
WHERE id IN (
    SELECT id
    FROM account_usage_samples
    WHERE account_id = ? AND provider = ?
    ORDER BY observed_at DESC, id DESC
    LIMIT -1 OFFSET 1000
)
        "#,
    )
    .bind(account_id)
    .bind(provider)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn account_usage_records_and_reads_tokens() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 123,
            input_tokens: 100,
            cached_input_tokens: 10,
            output_tokens: 13,
            reasoning_output_tokens: 0,
        };

        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record");

        let snapshot = runtime
            .get_account_usage("account-1")
            .await
            .expect("read")
            .expect("row");

        assert_eq!(snapshot.total_tokens, 103);
        assert_eq!(snapshot.input_tokens, 90);
        assert_eq!(snapshot.cached_input_tokens, 10);
        assert_eq!(snapshot.output_tokens, 13);
    }

    #[tokio::test]
    async fn clear_usage_for_account_deletes_only_target_account_rows() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 100,
            input_tokens: 80,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record account-1");
        runtime
            .record_account_token_usage("account-2", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record account-2");

        insert_account_usage_sample(
            runtime.pool.as_ref(),
            "account-1",
            "test-provider",
            /*observed_at*/ 1,
            /*start_percent_int*/ 0,
            /*end_percent_int*/ 1,
            /*delta_percent_int*/ 1,
            &SampleTokenDeltas {
                blended_tokens: 100,
                input_tokens: 80,
                cached_input_tokens: 0,
                output_tokens: 20,
                context_total_tokens: 100,
                min_total_cached_output_tokens: 20,
                sent_bytes: 0,
                recv_bytes: 0,
                sent_recv_bytes: 0,
            },
            Some(60),
            Some(123),
        )
        .await
        .expect("insert account-1 sample");
        insert_account_usage_sample(
            runtime.pool.as_ref(),
            "account-2",
            "test-provider",
            /*observed_at*/ 1,
            /*start_percent_int*/ 0,
            /*end_percent_int*/ 1,
            /*delta_percent_int*/ 1,
            &SampleTokenDeltas {
                blended_tokens: 100,
                input_tokens: 80,
                cached_input_tokens: 0,
                output_tokens: 20,
                context_total_tokens: 100,
                min_total_cached_output_tokens: 20,
                sent_bytes: 0,
                recv_bytes: 0,
                sent_recv_bytes: 0,
            },
            Some(60),
            Some(123),
        )
        .await
        .expect("insert account-2 sample");

        let (usage_rows_deleted, sample_rows_deleted) = runtime
            .clear_usage_for_account("account-1")
            .await
            .expect("clear account-1");
        assert_eq!(usage_rows_deleted, 1);
        assert_eq!(sample_rows_deleted, 1);

        let account_1_usage = runtime
            .get_account_usage("account-1")
            .await
            .expect("read account-1");
        assert!(account_1_usage.is_none());
        let account_2_usage = runtime
            .get_account_usage("account-2")
            .await
            .expect("read account-2");
        assert!(account_2_usage.is_some());

        let account_1_samples: i64 = sqlx::query_scalar(
            r#"
SELECT COUNT(*) FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count account-1 samples");
        let account_2_samples: i64 = sqlx::query_scalar(
            r#"
SELECT COUNT(*) FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-2")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count account-2 samples");
        assert_eq!(account_1_samples, 0);
        assert_eq!(account_2_samples, 1);
    }

    #[tokio::test]
    async fn clear_usage_for_all_accounts_deletes_only_default_provider_rows() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 100,
            input_tokens: 80,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        sqlx::query(
            r#"
INSERT INTO account_usage (
    account_id,
    provider,
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    reasoning_output_tokens,
    updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind("account-external")
        .bind("other-provider")
        .bind(1_i64)
        .bind(1_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(1_i64)
        .execute(runtime.pool.as_ref())
        .await
        .expect("insert other-provider account_usage");
        sqlx::query(
            r#"
INSERT INTO account_usage_samples (
    account_id,
    provider,
    observed_at,
    start_percent_int,
    end_percent_int,
    delta_percent_int,
    delta_tokens,
    delta_input_tokens,
    delta_cached_input_tokens,
    delta_output_tokens,
    delta_context_total_tokens,
    delta_min_total_cached_output_tokens
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind("account-external")
        .bind("other-provider")
        .bind(1_i64)
        .bind(0_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(1_i64)
        .bind(0_i64)
        .execute(runtime.pool.as_ref())
        .await
        .expect("insert other-provider account_usage_samples");

        let (usage_rows_deleted, sample_rows_deleted) = runtime
            .clear_usage_for_all_accounts()
            .await
            .expect("clear all test-provider accounts");
        assert_eq!(usage_rows_deleted, 1);
        assert_eq!(sample_rows_deleted, 0);

        let default_provider_usage_count: i64 = sqlx::query_scalar(
            r#"
SELECT COUNT(*) FROM account_usage
WHERE provider = ?
            "#,
        )
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count test-provider usage");
        let other_provider_usage_count: i64 = sqlx::query_scalar(
            r#"
SELECT COUNT(*) FROM account_usage
WHERE provider = ?
            "#,
        )
        .bind("other-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count other-provider usage");
        let other_provider_sample_count: i64 = sqlx::query_scalar(
            r#"
SELECT COUNT(*) FROM account_usage_samples
WHERE provider = ?
            "#,
        )
        .bind("other-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("count other-provider samples");
        assert_eq!(default_provider_usage_count, 0);
        assert_eq!(other_provider_usage_count, 1);
        assert_eq!(other_provider_sample_count, 1);
    }

    #[tokio::test]
    async fn account_usage_resets_totals_when_backend_window_resets() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 123,
            input_tokens: 100,
            cached_input_tokens: 10,
            output_tokens: 13,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 12.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record backend");

        let snapshot_reset = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 0.0,
                window_minutes: Some(10080),
                resets_at: Some(67890),
            }),
            ..snapshot
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot_reset)
            .await
            .expect("record reset");

        let row = sqlx::query(
            r#"
SELECT total_tokens, last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("usage row");

        let total_tokens: i64 = row.try_get("total_tokens").expect("total_tokens");
        let backend_used_percent: f64 = row
            .try_get("last_backend_used_percent")
            .expect("backend used");

        assert_eq!(total_tokens, 0);
        assert_eq!(backend_used_percent, 0.0);

        let remaining_samples: i64 = sqlx::query(
            r#"
SELECT COUNT(*) AS count
FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("sample count")
        .try_get("count")
        .expect("count");
        assert_eq!(remaining_samples, 1);
    }

    #[tokio::test]
    async fn account_usage_records_samples_on_percent_increase() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 100,
            input_tokens: 80,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 1.2,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record snapshot");

        let usage = TokenUsage {
            total_tokens: 50,
            input_tokens: 40,
            cached_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot_2 = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 2.2,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            ..snapshot
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot_2)
            .await
            .expect("record snapshot 2");

        let (limit, samples) = runtime
            .estimate_account_limit_tokens("account-1")
            .await
            .expect("estimate");
        let limit = limit.expect("estimate");
        assert_eq!(samples, 2);
        // 150 tokens across a 2% increase implies a 7,500 token allowance.
        let expected_limit = 150.0 / (2.0 / 100.0);
        assert!((limit - expected_limit).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn account_usage_tracks_latest_backend_anchor() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 120,
            input_tokens: 100,
            cached_input_tokens: 10,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 1.2,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record snapshot");

        let row = sqlx::query(
            r#"
SELECT
    window_start_percent_int,
    window_start_total_tokens,
    window_start_input_tokens,
    window_start_cached_input_tokens,
    window_start_output_tokens
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("window start row");

        let window_start_percent: i64 = row
            .try_get("window_start_percent_int")
            .expect("window_start_percent_int");
        let window_start_total: i64 = row
            .try_get("window_start_total_tokens")
            .expect("window_start_total_tokens");
        let window_start_input: i64 = row
            .try_get("window_start_input_tokens")
            .expect("window_start_input_tokens");
        let window_start_cached_input: i64 = row
            .try_get("window_start_cached_input_tokens")
            .expect("window_start_cached_input_tokens");
        let window_start_output: i64 = row
            .try_get("window_start_output_tokens")
            .expect("window_start_output_tokens");

        assert_eq!(window_start_percent, 1);
        assert_eq!(window_start_total, 120);
        assert_eq!(window_start_input, 90);
        assert_eq!(window_start_cached_input, 10);
        assert_eq!(window_start_output, 20);
    }

    #[tokio::test]
    async fn account_usage_reset_clears_samples_only_after_hitting_100_percent() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 200,
            input_tokens: 160,
            cached_input_tokens: 0,
            output_tokens: 40,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 100.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record 100% snapshot");

        let samples_before_reset: i64 = sqlx::query(
            r#"
SELECT COUNT(*) AS count
FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("sample count before reset")
        .try_get("count")
        .expect("count");
        assert_eq!(samples_before_reset, 1);

        let snapshot_reset = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 0.0,
                window_minutes: Some(10080),
                resets_at: Some(67890),
            }),
            ..snapshot
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot_reset)
            .await
            .expect("record reset");

        let samples_after_reset: i64 = sqlx::query(
            r#"
SELECT COUNT(*) AS count
FROM account_usage_samples
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("sample count after reset")
        .try_get("count")
        .expect("count");
        assert_eq!(samples_after_reset, 0);
    }

    #[tokio::test]
    async fn account_usage_ignores_non_codex_limits() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("other".to_string()),
            limit_name: Some("Monthly".to_string()),
            primary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 50.0,
                window_minutes: Some(43200),
                resets_at: Some(12345),
            }),
            secondary: None,
            credits: None,
            plan_type: None,
        };

        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record");

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM account_usage")
            .fetch_one(runtime.pool.as_ref())
            .await
            .expect("count")
            .try_get("count")
            .expect("count");

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn account_usage_gates_used_percent_drop_without_rewinding_totals() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 100,
            input_tokens: 80,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 1.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot)
            .await
            .expect("record snapshot");

        let usage = TokenUsage {
            total_tokens: 50,
            input_tokens: 40,
            cached_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let snapshot_2 = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 2.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            ..snapshot.clone()
        };
        runtime
            .record_account_backend_rate_limit("account-1", &snapshot_2)
            .await
            .expect("record snapshot 2");

        let row_before = sqlx::query(
            r#"
SELECT
    total_tokens,
    last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("usage row before drop");
        let total_tokens_before: i64 = row_before.try_get("total_tokens").expect("total_tokens");
        let last_backend_used_percent_before: f64 = row_before
            .try_get("last_backend_used_percent")
            .expect("last_backend_used_percent");
        assert_eq!(last_backend_used_percent_before, 2.0);

        let refund_snapshot = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 1.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            ..snapshot
        };
        runtime
            .record_account_backend_rate_limit("account-1", &refund_snapshot)
            .await
            .expect("record refund snapshot");

        let row_after_first_drop = sqlx::query(
            r#"
SELECT
    total_tokens,
    last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("usage row after first drop");
        let total_tokens_after_first_drop: i64 = row_after_first_drop
            .try_get("total_tokens")
            .expect("total_tokens");
        let last_backend_used_percent_after_first_drop: f64 = row_after_first_drop
            .try_get("last_backend_used_percent")
            .expect("last_backend_used_percent");
        assert_eq!(total_tokens_after_first_drop, total_tokens_before);
        assert_eq!(last_backend_used_percent_after_first_drop, 2.0);

        runtime
            .record_account_backend_rate_limit("account-1", &refund_snapshot)
            .await
            .expect("confirm refund snapshot");

        let row_after_confirmation = sqlx::query(
            r#"
SELECT
    total_tokens,
    last_backend_used_percent
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("usage row after confirmation");
        let total_tokens_after_confirmation: i64 = row_after_confirmation
            .try_get("total_tokens")
            .expect("total_tokens");
        let last_backend_used_percent_after_confirmation: f64 = row_after_confirmation
            .try_get("last_backend_used_percent")
            .expect("last_backend_used_percent");

        assert_eq!(total_tokens_after_confirmation, total_tokens_before);
        assert_eq!(last_backend_used_percent_after_confirmation, 1.0);
    }

    #[tokio::test]
    async fn account_usage_confirms_pending_backend_change_across_store_reinit() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("init");

        let usage = TokenUsage {
            total_tokens: 100,
            input_tokens: 80,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage("account-1", &usage, AccountUsageEventMeta::default())
            .await
            .expect("record usage");

        let baseline_snapshot = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: Some("Weekly".to_string()),
            primary: None,
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 50.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            credits: None,
            plan_type: None,
        };
        runtime
            .record_account_backend_rate_limit("account-1", &baseline_snapshot)
            .await
            .expect("record baseline snapshot pending");
        runtime
            .record_account_backend_rate_limit("account-1", &baseline_snapshot)
            .await
            .expect("confirm baseline snapshot");

        let suspicious_snapshot = RateLimitSnapshot {
            secondary: Some(codex_protocol::protocol::RateLimitWindow {
                used_percent: 59.0,
                window_minutes: Some(10080),
                resets_at: Some(12345),
            }),
            ..baseline_snapshot.clone()
        };
        runtime
            .record_account_backend_rate_limit("account-1", &suspicious_snapshot)
            .await
            .expect("record suspicious snapshot");

        let row_after_pending = sqlx::query(
            r#"
SELECT
    last_backend_used_percent,
    last_snapshot_percent_int
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("usage row after pending");
        let last_backend_used_percent_after_pending: f64 = row_after_pending
            .try_get("last_backend_used_percent")
            .expect("last_backend_used_percent");
        let last_snapshot_percent_after_pending: i64 = row_after_pending
            .try_get("last_snapshot_percent_int")
            .expect("last_snapshot_percent_int");
        assert_eq!(last_backend_used_percent_after_pending, 59.0);
        assert_eq!(last_snapshot_percent_after_pending, 50);

        let runtime_reinit =
            AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
                .await
                .expect("reinit");
        runtime_reinit
            .record_account_backend_rate_limit("account-1", &suspicious_snapshot)
            .await
            .expect("confirm suspicious snapshot after reinit");

        let row_after_confirm = sqlx::query(
            r#"
SELECT
    last_backend_used_percent,
    last_snapshot_percent_int
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind("account-1")
        .bind("test-provider")
        .fetch_one(runtime_reinit.pool.as_ref())
        .await
        .expect("usage row after confirm");
        let last_backend_used_percent_after_confirm: f64 = row_after_confirm
            .try_get("last_backend_used_percent")
            .expect("last_backend_used_percent");
        let last_snapshot_percent_after_confirm: i64 = row_after_confirm
            .try_get("last_snapshot_percent_int")
            .expect("last_snapshot_percent_int");
        assert_eq!(last_backend_used_percent_after_confirm, 59.0);
        assert_eq!(last_snapshot_percent_after_confirm, 59);
    }

    #[test]
    fn estimate_account_usage_percent_for_log_can_exceed_100() {
        // estimated_limit=1000 -> avg_tokens_per_pct=10.
        // base_percent=95.2 and delta_tokens=1000 => +100 percentage points.
        let usage_pct = estimate_account_usage_percent_for_log(
            /*total_tokens*/ 1_050, /*backend_anchor_percent*/ 95.2,
            /*backend_anchor_percent_int*/ 95, /*backend_anchor_tokens*/ 50,
            /*estimated_limit*/ 1_000.0,
        );
        assert_eq!(usage_pct, Some(195.2));
    }

    #[test]
    fn format_si_three_digits_uses_three_significant_digits() {
        assert_eq!(format_si_three_digits(/*value*/ 2_646_777.0), "2.65M");
        assert_eq!(format_si_three_digits(/*value*/ 35_705_600.0), "35.7M");
        assert_eq!(format_si_three_digits(/*value*/ 24_813.0), "24.8K");
    }
    #[test]
    fn normalize_usage_for_accounting_uses_non_cached_input() {
        let usage = TokenUsage {
            input_tokens: 250,
            cached_input_tokens: 200,
            output_tokens: 25,
            reasoning_output_tokens: 3,
            total_tokens: 275,
        };

        let normalized = normalize_usage_for_accounting(&usage);
        assert_eq!(
            normalized,
            TokenUsage {
                input_tokens: 50,
                cached_input_tokens: 200,
                output_tokens: 25,
                reasoning_output_tokens: 3,
                total_tokens: 75,
            }
        );
    }

    #[test]
    fn min_input_cached_output_tokens_uses_min_input_plus_output() {
        assert_eq!(
            min_input_cached_output_tokens(
                /*input_tokens*/ 120, /*cached_input_tokens*/ 80,
                /*output_tokens*/ 35,
            ),
            115
        );
        assert_eq!(
            min_input_cached_output_tokens(
                /*input_tokens*/ 40, /*cached_input_tokens*/ 150,
                /*output_tokens*/ 10,
            ),
            50
        );
    }
}
