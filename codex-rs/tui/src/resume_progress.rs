//! Resume startup progress estimated from prior elapsed durations.

use codex_app_server_protocol::ThreadHistoryMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const ESTIMATE_FILE_NAME: &str = "resume_progress_estimates.json";
const MAX_SAMPLES: usize = 64;
const MIN_ESTIMATE: Duration = Duration::from_secs(2);
const MAX_ESTIMATE: Duration = Duration::from_secs(600);
const DEFAULT_PRE_PROMPT_ESTIMATE: Duration = Duration::from_secs(2);
const DEFAULT_BYTES_PER_SECOND: u64 = 8 * 1024 * 1024;
const FIXED_OVERHEAD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum HistoryKind {
    Legacy,
    Paginated,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum CompressionKind {
    Plain,
    Compressed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Workload {
    size_bytes: u64,
    history: HistoryKind,
    compression: CompressionKind,
}

impl Workload {
    fn from_rollout(rollout_path: Option<&Path>, history_mode: Option<ThreadHistoryMode>) -> Self {
        let size_bytes = rollout_path
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |metadata| metadata.len());
        let compression = rollout_path.map_or(CompressionKind::Unknown, |path| {
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("gz" | "zst" | "zstd") => CompressionKind::Compressed,
                Some("jsonl") => CompressionKind::Plain,
                _ => CompressionKind::Unknown,
            }
        });
        let history = match history_mode {
            Some(ThreadHistoryMode::Legacy) => HistoryKind::Legacy,
            Some(ThreadHistoryMode::Paginated) => HistoryKind::Paginated,
            None => HistoryKind::Unknown,
        };
        Self {
            size_bytes,
            history,
            compression,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Sample {
    workload: Workload,
    duration_ms: u64,
    #[serde(default)]
    pre_prompt_duration_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct EstimateHistory {
    version: u8,
    samples: Vec<Sample>,
}

/// Stores only rollout size/format and elapsed times. Rollout paths and contents are never stored.
#[derive(Clone)]
pub(crate) struct ResumeProgressEstimate {
    codex_home: AbsolutePathBuf,
    workload: Workload,
    expected_duration: Duration,
    pre_prompt_expected_duration: Duration,
}

impl ResumeProgressEstimate {
    pub(crate) fn load(
        codex_home: AbsolutePathBuf,
        rollout_path: Option<&Path>,
        history_mode: Option<ThreadHistoryMode>,
    ) -> Self {
        let workload = Workload::from_rollout(rollout_path, history_mode);
        let history = Self::read_history(&codex_home);
        let expected_duration = estimate_duration(workload, &history.samples);
        let pre_prompt_expected_duration = estimate_pre_prompt_duration(workload, &history.samples);
        Self {
            codex_home,
            workload,
            expected_duration,
            pre_prompt_expected_duration,
        }
    }

    pub(crate) fn progress_percentage(&self, elapsed: Duration) -> u8 {
        progress_percentage(elapsed, self.expected_duration)
    }

    pub(crate) fn pre_prompt_progress_percentage(&self, elapsed: Duration) -> u8 {
        progress_percentage(elapsed, self.pre_prompt_expected_duration)
    }

    pub(crate) fn record_completion(
        &self,
        pre_prompt_duration: Option<Duration>,
        duration: Duration,
    ) {
        if duration < Duration::from_millis(250) {
            return;
        }
        let mut history = Self::read_history(&self.codex_home);
        history.version = 2;
        history.samples.push(Sample {
            workload: self.workload,
            duration_ms: duration.as_millis().min(u128::from(u64::MAX)) as u64,
            pre_prompt_duration_ms: pre_prompt_duration
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64),
        });
        let retained = history.samples.len().saturating_sub(MAX_SAMPLES);
        history.samples.drain(..retained);
        let path = self.codex_home.join(ESTIMATE_FILE_NAME);
        match serde_json::to_string(&history) {
            Ok(contents) => {
                if let Err(error) = codex_utils_path::write_atomically(path.as_path(), &contents) {
                    tracing::warn!(%error, "failed to save resume progress estimate");
                }
            }
            Err(error) => tracing::warn!(%error, "failed to serialize resume progress estimate"),
        }
    }

    fn read_history(codex_home: &AbsolutePathBuf) -> EstimateHistory {
        let path = codex_home.join(ESTIMATE_FILE_NAME);
        std::fs::read(path.as_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|history: &EstimateHistory| history.version == 2)
            .unwrap_or_default()
    }
}

fn estimate_pre_prompt_duration(workload: Workload, samples: &[Sample]) -> Duration {
    let mut durations = samples
        .iter()
        .filter(|sample| {
            sample.workload.history == workload.history
                && sample.workload.compression == workload.compression
        })
        .filter_map(|sample| sample.pre_prompt_duration_ms)
        .filter(|duration| *duration > 0)
        .map(Duration::from_millis)
        .collect::<Vec<_>>();
    if durations.is_empty() {
        return DEFAULT_PRE_PROMPT_ESTIMATE;
    }
    durations.sort_unstable();
    durations[durations.len() / 2].clamp(MIN_ESTIMATE, MAX_ESTIMATE)
}

fn estimate_duration(workload: Workload, samples: &[Sample]) -> Duration {
    let matching = samples
        .iter()
        .filter(|sample| {
            sample.workload.history == workload.history
                && sample.workload.compression == workload.compression
        })
        .collect::<Vec<_>>();
    let mut rates = matching
        .iter()
        .filter(|sample| sample.workload.size_bytes > 0)
        .map(|sample| {
            let measured = Duration::from_millis(sample.duration_ms)
                .saturating_sub(FIXED_OVERHEAD)
                .as_secs_f64();
            measured / sample.workload.size_bytes as f64
        })
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .collect::<Vec<_>>();

    let estimate = if workload.size_bytes > 0 && !rates.is_empty() {
        rates.sort_by(f64::total_cmp);
        let median_rate = rates[rates.len() / 2];
        let seconds =
            (median_rate * workload.size_bytes as f64).clamp(0.0, MAX_ESTIMATE.as_secs_f64());
        FIXED_OVERHEAD + Duration::from_secs_f64(seconds)
    } else if let Some(sample) = matching.last() {
        Duration::from_millis(sample.duration_ms)
    } else if workload.size_bytes > 0 {
        FIXED_OVERHEAD
            + Duration::from_secs_f64(workload.size_bytes as f64 / DEFAULT_BYTES_PER_SECOND as f64)
    } else {
        Duration::from_secs(5)
    };
    estimate.clamp(MIN_ESTIMATE, MAX_ESTIMATE)
}

fn progress_percentage(elapsed: Duration, expected: Duration) -> u8 {
    let expected_millis = expected.as_millis().max(1);
    let elapsed_millis = elapsed.as_millis();
    (elapsed_millis.saturating_mul(100) / expected_millis).min(99) as u8
}

#[cfg(test)]
#[path = "resume_progress_tests.rs"]
mod tests;
