//! Reconstructs model context and preserves source runtime metadata across fork cutoffs.

use std::io;
use std::path::Path;

use codex_protocol::protocol::HistoryPosition;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_rollout::ModelContextScan;
use codex_rollout::ModelContextScanProgress;
use codex_rollout::ReverseJsonlScanner;
use codex_rollout::RolloutItem;
use codex_rollout::ScanOutcome;

use super::LocalThreadStore;
use super::read_thread;
use super::rollout_lineage::RolloutLineage;
use super::thread_rollout_resolver;
use crate::LoadThreadHistoryParams;
use crate::StoredModelContext;
use crate::ThreadReadProgressCallback;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

#[cfg(test)]
#[path = "model_context_tests.rs"]
mod tests;

/// Loads rollout items needed to reconstruct the latest model-visible context.
///
/// Paginated JSONL rollouts use a reverse scan. It stops at the newest `CompactedItem` with both
/// replacement history and a window number, and returns that compaction plus its newer suffix. If
/// the newest compaction lacks either field, the scan continues to the beginning of the rollout.
///
/// Compressed segments are decoded before applying their original JSONL offsets. Legacy rollouts
/// keep the existing full-history path.
pub(super) async fn load_latest_model_context(
    store: &LocalThreadStore,
    params: LoadThreadHistoryParams,
) -> ThreadStoreResult<StoredModelContext> {
    load_latest_model_context_with_optional_progress(store, params, None).await
}

pub(super) async fn load_latest_model_context_with_progress(
    store: &LocalThreadStore,
    params: LoadThreadHistoryParams,
    progress: ThreadReadProgressCallback,
) -> ThreadStoreResult<StoredModelContext> {
    load_latest_model_context_with_optional_progress(store, params, Some(progress)).await
}

async fn load_latest_model_context_with_optional_progress(
    store: &LocalThreadStore,
    params: LoadThreadHistoryParams,
    progress: Option<ThreadReadProgressCallback>,
) -> ThreadStoreResult<StoredModelContext> {
    let resolved = if params.include_archived {
        thread_rollout_resolver::resolve_current_including_archived(store, params.thread_id).await?
    } else {
        thread_rollout_resolver::resolve_current(store, params.thread_id).await?
    };
    let path =
        resolved
            .map(|resolved| resolved.path)
            .ok_or_else(|| ThreadStoreError::InvalidRequest {
                message: format!("no rollout found for thread id {}", params.thread_id),
            })?;

    load_from_rollout_path(store, params.thread_id, &path, progress).await
}

pub(super) async fn load_from_rollout_path(
    store: &LocalThreadStore,
    thread_id: codex_protocol::ThreadId,
    path: &Path,
    progress: Option<ThreadReadProgressCallback>,
) -> ThreadStoreResult<StoredModelContext> {
    let before = super::history_revision::read(path).await;
    let session_meta = codex_rollout::read_session_meta_line(path)
        .await
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to read session metadata {}: {err}", path.display()),
        })?;
    if session_meta.meta.id != thread_id {
        return Err(ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout at {} belongs to thread {}, not {}",
                path.display(),
                session_meta.meta.id,
                thread_id
            ),
        });
    }

    let items = if matches!(session_meta.meta.history_mode, ThreadHistoryMode::Paginated) {
        let lineage = store
            .resolve_rollout_lineage(thread_id, Some(path.to_path_buf()))
            .await?;
        scan_model_context_from_lineage(lineage, session_meta, progress.clone()).await?
    } else {
        read_thread::load_history_items(path).await?
    };

    let after = super::history_revision::read(path).await;
    Ok(StoredModelContext {
        revision: before.filter(|revision| Some(revision) == after.as_ref()),
        thread_id,
        items,
    })
}

