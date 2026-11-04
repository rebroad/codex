//! Selects a smaller resume context for legacy subagent rollouts.
//!
//! Legacy subagents copied the parent's full rollout into every child rollout. Migrating those
//! files verbatim can preserve gigabytes of duplicated history even though a resumed subagent only
//! needs its latest bounded model context.
//!
//! This module reverse-scans legacy rollouts for the newest compaction with replacement history
//! and a window number. Malformed records are skipped. Cold resume keeps the full parsed history
//! when no safe checkpoint exists; subagent migration retains its full tolerant replay fallback.

use std::path::PathBuf;

use codex_rollout::ModelContextScan;
use codex_rollout::ModelContextScanProgress;
use codex_rollout::ReverseJsonlScanner;
use codex_rollout::RolloutItem;
use codex_rollout::ScanOutcome;
use codex_rollout::open_rollout_seekable_reader;
use serde_json::Value;

use super::line_parser;
use super::migration_error;
use crate::ThreadReadProgressCallback;
use crate::ThreadStoreResult;

/// Returns the suffix starting at the newest usable compaction, if one exists.
pub(super) async fn select_bounded_context(
    rollout_path: PathBuf,
) -> ThreadStoreResult<Option<Vec<RolloutItem>>> {
    let (items, found_checkpoint) =
        scan_context(rollout_path, None, Some(super::MAX_ROLLOUT_LINE_BYTES)).await?;
    Ok(found_checkpoint.then_some(items))
}

pub(super) async fn load_model_context(
    rollout_path: PathBuf,
    progress: Option<ThreadReadProgressCallback>,
) -> ThreadStoreResult<Vec<RolloutItem>> {
    let (items, _) = scan_context(rollout_path, progress, None).await?;
    Ok(items)
}

async fn scan_context(
    rollout_path: PathBuf,
    progress: Option<ThreadReadProgressCallback>,
    max_record_bytes: Option<usize>,
) -> ThreadStoreResult<(Vec<RolloutItem>, bool)> {
    tokio::task::spawn_blocking(move || {
        let file = open_rollout_seekable_reader(&rollout_path).map_err(migration_error)?;
        let total_bytes = file.metadata().map_err(migration_error)?.len();
        let mut scanner = ReverseJsonlScanner::new(file).map_err(migration_error)?;
        if let Some(max_record_bytes) = max_record_bytes {
            scanner = scanner.with_max_record_bytes(max_record_bytes);
        }
        let mut scan = ModelContextScan::default();
        let mut reported_progress = 0_u8;
        let mut found_checkpoint = false;

        while let Some(outcome) = scanner.scan_next::<Value>().map_err(migration_error)? {
            if let Some(progress) = &progress
                && total_bytes > 0
            {
                let percentage =
                    (scanner.bytes_scanned().saturating_mul(100) / total_bytes).min(99) as u8;
                if percentage > reported_progress {
                    reported_progress = percentage;
                    progress(percentage);
                }
            }
            let value = match outcome {
                ScanOutcome::Parsed(value) => value,
                ScanOutcome::Rejected(_) => continue,
            };
            let Ok(Some(line)) = line_parser::parse_legacy_rollout_value(value) else {
                continue;
            };
            if scan.push(line.item) == ModelContextScanProgress::Complete {
                found_checkpoint = true;
                break;
            }
        }

        let mut items = scan.finish();
        items.retain(|item| !matches!(item, RolloutItem::SessionMeta(_)));
        if let Some(progress) = progress
            && total_bytes > 0
        {
            progress(100);
        }
        Ok((items, found_checkpoint))
    })
    .await
    .map_err(migration_error)?
}
