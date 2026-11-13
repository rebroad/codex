use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

/// Acquires the cross-process lock shared with codex-status for one auth file.
pub(super) async fn acquire(auth_file: &Path) -> io::Result<File> {
    let canonical = auth_file
        .canonicalize()
        .unwrap_or_else(|_| auth_file.to_path_buf());
    #[cfg(unix)]
    let user = unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string());
    let temp_dir = std::env::var_os("TMPDIR").unwrap_or_else(|| "/var/tmp".into());
    let lock_dir = Path::new(&temp_dir).join(format!("codex-auth-locks-{user}"));
    fs::create_dir_all(&lock_dir)?;
    #[cfg(unix)]
    fs::set_permissions(&lock_dir, fs::Permissions::from_mode(0o700))?;
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    let lock_path = lock_dir.join(format!("{digest:x}.refresh.lock"));
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
