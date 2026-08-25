//! Linux sandbox helper entry point.
//!
//! On Linux-like targets, `codex-linux-sandbox` applies:
//! - in-process restrictions (`no_new_privs` + seccomp), and
//! - bubblewrap for filesystem isolation.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod bazel_bwrap;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod bundled_bwrap;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod bwrap;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod exec_util;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod fd_mount;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod landlock;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod launcher;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_run_main;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod proxy_lifecycle;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod proxy_routing;

/// Exit status returned when bundled bubblewrap fails digest verification.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub const BUNDLED_BWRAP_DIGEST_VERIFICATION_FAILURE_EXIT_CODE: i32 = 8;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn run_main() -> ! {
    linux_run_main::run_main();
}

#[cfg(not(target_os = "linux"))]
pub fn run_main() -> ! {
    panic!("codex-linux-sandbox is only supported on Linux");
}