/// Loads startup context from a fork's frozen inherited prefix.
pub(super) async fn load_for_fork(
    lineage: RolloutLineage,
    history_base: Option<HistoryPosition>,
) -> ThreadStoreResult<Vec<RolloutItem>> {
    let source_path = lineage
        .segments()
        .last()
        .map(|segment| segment.rollout_path.as_path())
        .ok_or_else(|| ThreadStoreError::Internal {
            message: "fork lineage has no source segment".to_string(),
        })?;
    let mut session_meta = codex_rollout::read_session_meta_line(source_path)
        .await
        .map_err(|err| ThreadStoreError::Internal {
            message: format!(
                "failed to read session metadata {}: {err}",
                source_path.display()
            ),
        })?;
    if session_meta.meta.multi_agent_version.is_none() {
        // Recover only the runtime version before applying the fork cutoff. Stop at the
        // newest version-bearing context instead of retaining the source's full replay.
        let source_lineage = lineage.clone();
        session_meta.meta.multi_agent_version = tokio::task::spawn_blocking(move || {
            for segment in source_lineage.segments().iter().rev() {
                let file =
                    codex_rollout::open_rollout_seekable_reader(segment.rollout_path.as_path())?;
                let mut scanner = match segment.end.map(|end| end.end_byte_offset) {
                    Some(end_byte_offset) => ReverseJsonlScanner::new_at(file, end_byte_offset)?,
                    None => ReverseJsonlScanner::new(file)?,
                };
                while let Some(outcome) = scanner.scan_next_rollout_line()? {
                    let ScanOutcome::Parsed(line) = outcome else {
                        continue;
                    };
                    if let Some(version) = codex_rollout::resume_multi_agent_version(&line.item) {
                        return Ok(Some(version));
                    }
                    // Ancestor metadata does not describe the immediate source's runtime.
                    if matches!(line.item, RolloutItem::SessionMeta(_)) {
                        break;
                    }
                }
            }
            Ok::<_, io::Error>(None)
        })
        .await
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to join fork runtime version scan: {err}"),
        })?
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to read fork runtime version: {err}"),
        })?;
    }
    match history_base {
        Some(history_base) => {
            let lineage = lineage.truncate_at(history_base).await?;
            scan_model_context_from_lineage(lineage, session_meta, None).await
        }
        None => Ok(vec![RolloutItem::SessionMeta(session_meta)]),
    }
}

async fn scan_model_context_from_lineage(
    lineage: RolloutLineage,
    session_meta: SessionMetaLine,
    progress: Option<ThreadReadProgressCallback>,
) -> ThreadStoreResult<Vec<RolloutItem>> {
    let total_bytes = if progress.is_some() {
        let mut total_bytes = Some(0_u64);
        for segment in lineage.segments() {
            let segment_bytes = match segment.end {
                Some(end) => Some(end.end_byte_offset),
                None => codex_rollout::rollout_uncompressed_size(segment.rollout_path.as_path())
                    .await
                    .ok()
                    .flatten(),
            };
            total_bytes = total_bytes.and_then(|total| {
                segment_bytes.map(|segment_bytes| total.saturating_add(segment_bytes))
            });
        }
        total_bytes
    } else {
        None
    };
    let scan = tokio::task::spawn_blocking(move || {
        scan_model_context_from_lineage_blocking(&lineage, session_meta, progress, total_bytes)
    })
    .await
    .map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to join model context scan: {err}"),
    })?;
    match scan {
        Ok(items) => Ok(items),
        Err(err) => Err(ThreadStoreError::Internal {
            message: format!("failed to scan paginated model context lineage: {err}"),
        }),
    }
}

fn scan_model_context_from_lineage_blocking(
    lineage: &RolloutLineage,
    session_meta: SessionMetaLine,
    progress: Option<ThreadReadProgressCallback>,
    total_bytes: Option<u64>,
) -> io::Result<Vec<RolloutItem>> {
    let mut scan = ModelContextScan::default();
    let mut bytes_before_segment = 0_u64;
    let mut reported_progress = 0_u8;
    'segments: for segment in lineage.segments().iter().rev() {
        let file = codex_rollout::open_rollout_seekable_reader(segment.rollout_path.as_path())?;
        let mut scanner = match segment.end.map(|end| end.end_byte_offset) {
            Some(end_byte_offset) => ReverseJsonlScanner::new_at(file, end_byte_offset)?,
            None => ReverseJsonlScanner::new(file)?,
        };
        let segment_bytes = segment
            .end
            .map_or_else(|| scanner.scan_end_offset(), |end| end.end_byte_offset);
        while let Some(outcome) = scanner.scan_next_rollout_line()? {
            if let (Some(progress), Some(total_bytes)) = (&progress, total_bytes)
                && total_bytes > 0
            {
                let bytes_scanned = bytes_before_segment.saturating_add(scanner.bytes_scanned());
                let percentage = (bytes_scanned.saturating_mul(100) / total_bytes).min(99) as u8;
                if percentage > reported_progress {
                    reported_progress = percentage;
                    progress(percentage);
                }
            }
            let ScanOutcome::Parsed(line) = outcome else {
                continue;
            };
            // Each rollout segment contributes only its local delta. Its session metadata is
            // replaced with the requested thread's canonical SessionMeta after replay.
            if matches!(&line.item, RolloutItem::SessionMeta(_)) {
                break;
            }
            match scan.push(line.item) {
                ModelContextScanProgress::Continue => {}
                ModelContextScanProgress::Complete => break 'segments,
            }
        }
        bytes_before_segment = bytes_before_segment.saturating_add(segment_bytes);
    }

    let mut items = scan.finish();
    items.insert(0, RolloutItem::SessionMeta(session_meta));
    if let Some(progress) = progress
        && total_bytes.is_some()
    {
        progress(100);
    }
    Ok(items)
}
