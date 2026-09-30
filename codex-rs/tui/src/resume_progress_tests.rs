use super::CompressionKind;
use super::EstimateHistory;
use super::HistoryKind;
use super::ResumeProgressEstimate;
use super::Sample;
use super::Workload;
use super::estimate_duration;
use super::estimate_pre_prompt_duration;
use super::progress_percentage;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tempfile::TempDir;

fn workload(size_bytes: u64) -> Workload {
    Workload {
        size_bytes,
        history: HistoryKind::Paginated,
        compression: CompressionKind::Plain,
    }
}

#[test]
fn progress_advances_linearly_and_reserves_completion() {
    let expected = Duration::from_secs(12);

    assert_eq!(progress_percentage(Duration::ZERO, expected), 0);
    assert_eq!(progress_percentage(Duration::from_secs(3), expected), 25);
    assert_eq!(progress_percentage(Duration::from_secs(6), expected), 50);
    assert_eq!(progress_percentage(expected, expected), 99);
    assert_eq!(progress_percentage(Duration::from_secs(30), expected), 99);
}

#[test]
fn learned_rollout_throughput_scales_estimate_to_file_size() {
    let history = EstimateHistory {
        version: 2,
        samples: vec![Sample {
            workload: workload(10 * 1024 * 1024),
            duration_ms: 10_500,
            pre_prompt_duration_ms: None,
        }],
    };

    assert_eq!(
        estimate_duration(workload(5 * 1024 * 1024), &history.samples),
        Duration::from_millis(5_500)
    );
}

#[test]
fn unrelated_rollout_formats_do_not_train_each_other() {
    let history = EstimateHistory {
        version: 2,
        samples: vec![Sample {
            workload: Workload {
                size_bytes: 10 * 1024 * 1024,
                history: HistoryKind::Legacy,
                compression: CompressionKind::Compressed,
            },
            duration_ms: 60_500,
            pre_prompt_duration_ms: None,
        }],
    };

    let estimate = estimate_duration(workload(1024 * 1024), &history.samples);
    assert!(estimate < Duration::from_secs(5));
}

#[test]
fn fallback_estimate_uses_size_and_is_bounded() {
    let estimate = estimate_duration(workload(8 * 1024 * 1024), &[]);

    assert_eq!(estimate, Duration::from_secs(2));
    assert_eq!(
        estimate_duration(workload(u64::MAX), &[]),
        Duration::from_secs(600)
    );
}

#[test]
fn pre_prompt_estimate_uses_only_matching_resume_samples() {
    let history = EstimateHistory {
        version: 2,
        samples: vec![
            Sample {
                workload: workload(10 * 1024 * 1024),
                duration_ms: 10_500,
                pre_prompt_duration_ms: Some(4_500),
            },
            Sample {
                workload: Workload {
                    history: HistoryKind::Legacy,
                    ..workload(10 * 1024 * 1024)
                },
                duration_ms: 20_000,
                pre_prompt_duration_ms: Some(20_000),
            },
        ],
    };

    assert_eq!(
        estimate_pre_prompt_duration(workload(1024), &history.samples),
        Duration::from_millis(4_500)
    );
    assert_eq!(
        estimate_pre_prompt_duration(workload(1024), &[]),
        Duration::from_secs(2)
    );
}

#[test]
fn completed_resume_learns_numeric_metrics_without_storing_paths() {
    let home = TempDir::new().expect("create temporary Codex home");
    let rollout = home.path().join("private-session.jsonl");
    std::fs::write(&rollout, vec![b'x'; 1024 * 1024]).expect("write rollout fixture");
    let codex_home = AbsolutePathBuf::try_from(home.path().to_path_buf())
        .expect("temporary Codex home is absolute");

    let estimate = ResumeProgressEstimate::load(
        codex_home.clone(),
        Some(rollout.as_path()),
        Some(ThreadHistoryMode::Legacy),
    );
    estimate.record_completion(Some(Duration::from_secs(4)), Duration::from_secs(8));

    let saved = std::fs::read_to_string(codex_home.join("resume_progress_estimates.json"))
        .expect("read saved estimate");
    assert!(!saved.contains("private-session"));
    assert!(!saved.contains(home.path().to_str().expect("UTF-8 temp path")));
    let saved: EstimateHistory = serde_json::from_str(&saved).expect("parse saved estimate");
    assert_eq!(saved.samples[0].duration_ms, 8_000);
    assert_eq!(saved.samples[0].pre_prompt_duration_ms, Some(4_000));
    let learned = ResumeProgressEstimate::load(
        codex_home,
        Some(rollout.as_path()),
        Some(ThreadHistoryMode::Legacy),
    );
    assert!(learned.expected_duration >= Duration::from_millis(7_999));
    assert!(learned.expected_duration <= Duration::from_millis(8_001));
    assert_eq!(learned.progress_percentage(Duration::from_secs(4)), 50);
    assert_eq!(
        learned.pre_prompt_progress_percentage(Duration::from_secs(2)),
        50
    );
}
