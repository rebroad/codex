use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

/// Acquires the cross-process lock shared with codex-status for one auth file.
pub(super) async fn acquire(auth_file: &Path) -> io::Result<File> {
    let canonical = auth_file
        .canonicalize()
        .unwrap_or_else(|_| auth_file.to_path_buf());
    let lock_path = canonical.with_file_name(format!(
        "{}.refresh.lock",
        canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("auth.json")
    ));
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    loop {
        match codex_utils_file_lock::try_lock(&file) {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(std::fs::TryLockError::Error(error)) if error.kind() == ErrorKind::Interrupted => {
                continue;
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}
