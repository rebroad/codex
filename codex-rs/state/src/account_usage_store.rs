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
const DEFAULT_CODEX_HOME_DIRNAME: &str = ".codex";
const USAGE_LOG_DIR_ENV_VAR: &str = "CODEX_USAGE_LOG_DIR";
const USAGE_LOG_FILENAME_PREFIX: &str = "usage-";
const USAGE_LOG_FILENAME_SUFFIX: &str = ".log";
const USAGE_LIMIT_100_LOG_FILENAME: &str = "usage-limit-100.log";
const USAGE_LIMIT_101_LOG_FILENAME: &str = "usage-limit-101.log";
const STABILIZED_BACKEND_MEDIAN_WINDOW_SAMPLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccountUsageEstimatorConfig {
    pub min_usage_pct_sample_count: i64,
    pub max_usage_pct_display_percent_before_full: f64,
    pub stable_backend_percent_window: i64,
}

impl Default for AccountUsageEstimatorConfig {
    fn default() -> Self {
        Self {
            min_usage_pct_sample_count: 1,
            max_usage_pct_display_percent_before_full: 0.0,
            stable_backend_percent_window: 5,
        }
    }
}

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
    pub sent_bytes: i64,
    pub recv_bytes: i64,
    pub sent_recv_bytes: i64,
    pub prewarm_sent_bytes: i64,
    pub prewarm_recv_bytes: i64,
    pub prewarm_sent_recv_bytes: i64,
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
    pub window_start_sent_bytes: Option<i64>,
    pub window_start_recv_bytes: Option<i64>,
    pub window_start_sent_recv_bytes: Option<i64>,
    pub window_start_prewarm_sent_bytes: Option<i64>,
    pub window_start_prewarm_recv_bytes: Option<i64>,
    pub window_start_prewarm_sent_recv_bytes: Option<i64>,
    pub last_backend_resets_at: Option<i64>,
    pub last_backend_window_minutes: Option<i64>,
    pub last_backend_seen_at: Option<i64>,
    pub backend_percent_history: Option<String>,
    pub cached_q_limit: Option<f64>,
    pub cached_q_limit_sample_count: Option<i64>,
    pub cached_q_limit_computed_at: Option<i64>,
    pub cached_q_limit_for_updated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AccountUsageEventMeta<'a> {
    pub query_id: Option<&'a str>,
    pub sent_bytes: Option<i64>,
    pub recv_bytes: Option<i64>,
    pub is_prewarm: bool,
}

#[derive(Clone)]
pub struct AccountUsageStore {
    sqlite_home: PathBuf,
    default_provider: String,
    estimator_config: AccountUsageEstimatorConfig,
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
    byte_weights: ByteWeights,
    composite_q_limit: Option<f64>,
    composite_q_bytes_limit: Option<f64>,
    composite_q_bytes_no_prewarm_limit: Option<f64>,
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
    prewarm_sent_bytes: i64,
    prewarm_recv_bytes: i64,
    prewarm_sent_recv_bytes: i64,
}

#[derive(Debug, Clone, Copy)]
struct ThresholdUsageCounts {
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    sent_bytes: i64,
    recv_bytes: i64,
    sent_bytes_including_warmups: i64,
    recv_bytes_including_warmups: i64,
}

// Composite usage model calibrated from local/backend usage logs.
const COMPOSITE_Q_INPUT_WEIGHT: f64 = 0.006;
const COMPOSITE_Q_CACHED_INPUT_WEIGHT: f64 = 0.003;
const DEFAULT_COMPOSITE_Q_SENT_BYTES_WEIGHT: f64 = 0.15;
const DEFAULT_COMPOSITE_Q_RECV_BYTES_WEIGHT: f64 = 0.85;
const BYTE_WEIGHT_FIT_STEP: f64 = 0.01;
const BYTE_WEIGHT_FIT_MIN_SAMPLES: usize = 3;

#[derive(Debug, Clone, Copy)]
struct ByteWeights {
    sent_weight: f64,
    recv_weight: f64,
}

impl ByteWeights {
    fn defaults() -> Self {
        Self {
            sent_weight: DEFAULT_COMPOSITE_Q_SENT_BYTES_WEIGHT,
            recv_weight: DEFAULT_COMPOSITE_Q_RECV_BYTES_WEIGHT,
        }
    }
}

impl AccountUsageStore {
    pub async fn init(sqlite_home: PathBuf, default_provider: String) -> anyhow::Result<Arc<Self>> {
        Self::init_with_estimator_config(
            sqlite_home,
            default_provider,
            AccountUsageEstimatorConfig::default(),
        )
        .await
    }

