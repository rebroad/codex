//! Progress timelines learned from prior rollout decoding markers.

use codex_app_server_protocol::ThreadHistoryMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

const ESTIMATE_FILE_NAME: &str = "resume_progress_estimates.json";
const MAX_SAMPLES: usize = 64;
const MIN_ESTIMATE: Duration = Duration::from_secs(2);
const MAX_ESTIMATE: Duration = Duration::from_secs(600);
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
    markers: Vec<ProgressMarker>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProgressMarker {
    progress: u8,
    elapsed_ms: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct EstimateHistory {
    version: u8,
    samples: Vec<Sample>,
}

/// Stores only rollout size/format, decoded progress markers, and elapsed times.
/// Rollout paths and contents are never stored.
#[derive(Clone)]
pub(crate) struct ResumeProgressEstimate {
    codex_home: AbsolutePathBuf,
    workload: Workload,
    expected_duration: Duration,
    history: Vec<Sample>,
    current_markers: Arc<Mutex<Vec<ProgressMarker>>>,
}

#[derive(Clone)]
pub(crate) struct ResumeProgressObserver {
    started_at: Instant,
    markers: Arc<Mutex<Vec<ProgressMarker>>>,
}

impl ResumeProgressObserver {
    pub(crate) fn record(&self, progress: u8) {
        let marker = ProgressMarker {
            progress: progress.min(100),
            elapsed_ms: self
                .started_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        };
        let Ok(mut markers) = self.markers.lock() else {
            return;
        };
        if markers
            .last()
            .is_none_or(|previous| marker.progress > previous.progress)
        {
            markers.push(marker);
        }
    }
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
        Self {
            codex_home,
            workload,
            expected_duration,
            history: history.samples,
            current_markers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn observer(&self, started_at: Instant) -> ResumeProgressObserver {
        ResumeProgressObserver {
            started_at,
            markers: Arc::clone(&self.current_markers),
        }
    }

    pub(crate) fn progress_percentage(&self, elapsed: Duration) -> u8 {
        let sample = self.history.iter().rev().find(|sample| {
            sample.workload.history == self.workload.history
                && sample.workload.compression == self.workload.compression
                && !sample.markers.is_empty()
        });
        let Ok(current_markers) = self.current_markers.lock() else {
            return 0;
        };
        let Some(sample) = sample else {
            return current_markers
                .last()
                .map_or(0, |marker| marker.progress.min(99));
        };
        progress_from_sample(
            elapsed,
            self.expected_duration,
            sample,
            current_markers.last().copied(),
        )
    }

    pub(crate) fn record_completion(&self, duration: Duration) {
        if duration < Duration::from_millis(250) {
            return;
        }
        let mut history = Self::read_history(&self.codex_home);
        history.version = 1;
        history.samples.push(Sample {
            workload: self.workload,
            duration_ms: duration.as_millis().min(u128::from(u64::MAX)) as u64,
            markers: self
                .current_markers
                .lock()
                .map(|markers| markers.clone())
                .unwrap_or_default(),
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
            .filter(|history: &EstimateHistory| history.version == 1)
            .unwrap_or_default()
    }
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

fn progress_from_sample(
    elapsed: Duration,
    expected: Duration,
    sample: &Sample,
    current_marker: Option<ProgressMarker>,
) -> u8 {
    let sample_duration = sample.duration_ms.max(1) as f64;
    let expected_ms = expected.as_millis() as f64;
    let scale = expected_ms / sample_duration;
    let mut points = vec![(0.0, 0.0)];
    for marker in &sample.markers {
        let progress = marker.progress.min(99) as f64;
        let time_ms = marker.elapsed_ms as f64 * scale;
        if progress > points.last().map_or(0.0, |point| point.1)
            && time_ms > points.last().map_or(0.0, |point| point.0)
        {
            points.push((time_ms, progress));
        }
    }
    if points.last().is_some_and(|point| point.1 < 100.0) {
        points.push((expected_ms, 100.0));
    }

    let current_elapsed_ms = elapsed.as_millis() as f64;
    if let Some(marker) = current_marker {
        let current_progress = marker.progress.min(99) as f64;
        if current_progress > 0.0 {
            let predicted_ms = interpolate_time(&points, current_progress);
            if predicted_ms > 0.0 {
                let adjustment = current_elapsed_ms / predicted_ms;
                for point in &mut points {
                    point.0 *= adjustment;
                }
            }
        }
    }
    interpolate_progress(&points, current_elapsed_ms)
        .floor()
        .clamp(0.0, 99.0) as u8
}

fn interpolate_time(points: &[(f64, f64)], progress: f64) -> f64 {
    for pair in points.windows(2) {
        let [(start_time, start_progress), (end_time, end_progress)] = pair else {
            continue;
        };
        if progress <= *end_progress {
            let progress_delta = end_progress - start_progress;
            if progress_delta <= 0.0 {
                return *end_time;
            }
            return start_time
                + (end_time - start_time) * (progress - start_progress) / progress_delta;
        }
    }
    points.last().map_or(0.0, |point| point.0)
}

fn interpolate_progress(points: &[(f64, f64)], elapsed_ms: f64) -> f64 {
    for pair in points.windows(2) {
        let [(start_time, start_progress), (end_time, end_progress)] = pair else {
            continue;
        };
        if elapsed_ms <= *end_time {
            let time_delta = end_time - start_time;
            if time_delta <= 0.0 {
                return *end_progress;
            }
            let fraction = (elapsed_ms - start_time) / time_delta;
            return start_progress + (end_progress - start_progress) * fraction;
        }
    }
    points.last().map_or(0.0, |point| point.1)
}

#[cfg(test)]
#[path = "resume_progress_tests.rs"]
mod tests;
