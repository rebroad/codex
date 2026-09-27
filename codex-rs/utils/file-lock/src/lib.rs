//! Advisory file locking with native Android support absent from `std::fs::File`.
//! Locks belong to the supplied file and are released when it closes or is unlocked.

use std::fs::File;
use std::fs::TryLockError;
use std::io;

/// Acquire an exclusive lock, waiting for other owners to release it.
pub fn lock(file: &File) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return native_lock(file, libc::LOCK_EX);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    file.lock()
}

/// Acquire a shared lock, waiting for exclusive owners to release it.
pub fn lock_shared(file: &File) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return native_lock(file, libc::LOCK_SH);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    file.lock_shared()
}

/// Acquire an exclusive lock without waiting, distinguishing contention from failure.
pub fn try_lock(file: &File) -> Result<(), TryLockError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return native_try_lock(file, libc::LOCK_EX);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    file.try_lock()
}

/// Acquire a shared lock without waiting, distinguishing contention from failure.
pub fn try_lock_shared(file: &File) -> Result<(), TryLockError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return native_try_lock(file, libc::LOCK_SH);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    file.try_lock_shared()
}

/// Release a previously acquired lock without closing the file.
pub fn unlock(file: &File) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return native_lock(file, libc::LOCK_UN);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    file.unlock()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_try_lock(file: &File, operation: libc::c_int) -> Result<(), TryLockError> {
    native_lock(file, operation | libc::LOCK_NB).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            TryLockError::WouldBlock
        } else {
            TryLockError::Error(error)
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_lock(file: &File, operation: libc::c_int) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: The borrowed file owns this live descriptor throughout the syscall.
        if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(test)]
#[path = "file_lock_tests.rs"]
mod tests;