    pub async fn init_with_estimator_config(
        sqlite_home: PathBuf,
        default_provider: String,
        estimator_config: AccountUsageEstimatorConfig,
    ) -> anyhow::Result<Arc<Self>> {
        tokio::fs::create_dir_all(&sqlite_home).await?;
        let usage_path = usage_db_path(sqlite_home.as_path());
        let pool = open_sqlite(&usage_path).await?;
        Ok(Arc::new(Self {
            sqlite_home,
            default_provider,
            estimator_config,
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
    sent_bytes,
    recv_bytes,
    sent_recv_bytes,
    prewarm_sent_bytes,
    prewarm_recv_bytes,
    prewarm_sent_recv_bytes,
    prewarm_sent_bytes,
    prewarm_recv_bytes,
    prewarm_sent_recv_bytes,
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
    window_start_sent_bytes,
    window_start_recv_bytes,
    window_start_sent_recv_bytes,
    window_start_prewarm_sent_bytes,
    window_start_prewarm_recv_bytes,
    window_start_prewarm_sent_recv_bytes,
    window_start_context_total_tokens,
    window_start_min_total_cached_output_tokens,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at,
    backend_percent_history,
    cached_q_limit,
    cached_q_limit_sample_count,
    cached_q_limit_computed_at,
    cached_q_limit_for_updated_at
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
            sent_bytes: row.try_get("sent_bytes")?,
            recv_bytes: row.try_get("recv_bytes")?,
            sent_recv_bytes: row.try_get("sent_recv_bytes")?,
            prewarm_sent_bytes: row.try_get("prewarm_sent_bytes")?,
            prewarm_recv_bytes: row.try_get("prewarm_recv_bytes")?,
            prewarm_sent_recv_bytes: row.try_get("prewarm_sent_recv_bytes")?,
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
            window_start_sent_bytes: row.try_get("window_start_sent_bytes")?,
            window_start_recv_bytes: row.try_get("window_start_recv_bytes")?,
            window_start_sent_recv_bytes: row.try_get("window_start_sent_recv_bytes")?,
            window_start_prewarm_sent_bytes: row.try_get("window_start_prewarm_sent_bytes")?,
            window_start_prewarm_recv_bytes: row.try_get("window_start_prewarm_recv_bytes")?,
            window_start_prewarm_sent_recv_bytes: row
                .try_get("window_start_prewarm_sent_recv_bytes")?,
            last_backend_resets_at: row.try_get("last_backend_resets_at")?,
            last_backend_window_minutes: row.try_get("last_backend_window_minutes")?,
            last_backend_seen_at: row.try_get("last_backend_seen_at")?,
            backend_percent_history: row.try_get("backend_percent_history")?,
            cached_q_limit: row.try_get("cached_q_limit")?,
            cached_q_limit_sample_count: row.try_get("cached_q_limit_sample_count")?,
            cached_q_limit_computed_at: row.try_get("cached_q_limit_computed_at")?,
            cached_q_limit_for_updated_at: row.try_get("cached_q_limit_for_updated_at")?,
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

    pub async fn estimate_account_limit_tokens_q_cached(
        &self,
        account_id: &str,
        usage: &AccountUsageSnapshot,
    ) -> anyhow::Result<(Option<f64>, i64)> {
        if usage.cached_q_limit_for_updated_at == Some(usage.updated_at) {
            return Ok((usage.cached_q_limit, usage.cached_q_limit_sample_count.unwrap_or(0)));
        }

        let (cached_q_limit, cached_q_limit_sample_count) =
            self.estimate_account_limit_tokens_q(account_id).await?;
        let now = Utc::now().timestamp();
        sqlx::query(
            r#"
UPDATE account_usage
SET
    cached_q_limit = ?,
    cached_q_limit_sample_count = ?,
    cached_q_limit_computed_at = ?,
    cached_q_limit_for_updated_at = ?
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(cached_q_limit)
        .bind(cached_q_limit_sample_count)
        .bind(now)
        .bind(usage.updated_at)
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .execute(self.pool.as_ref())
        .await?;

        Ok((cached_q_limit, cached_q_limit_sample_count))
    }

    pub async fn estimate_account_limit_bytes_q(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(Option<f64>, i64)> {
        let estimates = self.estimate_account_limit_tokens_multi(account_id).await?;
        Ok((estimates.composite_q_bytes_limit, estimates.sample_count))
    }

    pub async fn estimate_account_limit_bytes_q_no_prewarm(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(Option<f64>, i64)> {
        let estimates = self.estimate_account_limit_tokens_multi(account_id).await?;
        Ok((
            estimates.composite_q_bytes_no_prewarm_limit,
            estimates.sample_count,
        ))
    }

    async fn estimate_account_limit_tokens_multi(
        &self,
        account_id: &str,
    ) -> anyhow::Result<AccountLimitEstimates> {
        let usage_row = sqlx::query(
            r#"
SELECT
    total_tokens,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    context_total_tokens,
    min_total_cached_output_tokens,
    sent_bytes,
    recv_bytes,
    sent_recv_bytes,
    prewarm_sent_bytes,
    prewarm_recv_bytes,
    prewarm_sent_recv_bytes,
    last_backend_used_percent,
    backend_percent_history
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .fetch_optional(self.pool.as_ref())
        .await?;
        let current_total_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("total_tokens").ok())
            .unwrap_or(0);
        let current_input_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("input_tokens").ok())
            .unwrap_or(0);
        let current_cached_input_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("cached_input_tokens").ok())
            .unwrap_or(0);
        let current_output_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("output_tokens").ok())
            .unwrap_or(0);
        let current_context_total_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("context_total_tokens").ok())
            .unwrap_or(0);
        let current_min_total_cached_output_tokens = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("min_total_cached_output_tokens").ok())
            .unwrap_or(0);
        let current_sent_bytes = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("sent_bytes").ok())
            .unwrap_or(0);
        let current_recv_bytes = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("recv_bytes").ok())
            .unwrap_or(0);
        let current_sent_recv_bytes = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("sent_recv_bytes").ok())
            .unwrap_or(0);
        let current_prewarm_sent_bytes = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("prewarm_sent_bytes").ok())
            .unwrap_or(0);
        let current_prewarm_recv_bytes = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("prewarm_recv_bytes").ok())
            .unwrap_or(0);
        let current_backend_used_percent = usage_row.as_ref().and_then(|row| {
            row.try_get::<Option<f64>, _>("last_backend_used_percent")
                .ok()
                .flatten()
        });
        let recent_backend_percents = usage_row
            .as_ref()
            .and_then(|row| row.try_get::<Option<String>, _>("backend_percent_history").ok())
            .flatten()
            .as_deref()
            .map(parse_backend_percent_history)
            .unwrap_or_default();
        let sample_count = recent_backend_percents.len() as i64;
        let smoothed_backend_percent = smooth_backend_used_percent(
            current_backend_used_percent,
            recent_backend_percents.as_slice(),
            self.estimator_config,
        );
        let byte_weights = if let Some(stabilized_percent) = smoothed_backend_percent {
            let byte_fit_samples = sqlx::query(
                r#"
SELECT
    delta_sent_bytes,
    delta_recv_bytes,
    delta_prewarm_sent_bytes,
    delta_prewarm_recv_bytes,
    delta_percent_int
FROM account_usage_samples
WHERE account_id = ? AND provider = ? AND delta_percent_int > 0
ORDER BY observed_at DESC, id DESC
LIMIT 200
                "#,
            )
            .bind(account_id)
            .bind(self.default_provider.as_str())
            .fetch_all(self.pool.as_ref())
            .await?;
            let byte_fit_samples = byte_fit_samples
                .into_iter()
                .filter_map(|row| {
                    let sent = row.try_get::<i64, _>("delta_sent_bytes").ok()?;
                    let recv = row.try_get::<i64, _>("delta_recv_bytes").ok()?;
                    let prewarm_sent = row.try_get::<i64, _>("delta_prewarm_sent_bytes").ok()?;
                    let prewarm_recv = row.try_get::<i64, _>("delta_prewarm_recv_bytes").ok()?;
                    let delta_percent = row.try_get::<i64, _>("delta_percent_int").ok()?;
                    (delta_percent > 0).then_some((
                        sent + prewarm_sent,
                        recv + prewarm_recv,
                        delta_percent,
                    ))
                })
                .collect::<Vec<_>>();
            fit_byte_weights(
                current_sent_bytes + current_prewarm_sent_bytes,
                current_recv_bytes + current_prewarm_recv_bytes,
                byte_fit_samples.as_slice(),
                stabilized_percent,
            )
        } else {
            None
        }
        .unwrap_or_else(ByteWeights::defaults);
        let cumulative_estimate =
            |tokens: f64| estimate_limit_from_running_totals(tokens, smoothed_backend_percent);

        Ok(AccountLimitEstimates {
            byte_weights,
            composite_q_limit: cumulative_estimate(composite_q_tokens(
                current_input_tokens,
                current_cached_input_tokens,
                current_output_tokens,
            )),
            composite_q_bytes_limit: cumulative_estimate(composite_q_bytes(
                current_sent_bytes + current_prewarm_sent_bytes,
                current_recv_bytes + current_prewarm_recv_bytes,
                byte_weights,
            )),
            composite_q_bytes_no_prewarm_limit: cumulative_estimate(composite_q_bytes(
                current_sent_bytes,
                current_recv_bytes,
                byte_weights,
            )),
            blended_limit: cumulative_estimate(current_total_tokens as f64),
            cached_input_limit: cumulative_estimate(current_cached_input_tokens as f64),
            output_limit: cumulative_estimate(current_output_tokens as f64),
            context_total_limit: cumulative_estimate(current_context_total_tokens as f64),
            min_total_cached_output_limit: cumulative_estimate(current_min_total_cached_output_tokens as f64),
            min_input_cached_output_limit: cumulative_estimate(
                min_input_cached_output_tokens(
                    current_input_tokens,
                    current_cached_input_tokens,
                    current_output_tokens,
                ) as f64,
            ),
            sent_limit: cumulative_estimate(current_sent_bytes as f64),
            recv_limit: cumulative_estimate(current_recv_bytes as f64),
            sent_recv_limit: cumulative_estimate(current_sent_recv_bytes as f64),
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
        let prewarm_sent = if meta.is_prewarm { sent } else { 0 };
        let prewarm_recv = if meta.is_prewarm { recv } else { 0 };
        let prewarm_sent_recv = prewarm_sent.saturating_add(prewarm_recv);
        let sent_without_prewarm = if meta.is_prewarm { 0 } else { sent };
        let recv_without_prewarm = if meta.is_prewarm { 0 } else { recv };
        let sent_recv_without_prewarm = sent_without_prewarm.saturating_add(recv_without_prewarm);
        if total_tokens == 0
            && input_tokens == 0
            && cached_input_tokens == 0
            && output_tokens == 0
            && reasoning_output_tokens == 0
            && sent == 0
            && recv == 0
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
    prewarm_sent_bytes,
    prewarm_recv_bytes,
    prewarm_sent_recv_bytes,
    updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    prewarm_sent_bytes = prewarm_sent_bytes + excluded.prewarm_sent_bytes,
    prewarm_recv_bytes = prewarm_recv_bytes + excluded.prewarm_recv_bytes,
    prewarm_sent_recv_bytes = prewarm_sent_recv_bytes + excluded.prewarm_sent_recv_bytes,
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
        .bind(sent_without_prewarm)
        .bind(recv_without_prewarm)
        .bind(sent_recv_without_prewarm)
        .bind(prewarm_sent)
        .bind(prewarm_recv)
        .bind(prewarm_sent_recv)
        .bind(updated_at)
        .execute(self.pool.as_ref())
        .await?;

        let query_id_suffix = meta
            .query_id
            .map(|value| format!(" query_id={value}"))
            .unwrap_or_default();
        let mut usage_fields = Vec::with_capacity(10);
        for (name, value) in [
            ("total", total_tokens),
            ("input", input_tokens),
            ("cached_input", cached_input_tokens),
            ("output", output_tokens),
            ("reasoning", reasoning_output_tokens),
            ("context_total", context_total_tokens),
            ("sent", sent_without_prewarm),
            ("recv", recv_without_prewarm),
            ("prewarm_sent", prewarm_sent),
            ("prewarm_recv", prewarm_recv),
        ] {
            if value != 0 {
                usage_fields.push(format!("{name}={value}"));
            }
        }
        let usage_message = if usage_fields.is_empty() {
            query_id_suffix.trim_start().to_string()
        } else {
            format!("{}{}", usage_fields.join(", "), query_id_suffix)
        };
        self.log_usage_event(
            account_id,
            /*used_percent*/ None,
            /*previous_percent*/ None,
            usage_message,
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
    last_snapshot_prewarm_sent_bytes,
    last_snapshot_prewarm_recv_bytes,
    last_snapshot_prewarm_sent_recv_bytes,
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
    window_start_prewarm_sent_bytes,
    window_start_prewarm_recv_bytes,
    window_start_prewarm_sent_recv_bytes,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at,
    backend_percent_history
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
        let backend_percent_changed = previous_backend_percent
            .is_none_or(|previous| (previous - used_percent).abs() > USED_PERCENT_REFUND_EPSILON);
        if backend_percent_changed {
            let delta_percent = previous_backend_percent.map_or(used_percent, |previous| used_percent - previous);
            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_backend_percent,
                format!("backend_percent_changed=1 delta_percent={delta_percent}"),
            )
            .await;
        }
        let previous_backend_percent_history = prior_usage
            .as_ref()
            .and_then(|row| row.try_get::<Option<String>, _>("backend_percent_history").ok())
            .flatten();
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
                previous_percent_for_jump,
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
                            byte_weights: ByteWeights::defaults(),
                            composite_q_limit: None,
                            composite_q_bytes_limit: None,
                            composite_q_bytes_no_prewarm_limit: None,
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
            let same_candidate_as_db_pending = prior_usage.as_ref().is_some_and(|row| {
                let previous_resets_at: Option<i64> = row.try_get("last_backend_resets_at").ok();
                let previous_window: Option<i64> = row.try_get("last_backend_window_minutes").ok();
                previous_window == window_minutes
                    && previous_resets_at == resets_at
                    && previous_backend_percent
                        .is_some_and(|previous| (previous - used_percent).abs() <= USED_PERCENT_REFUND_EPSILON)
            });
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
                self
                    .persist_backend_percent_history(
                        account_id,
                        previous_backend_percent_history.as_deref(),
                        used_percent,
                    )
                    .await?;
                // Suppress repeated "pending confirmations=1" logs when the backend
                // candidate is unchanged from what we already persisted.
                if confirmations > 1 || !same_candidate_as_db_pending {
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
                }
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
                previous_percent_for_jump,
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
            prewarm_sent_bytes_now,
            prewarm_recv_bytes_now,
            prewarm_sent_recv_bytes_now,
            last_sample_tokens,
            last_sample_input_tokens,
            last_sample_cached_input_tokens,
            last_sample_output_tokens,
            last_sample_context_total_tokens,
            last_sample_min_total_cached_output_tokens,
            last_sample_sent_bytes,
            last_sample_recv_bytes,
            last_sample_sent_recv_bytes,
            last_sample_prewarm_sent_bytes,
            last_sample_prewarm_recv_bytes,
            last_sample_prewarm_sent_recv_bytes,
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
            window_start_prewarm_sent_bytes,
            window_start_prewarm_recv_bytes,
            window_start_prewarm_sent_recv_bytes,
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
                row.try_get::<i64, _>("prewarm_sent_bytes").unwrap_or(0),
                row.try_get::<i64, _>("prewarm_recv_bytes").unwrap_or(0),
                row.try_get::<i64, _>("prewarm_sent_recv_bytes")
                    .unwrap_or(0),
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
                row.try_get::<i64, _>("last_snapshot_prewarm_sent_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_prewarm_recv_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("last_snapshot_prewarm_sent_recv_bytes")
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
                row.try_get::<i64, _>("window_start_prewarm_sent_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_prewarm_recv_bytes")
                    .unwrap_or(0),
                row.try_get::<i64, _>("window_start_prewarm_sent_recv_bytes")
                    .unwrap_or(0),
            )
        } else {
            (
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
                0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
                0_i64, 0_i64,
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
    prewarm_sent_bytes = 0,
    prewarm_recv_bytes = 0,
    prewarm_sent_recv_bytes = 0,
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
    last_snapshot_prewarm_sent_bytes = ?,
    last_snapshot_prewarm_recv_bytes = ?,
    last_snapshot_prewarm_sent_recv_bytes = ?,
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
    window_start_prewarm_sent_bytes = ?,
    window_start_prewarm_recv_bytes = ?,
    window_start_prewarm_sent_recv_bytes = ?,
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
            self
                .persist_backend_percent_history(
                    account_id,
                    previous_backend_percent_history.as_deref(),
                    used_percent,
                )
                .await?;

            self.log_usage_event(
                account_id,
                Some(used_percent),
                previous_percent_for_jump,
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
        let mut snapshot_prewarm_sent_bytes = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_prewarm_sent_bytes")
                    .ok()
            })
            .unwrap_or(0);
        let mut snapshot_prewarm_recv_bytes = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_prewarm_recv_bytes")
                    .ok()
            })
            .unwrap_or(0);
        let mut snapshot_prewarm_sent_recv_bytes = prior_usage
            .as_ref()
            .and_then(|row| {
                row.try_get::<i64, _>("last_snapshot_prewarm_sent_recv_bytes")
                    .ok()
            })
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
                prewarm_sent_bytes: (prewarm_sent_bytes_now - last_sample_prewarm_sent_bytes)
                    .max(0),
                prewarm_recv_bytes: (prewarm_recv_bytes_now - last_sample_prewarm_recv_bytes)
                    .max(0),
                prewarm_sent_recv_bytes: (prewarm_sent_recv_bytes_now
                    - last_sample_prewarm_sent_recv_bytes)
                    .max(0),
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
            let updated_window_start_prewarm_sent_bytes = window_start_prewarm_sent_bytes;
            let updated_window_start_prewarm_recv_bytes = window_start_prewarm_recv_bytes;
            let updated_window_start_prewarm_sent_recv_bytes = window_start_prewarm_sent_recv_bytes;
            let estimates = self
                .estimate_account_limit_tokens_multi(account_id)
                .await
                .unwrap_or(AccountLimitEstimates {
                    byte_weights: ByteWeights::defaults(),
                    composite_q_limit: None,
                    composite_q_bytes_limit: None,
                    composite_q_bytes_no_prewarm_limit: None,
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
                previous_percent_for_jump,
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
            snapshot_prewarm_sent_bytes = prewarm_sent_bytes_now;
            snapshot_prewarm_recv_bytes = prewarm_recv_bytes_now;
            snapshot_prewarm_sent_recv_bytes = prewarm_sent_recv_bytes_now;
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
    last_snapshot_prewarm_sent_bytes = ?,
    last_snapshot_prewarm_recv_bytes = ?,
    last_snapshot_prewarm_sent_recv_bytes = ?,
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
    window_start_prewarm_sent_bytes = ?,
    window_start_prewarm_recv_bytes = ?,
    window_start_prewarm_sent_recv_bytes = ?
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
            .bind(snapshot_prewarm_sent_bytes)
            .bind(snapshot_prewarm_recv_bytes)
            .bind(snapshot_prewarm_sent_recv_bytes)
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
            .bind(updated_window_start_prewarm_sent_bytes)
            .bind(updated_window_start_prewarm_recv_bytes)
            .bind(updated_window_start_prewarm_sent_recv_bytes)
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
            anchor_prewarm_sent_bytes,
            anchor_prewarm_recv_bytes,
            anchor_prewarm_sent_recv_bytes,
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
                prewarm_sent_bytes_now,
                prewarm_recv_bytes_now,
                prewarm_sent_recv_bytes_now,
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
                window_start_prewarm_sent_bytes,
                window_start_prewarm_recv_bytes,
                window_start_prewarm_sent_recv_bytes,
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
    prewarm_sent_bytes,
    prewarm_recv_bytes,
    prewarm_sent_recv_bytes,
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
    last_snapshot_prewarm_sent_bytes,
    last_snapshot_prewarm_recv_bytes,
    last_snapshot_prewarm_sent_recv_bytes,
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
    window_start_prewarm_sent_bytes,
    window_start_prewarm_recv_bytes,
    window_start_prewarm_sent_recv_bytes,
    last_backend_resets_at,
    last_backend_window_minutes,
    last_backend_seen_at
 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    last_snapshot_prewarm_sent_bytes = excluded.last_snapshot_prewarm_sent_bytes,
    last_snapshot_prewarm_recv_bytes = excluded.last_snapshot_prewarm_recv_bytes,
    last_snapshot_prewarm_sent_recv_bytes = excluded.last_snapshot_prewarm_sent_recv_bytes,
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
    window_start_prewarm_sent_bytes = excluded.window_start_prewarm_sent_bytes,
    window_start_prewarm_recv_bytes = excluded.window_start_prewarm_recv_bytes,
    window_start_prewarm_sent_recv_bytes = excluded.window_start_prewarm_sent_recv_bytes,
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
        .bind(snapshot_prewarm_sent_bytes)
        .bind(snapshot_prewarm_recv_bytes)
        .bind(snapshot_prewarm_sent_recv_bytes)
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
        .bind(anchor_prewarm_sent_bytes)
        .bind(anchor_prewarm_recv_bytes)
        .bind(anchor_prewarm_sent_recv_bytes)
        .bind(resets_at)
        .bind(window_minutes)
        .bind(seen_at)
        .execute(self.pool.as_ref())
        .await?;
        self
            .persist_backend_percent_history(
                account_id,
                previous_backend_percent_history.as_deref(),
                used_percent,
            )
            .await?;

        self.log_usage_limit_threshold_events(
            account_id,
            previous_backend_percent,
            used_percent,
            ThresholdUsageCounts {
                input_tokens: input_tokens_now,
                cached_input_tokens: cached_input_tokens_now,
                output_tokens: output_tokens_now,
                sent_bytes: sent_bytes_now.max(0),
                recv_bytes: recv_bytes_now.max(0),
                sent_bytes_including_warmups: (sent_bytes_now + prewarm_sent_bytes_now).max(0),
                recv_bytes_including_warmups: (recv_bytes_now + prewarm_recv_bytes_now).max(0),
            },
        )
        .await;

        Ok(())
    }

    pub async fn record_usage_limit_reached(&self, account_id: &str) -> anyhow::Result<()> {
        let threshold_state = sqlx::query(
            r#"
SELECT
    last_backend_used_percent,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    sent_bytes,
    recv_bytes,
    prewarm_sent_bytes,
    prewarm_recv_bytes
FROM account_usage
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .fetch_optional(self.pool.as_ref())
        .await?;
        let previous_percent = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<f64, _>("last_backend_used_percent").ok());
        let first_threshold_crossing = previous_percent.is_none_or(|value| !value.is_finite() || value < 101.0);
        if !first_threshold_crossing {
            return Ok(());
        }
        let now = Utc::now().timestamp();

        self.log_usage_event(
            account_id,
            Some(101.0),
            previous_percent,
            "usage_limit_reached=1 synthetic_used_percent=101".to_string(),
        )
        .await;
        let input_tokens = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("input_tokens").ok())
            .unwrap_or(0);
        let cached_input_tokens = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("cached_input_tokens").ok())
            .unwrap_or(0);
        let output_tokens = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("output_tokens").ok())
            .unwrap_or(0);
        let sent_bytes_now = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("sent_bytes").ok())
            .unwrap_or(0);
        let recv_bytes_now = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("recv_bytes").ok())
            .unwrap_or(0);
        let prewarm_sent_bytes = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("prewarm_sent_bytes").ok())
            .unwrap_or(0);
        let prewarm_recv_bytes = threshold_state
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("prewarm_recv_bytes").ok())
            .unwrap_or(0);
        self.log_usage_limit_threshold_events(
            account_id,
            previous_percent,
            101.0,
            ThresholdUsageCounts {
                input_tokens,
                cached_input_tokens,
                output_tokens,
                sent_bytes: sent_bytes_now.max(0),
                recv_bytes: recv_bytes_now.max(0),
                sent_bytes_including_warmups: (sent_bytes_now + prewarm_sent_bytes).max(0),
                recv_bytes_including_warmups: (recv_bytes_now + prewarm_recv_bytes).max(0),
            },
        )
        .await;
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
    prewarm_sent_bytes = 0,
    prewarm_recv_bytes = 0,
    prewarm_sent_recv_bytes = 0,
    updated_at = ?,
    last_backend_used_percent = ?,
    last_snapshot_total_tokens = 0,
    last_snapshot_input_tokens = 0,
    last_snapshot_cached_input_tokens = 0,
    last_snapshot_output_tokens = 0,
    last_snapshot_context_total_tokens = 0,
    last_snapshot_min_total_cached_output_tokens = 0,
    last_snapshot_sent_bytes = 0,
    last_snapshot_recv_bytes = 0,
    last_snapshot_sent_recv_bytes = 0,
    last_snapshot_prewarm_sent_bytes = 0,
    last_snapshot_prewarm_recv_bytes = 0,
    last_snapshot_prewarm_sent_recv_bytes = 0,
    last_snapshot_percent_int = 0,
    window_start_percent_int = 0,
    window_start_total_tokens = 0,
    window_start_input_tokens = 0,
    window_start_cached_input_tokens = 0,
    window_start_output_tokens = 0,
    window_start_context_total_tokens = 0,
    window_start_min_total_cached_output_tokens = 0,
    window_start_sent_bytes = 0,
    window_start_recv_bytes = 0,
    window_start_sent_recv_bytes = 0,
    window_start_prewarm_sent_bytes = 0,
    window_start_prewarm_recv_bytes = 0,
    window_start_prewarm_sent_recv_bytes = 0
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(now)
        .bind(101.0_f64)
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .execute(self.pool.as_ref())
        .await?;

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
            let backend_percent_history = sqlx::query(
                r#"
SELECT backend_percent_history
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
            .and_then(|row| row.try_get::<Option<String>, _>("backend_percent_history").ok())
            .flatten();
            Some(format_percent_display(
                previous_percent,
                used_percent,
                backend_percent_history.as_deref(),
                self.estimator_config,
            ))
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
    sent_recv_bytes,
    window_start_prewarm_sent_bytes,
    prewarm_sent_bytes,
    window_start_prewarm_recv_bytes,
    prewarm_recv_bytes
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
                let window_start_prewarm_sent_bytes = row
                    .try_get::<i64, _>("window_start_prewarm_sent_bytes")
                    .ok();
                let prewarm_sent_bytes_now = row.try_get::<i64, _>("prewarm_sent_bytes").ok();
                let window_start_prewarm_recv_bytes = row
                    .try_get::<i64, _>("window_start_prewarm_recv_bytes")
                    .ok();
                let prewarm_recv_bytes_now = row.try_get::<i64, _>("prewarm_recv_bytes").ok();
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
                    window_start_prewarm_sent_bytes,
                    prewarm_sent_bytes_now,
                    window_start_prewarm_recv_bytes,
                    prewarm_recv_bytes_now,
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
                Some(window_start_prewarm_sent_bytes),
                Some(prewarm_sent_bytes_now),
                Some(window_start_prewarm_recv_bytes),
                Some(prewarm_recv_bytes_now),
            )) = usage_row
            {
                let estimates = self
                    .estimate_account_limit_tokens_multi(account_id)
                    .await
                    .unwrap_or(AccountLimitEstimates {
                        byte_weights: ByteWeights::defaults(),
                        composite_q_limit: None,
                        composite_q_bytes_limit: None,
                        composite_q_bytes_no_prewarm_limit: None,
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
                if estimates.sample_count < self.estimator_config.min_usage_pct_sample_count {
                    String::new()
                } else {
                    let usage_pct_values = format_metric_values(
                        clamp_usage_pct_values(
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
                            estimates.composite_q_bytes_limit.and_then(|allowance| {
                                estimate_account_usage_percent_for_log_f64(
                                    composite_q_bytes(
                                        sent_bytes_now + prewarm_sent_bytes_now,
                                        recv_bytes_now + prewarm_recv_bytes_now,
                                        estimates.byte_weights,
                                    ),
                                    backend_anchor_percent,
                                    window_start_percent,
                                    composite_q_bytes(
                                        window_start_sent_bytes + window_start_prewarm_sent_bytes,
                                        window_start_recv_bytes + window_start_prewarm_recv_bytes,
                                        estimates.byte_weights,
                                    ),
                                    allowance,
                                )
                            }),
                            estimates
                                .composite_q_bytes_no_prewarm_limit
                                .and_then(|allowance| {
                                    estimate_account_usage_percent_for_log_f64(
                                        composite_q_bytes(
                                            sent_bytes_now,
                                            recv_bytes_now,
                                            estimates.byte_weights,
                                        ),
                                        backend_anchor_percent,
                                        window_start_percent,
                                        composite_q_bytes(
                                            window_start_sent_bytes,
                                            window_start_recv_bytes,
                                            estimates.byte_weights,
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
                            backend_anchor_percent,
                            self.estimator_config,
                        ),
                        /*precision*/ 2,
                    );
                    format!(" usage_pct[q/w/p/b/c/o/x/m/n/s/r/z]={usage_pct_values}%")
                }
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

    async fn persist_backend_percent_history(
        &self,
        account_id: &str,
        previous_history: Option<&str>,
        used_percent: f64,
    ) -> anyhow::Result<()> {
        let mut history = previous_history
            .map(parse_backend_percent_history)
            .unwrap_or_default();
        append_backend_percent_sample(&mut history, used_percent);
        let history_text = serialize_backend_percent_history(&history);
        sqlx::query(
            r#"
UPDATE account_usage
SET backend_percent_history = ?
WHERE account_id = ? AND provider = ?
            "#,
        )
        .bind(history_text)
        .bind(account_id)
        .bind(self.default_provider.as_str())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    async fn log_usage_limit_threshold_events(
        &self,
        account_id: &str,
        previous_percent: Option<f64>,
        current_percent: f64,
        counts: ThresholdUsageCounts,
    ) {
        let account_display = self.resolve_account_display(account_id).await;
        for (threshold, filename) in [
            (100.0, USAGE_LIMIT_100_LOG_FILENAME),
            (101.0, USAGE_LIMIT_101_LOG_FILENAME),
        ] {
            let crossed = if current_percent >= threshold {
                previous_percent.is_none_or(|value| !value.is_finite() || value < threshold)
            } else {
                false
            };
            if !crossed {
                continue;
            }
            if let Some(mut file) = open_usage_log_file_by_name(filename) {
                let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
                let _ = writeln!(
                    file,
                    "{ts} account={} input={} cached_input={} output={} recv_bytes={} sent_bytes={} recv_bytes_including_warmups={} sent_bytes_including_warmups={}",
                    account_display,
                    counts.input_tokens,
                    counts.cached_input_tokens,
                    counts.output_tokens,
                    counts.recv_bytes,
                    counts.sent_bytes,
                    counts.recv_bytes_including_warmups,
                    counts.sent_bytes_including_warmups
                );
            }
        }
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
    Some(usage_log_root_dir()?.join(usage_log_filename(account_display)))
}

fn open_usage_log_file(account_display: &str) -> Option<std::fs::File> {
    let path = usage_log_path(account_display)?;
    open_usage_log_file_path(path)
}

fn usage_named_log_path(filename: &str) -> Option<PathBuf> {
    Some(usage_log_root_dir()?.join(filename))
}

fn open_usage_log_file_by_name(filename: &str) -> Option<std::fs::File> {
    let path = usage_named_log_path(filename)?;
    open_usage_log_file_path(path)
}

fn open_usage_log_file_path(path: PathBuf) -> Option<std::fs::File> {
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn usage_log_root_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(USAGE_LOG_DIR_ENV_VAR).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    Some(dirs::home_dir()?.join(DEFAULT_CODEX_HOME_DIRNAME).join(USAGE_LOG_DIRNAME))
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

fn format_percent_display(
    previous_percent: Option<f64>,
    used_percent: Option<f64>,
    backend_percent_history: Option<&str>,
    estimator_config: AccountUsageEstimatorConfig,
) -> String {
    let used_percent_int = used_percent.map(|value| value.floor() as i64);
    let raw_display = match (previous_percent, used_percent_int) {
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
    };

    let history = backend_percent_history
        .map(parse_backend_percent_history)
        .unwrap_or_default();
    let stabilized_previous =
        stabilized_backend_percent(previous_percent, used_percent, &history, estimator_config);
    let stabilized_current =
        smooth_backend_used_percent(used_percent, history.as_slice(), estimator_config);

    match (stabilized_previous, stabilized_current) {
        (Some(previous), Some(current))
            if (current - previous).abs() > USED_PERCENT_REFUND_EPSILON =>
        {
            format!("{raw_display} stabilized_percent={previous:.2}->{current:.2}")
        }
        (_, Some(current)) => format!("{raw_display} stabilized_percent={current:.2}"),
        _ => raw_display,
    }
}

fn stabilized_backend_percent(
    previous_percent: Option<f64>,
    used_percent: Option<f64>,
    backend_percent_history: &[f64],
    estimator_config: AccountUsageEstimatorConfig,
) -> Option<f64> {
    let previous_percent = previous_percent?;
    let mut history = backend_percent_history.to_vec();
    if let Some(current_percent) = used_percent
        && let Some(last) = history.last().copied()
        && (last - current_percent).abs() <= USED_PERCENT_REFUND_EPSILON
        && (previous_percent - current_percent).abs() > USED_PERCENT_REFUND_EPSILON
    {
        history.pop();
    }
    smooth_backend_used_percent(Some(previous_percent), history.as_slice(), estimator_config)
}

fn parse_backend_percent_history(raw: &str) -> Vec<f64> {
    raw.split(',')
        .filter_map(|item| item.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect()
}

fn append_backend_percent_sample(history: &mut Vec<f64>, used_percent: f64) {
    if !used_percent.is_finite() || used_percent < 0.0 {
        return;
    }
    history.push(used_percent);
    let max_len = 200usize;
    if history.len() > max_len {
        let remove_count = history.len() - max_len;
        history.drain(0..remove_count);
    }
}

fn serialize_backend_percent_history(history: &[f64]) -> Option<String> {
    if history.is_empty() {
        None
    } else {
        Some(
            history
                .iter()
                .map(|value| format!("{value:.4}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

fn smooth_backend_used_percent(
    backend_percent: Option<f64>,
    recent_backend_percents: &[f64],
    estimator_config: AccountUsageEstimatorConfig,
) -> Option<f64> {
    let mut values = recent_backend_percents
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect::<Vec<_>>();
    if let Some(percent) = backend_percent {
        append_backend_percent_sample(&mut values, percent);
    }
    if values.is_empty() {
        return None;
    }
    let window_size = usize::try_from(estimator_config.stable_backend_percent_window.max(1))
        .unwrap_or(STABILIZED_BACKEND_MEDIAN_WINDOW_SAMPLES);
    let window_start = values.len().saturating_sub(window_size);
    let mut window = values[window_start..].to_vec();
    window.sort_by(f64::total_cmp);
    let mid = window.len() / 2;
    let percent = if window.len() % 2 == 0 {
        (window[mid - 1] + window[mid]) / 2.0
    } else {
        window[mid]
    };
    Some(percent)
}

fn estimate_limit_from_running_totals(tokens: f64, smoothed_percent: Option<f64>) -> Option<f64> {
    let percent = smoothed_percent?;
    if tokens <= 0.0 || !tokens.is_finite() || percent <= 0.0 || !percent.is_finite() {
        return None;
    }
    let estimated_limit = tokens * 100.0 / percent;
    if estimated_limit <= 0.0 || !estimated_limit.is_finite() {
        None
    } else {
        Some(estimated_limit)
    }
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

fn composite_q_bytes(sent_bytes: i64, recv_bytes: i64, weights: ByteWeights) -> f64 {
    weights.sent_weight * sent_bytes.max(0) as f64 + weights.recv_weight * recv_bytes.max(0) as f64
}

fn fit_byte_weights(
    total_sent_bytes: i64,
    total_recv_bytes: i64,
    samples: &[(i64, i64, i64)],
    stabilized_backend_percent: f64,
) -> Option<ByteWeights> {
    if samples.len() < BYTE_WEIGHT_FIT_MIN_SAMPLES
        || !stabilized_backend_percent.is_finite()
        || stabilized_backend_percent <= 0.0
    {
        return None;
    }
    let total_sent = total_sent_bytes.max(0) as f64;
    let total_recv = total_recv_bytes.max(0) as f64;
    if total_sent <= 0.0 && total_recv <= 0.0 {
        return None;
    }

    let mut best: Option<(f64, f64)> = None;
    for step in 0..=100 {
        let sent_weight = step as f64 * BYTE_WEIGHT_FIT_STEP;
        let recv_weight = 1.0 - sent_weight;
        let total_weighted = sent_weight * total_sent + recv_weight * total_recv;
        let Some(estimated_limit) =
            estimate_limit_from_running_totals(total_weighted, Some(stabilized_backend_percent))
        else {
            continue;
        };

        let mut score = 0.0;
        let mut included = 0usize;
        for (sample_sent, sample_recv, delta_percent_int) in samples {
            if *delta_percent_int <= 0 {
                continue;
            }
            let sample_weighted =
                sent_weight * (*sample_sent).max(0) as f64 + recv_weight * (*sample_recv).max(0) as f64;
            if sample_weighted <= 0.0 {
                continue;
            }
            let predicted_delta_percent = sample_weighted * 100.0 / estimated_limit;
            let observed_delta_percent = *delta_percent_int as f64;
            let error = predicted_delta_percent - observed_delta_percent;
            score += error * error;
            included += 1;
        }
        if included >= BYTE_WEIGHT_FIT_MIN_SAMPLES {
            let mean_score = score / included as f64;
            if best.is_none_or(|(best_score, _)| mean_score < best_score) {
                best = Some((mean_score, sent_weight));
            }
        }
    }

    let fitted_sent_weight = best.map(|(_, weight)| weight)?;
    Some(ByteWeights {
        sent_weight: fitted_sent_weight,
        recv_weight: 1.0 - fitted_sent_weight,
    })
}

fn format_account_limit_estimates(estimates: &AccountLimitEstimates) -> String {
    let avg = format_metric_values_si([
        estimates.composite_q_limit.map(|value| value / 100.0),
        estimates.composite_q_bytes_limit.map(|value| value / 100.0),
        estimates
            .composite_q_bytes_no_prewarm_limit
            .map(|value| value / 100.0),
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
        estimates.composite_q_bytes_limit,
        estimates.composite_q_bytes_no_prewarm_limit,
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
    format!(
        "avg_tpp={avg} est_allow={allowance} byte_weights={:.2}/{:.2}",
        estimates.byte_weights.sent_weight,
        estimates.byte_weights.recv_weight
    )
}

fn format_metric_values_si(values: [Option<f64>; 12]) -> String {
    let value = |entry: Option<f64>| match entry {
        Some(number) if number.is_finite() && number >= 0.0 => format_si_three_digits(number),
        _ => "-".to_string(),
    };
    values.into_iter().map(value).collect::<Vec<_>>().join("/")
}

fn format_metric_values(values: [Option<f64>; 12], precision: usize) -> String {
    let value = |entry: Option<f64>| match entry {
        Some(number) if number.is_finite() && number >= 0.0 => format!("{number:.precision$}"),
        _ => "-".to_string(),
    };
    values.into_iter().map(value).collect::<Vec<_>>().join("/")
}

fn clamp_usage_pct_values(
    values: [Option<f64>; 12],
    backend_anchor_percent: f64,
    estimator_config: AccountUsageEstimatorConfig,
) -> [Option<f64>; 12] {
    let configured_cap = estimator_config.max_usage_pct_display_percent_before_full;
    let max_percent = if backend_anchor_percent < 100.0
        && configured_cap.is_finite()
        && configured_cap > 0.0
    {
        Some(configured_cap)
    } else {
        None
    };
    values.map(|entry| {
        entry.and_then(|value| {
            if value.is_finite() {
                let clamped = value.max(0.0);
                Some(if let Some(max_percent) = max_percent {
                    clamped.min(max_percent)
                } else {
                    clamped
                })
            } else {
                None
            }
        })
    })
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
    delta_prewarm_sent_bytes,
    delta_prewarm_recv_bytes,
    delta_prewarm_sent_recv_bytes,
    window_minutes,
    resets_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(deltas.prewarm_sent_bytes)
    .bind(deltas.prewarm_recv_bytes)
    .bind(deltas.prewarm_sent_recv_bytes)
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
    async fn prewarm_network_bytes_do_not_increment_default_sent_recv_counters() {
        let home = tempfile::tempdir().expect("tempdir");
        let runtime = AccountUsageStore::init(home.path().to_path_buf(), "test-provider".to_string())
            .await
            .expect("init");

        let usage = TokenUsage {
            total_tokens: 0,
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
        };
        runtime
            .record_account_token_usage(
                "account-1",
                &usage,
                AccountUsageEventMeta {
                    sent_bytes: Some(11),
                    recv_bytes: Some(29),
                    is_prewarm: true,
                    ..AccountUsageEventMeta::default()
                },
            )
            .await
            .expect("record prewarm usage");
        runtime
            .record_account_token_usage(
                "account-1",
                &usage,
                AccountUsageEventMeta {
                    sent_bytes: Some(7),
                    recv_bytes: Some(5),
                    is_prewarm: false,
                    ..AccountUsageEventMeta::default()
                },
            )
            .await
            .expect("record regular usage");

        let snapshot = runtime
            .get_account_usage("account-1")
            .await
            .expect("read")
            .expect("row");
        assert_eq!(snapshot.sent_bytes, 7);
        assert_eq!(snapshot.recv_bytes, 5);
        assert_eq!(snapshot.sent_recv_bytes, 12);
        assert_eq!(snapshot.prewarm_sent_bytes, 11);
        assert_eq!(snapshot.prewarm_recv_bytes, 29);
        assert_eq!(snapshot.prewarm_sent_recv_bytes, 40);
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
    fn fit_byte_weights_refits_sent_recv_mix() {
        let samples = vec![
            (1000, 200, 8),
            (400, 900, 6),
            (1500, 100, 11),
            (300, 1200, 6),
            (1100, 600, 9),
        ];
        let weights = fit_byte_weights(
            /*total_sent_bytes*/ 3_200,
            /*total_recv_bytes*/ 860,
            samples.as_slice(),
            /*stabilized_backend_percent*/ 25.0,
        )
        .expect("fit byte weights");
        assert!(weights.sent_weight > 0.60);
        assert!(weights.sent_weight < 0.80);
        assert!((weights.sent_weight + weights.recv_weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn fit_byte_weights_requires_enough_samples() {
        let samples = vec![(100, 200, 1), (200, 100, 1)];
        let weights = fit_byte_weights(
            /*total_sent_bytes*/ 1_000,
            /*total_recv_bytes*/ 1_000,
            samples.as_slice(),
            /*stabilized_backend_percent*/ 20.0,
        );
        assert!(weights.is_none());
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
