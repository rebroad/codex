use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use crate::EPHEMERAL_SESSIONS_SUBDIR;
use crate::WriterLockCoordinator;
use crate::rollout_file_name::RolloutFileName;

/// Removes expired rollout files from the dedicated ephemeral session tree.
/// Files owned by a live writer are skipped.
pub fn cleanup_expired_ephemeral_rollouts(
    codex_home: &Path,
    retention: Duration,
) -> io::Result<usize> {
    let root = codex_home.join(EPHEMERAL_SESSIONS_SUBDIR);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Ok(0),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    }
    let cutoff = SystemTime::now()
        .checked_sub(retention)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let coordinator = Arc::new(WriterLockCoordinator::new(codex_home));
    let mut removed = 0;

    for year in read_directories(&root)? {
        for month in read_directories(&year)? {
            for day in read_directories(&month)? {
                for entry in fs::read_dir(day)? {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        continue;
                    }
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let Some(file_name) = RolloutFileName::parse(name) else {
                        continue;
                    };
                    if !is_expired(&path, cutoff)? {
                        continue;
                    }

                    let Some(_writer_lock) =
                        coordinator.try_acquire_for_cleanup(file_name.thread_id())?
                    else {
                        continue;
                    };
                    if is_expired(&path, cutoff)? {
                        match fs::remove_file(&path) {
                            Ok(()) => removed += 1,
                            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                            Err(err) => return Err(err),
                        }
                    }
                }
            }
        }
    }

    Ok(removed)
}

fn read_directories(path: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

fn is_expired(path: &Path, cutoff: SystemTime) -> io::Result<bool> {
    let modified = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.modified()?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    Ok(modified <= cutoff)
}

#[cfg(test)]
#[path = "ephemeral_cleanup_tests.rs"]
mod tests;
