//! The host-local rendezvous root for privileged app-server RPC sockets.
//!
//! Every listener uses this root, which sandboxes hide even before a daemon
//! starts. On Android it uses Termux's private temporary directory because
//! `/tmp` is not accessible there.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::MetadataExt;
#[cfg(any(target_os = "android", test))]
use std::path::Path;
use std::path::PathBuf;

/// Returns the fixed executor-local directory that every sandbox must hide.
pub fn shared_daemon_socket_directory() -> io::Result<PathBuf> {
    #[cfg(target_os = "android")]
    let temporary_root = {
        let prefix = std::env::var_os("PREFIX");
        let tmpdir = std::env::var_os("TMPDIR");
        let path = android_temporary_root(prefix.as_deref(), tmpdir.as_deref())?;
        let path = fs::canonicalize(path)?;
        if path == Path::new("/tmp") || path.starts_with("/tmp") {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Android app-server sockets must not use /tmp",
            ));
        }
        path
    };
    #[cfg(not(target_os = "android"))]
    // Resolve the system alias /tmp -> /private/tmp on macOS.
    let temporary_root = fs::canonicalize("/tmp")?;
    let uid = unsafe { libc::geteuid() };
    Ok(temporary_root.join(format!("codex-daemon-{uid}")))
}

#[cfg(any(target_os = "android", test))]
fn android_temporary_root(
    prefix: Option<&std::ffi::OsStr>,
    tmpdir: Option<&std::ffi::OsStr>,
) -> io::Result<PathBuf> {
    let path = prefix
        .map(PathBuf::from)
        .map(|prefix| prefix.join("tmp"))
        .or_else(|| tmpdir.map(PathBuf::from))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Termux PREFIX or TMPDIR is required",
            )
        })?;
    if !path.is_absolute() || path == Path::new("/tmp") || path.starts_with("/tmp") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Android app-server temporary directory must be an absolute path outside /tmp",
        ));
    }
    Ok(path)
}

#[cfg(test)]
#[path = "daemon_directory_tests.rs"]
mod tests;

/// Creates the reserved directory, rejecting symlinks and unsafe existing owners or modes.
pub fn prepare_shared_daemon_socket_directory() -> io::Result<PathBuf> {
    let directory = shared_daemon_socket_directory()?;
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "app-server socket directory must be a user-owned directory with mode 0700",
        ));
    }
    Ok(directory)
}
