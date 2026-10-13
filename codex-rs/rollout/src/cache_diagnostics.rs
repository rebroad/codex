use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use serde::Serialize;

use crate::rollout_file_name::RolloutFileName;

static NEXT_TEMPORARY_FILE_ID: AtomicU64 = AtomicU64::new(0);

/// Writes cache metadata beside its rollout without recording request text.
pub fn write_cache_diagnostics<T: Serialize>(
    rollout_path: &Path,
    diagnostics: &T,
) -> io::Result<()> {
    let path = cache_diagnostics_path(rollout_path)?;
    let bytes = serde_json::to_vec(diagnostics).map_err(io::Error::other)?;
    let temporary_path = path.with_file_name(format!(
        ".cache-diagnostics-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY_FILE_ID.fetch_add(1, Ordering::Relaxed),
    ));
    let result = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            use std::io::Write;
            options.open(&temporary_path)?.write_all(&bytes)?;
        }
        #[cfg(not(unix))]
        fs::write(&temporary_path, &bytes)?;
        fs::rename(&temporary_path, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Removes cache metadata paired with a rollout, tolerating a missing sidecar.
pub fn remove_cache_diagnostics_for_rollout(rollout_path: &Path) -> io::Result<()> {
    match fs::remove_file(cache_diagnostics_path(rollout_path)?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Moves the sidecar when its rollout is archived or restored.
pub fn move_cache_diagnostics_for_rollout(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(cache_diagnostics_path(from)?, cache_diagnostics_path(to)?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn cache_diagnostics_path(rollout_path: &Path) -> io::Result<PathBuf> {
    let name = rollout_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "rollout path has no UTF-8 filename",
            )
        })?;
    if RolloutFileName::parse(name).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid rollout filename",
        ));
    }
    Ok(rollout_path.with_file_name(format!("{name}.cache-diagnostics.json")))
}

#[cfg(test)]
#[path = "cache_diagnostics_tests.rs"]
mod tests;
