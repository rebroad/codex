use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_protocol::ThreadId;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;

use crate::ThreadStoreError;
use crate::ThreadStoreResult;

/// Removes later duplicate-ordinal records after preserving a backup of the durable rollout.
pub(super) async fn repair_duplicate_ordinals(
    thread_id: ThreadId,
    rollout_path: &Path,
) -> ThreadStoreResult<Option<PathBuf>> {
    let file = tokio::fs::File::open(rollout_path)
        .await
        .map_err(io_error)?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut line = Vec::new();
    let mut ordinals = HashSet::new();
    let mut kept_lines = Vec::new();
    let mut removed_count = 0;

    loop {
        line.clear();
        let bytes_read = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(io_error)?;
        if bytes_read == 0 {
            break;
        }
        let duplicate = serde_json::from_slice::<serde_json::Value>(&line)
            .ok()
            .and_then(|value| value.get("ordinal").and_then(serde_json::Value::as_u64))
            .is_some_and(|ordinal| !ordinals.insert(ordinal));
        if duplicate {
            removed_count += 1;
        } else {
            kept_lines.push(line.clone());
        }
    }

    if removed_count == 0 {
        return Ok(None);
    }

    let backup_path = backup_path(thread_id, rollout_path)?;
    tokio::fs::copy(rollout_path, backup_path.as_path())
        .await
        .map_err(io_error)?;

    let temporary_path = rollout_path.with_extension(format!(
        "jsonl.repair-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| ThreadStoreError::Internal {
                message: format!("failed to generate rollout repair timestamp: {err}"),
            })?
            .as_nanos()
    ));
    let result = async {
        let mut temporary = tokio::fs::File::create(&temporary_path)
            .await
            .map_err(io_error)?;
        for kept_line in kept_lines {
            temporary
                .write_all(kept_line.as_slice())
                .await
                .map_err(io_error)?;
        }
        temporary.flush().await.map_err(io_error)?;
        drop(temporary);
        replace_rollout(&temporary_path, rollout_path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
    }
    result?;

    tracing::warn!(
        thread_id = %thread_id,
        rollout_path = %rollout_path.display(),
        backup_path = %backup_path.display(),
        removed_count,
        "removed later duplicate rollout ordinals"
    );
    Ok(Some(backup_path))
}

fn backup_path(thread_id: ThreadId, rollout_path: &Path) -> ThreadStoreResult<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to generate rollout backup timestamp: {err}"),
        })?
        .as_nanos();
    let file_name = rollout_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rollout.jsonl");
    Ok(std::env::temp_dir().join(format!(
        "codex-rollout-repair-{thread_id}-{timestamp}-{file_name}.bak"
    )))
}

async fn replace_rollout(temporary_path: &Path, rollout_path: &Path) -> ThreadStoreResult<()> {
    #[cfg(windows)]
    {
        tokio::fs::remove_file(rollout_path)
            .await
            .map_err(io_error)?;
    }
    tokio::fs::rename(temporary_path, rollout_path)
        .await
        .map_err(io_error)
}

fn io_error(err: std::io::Error) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("failed to repair rollout: {err}"),
    }
}

#[cfg(test)]
#[path = "rollout_duplicate_repair_tests.rs"]
mod tests;
