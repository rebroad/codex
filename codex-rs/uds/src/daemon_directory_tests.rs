use std::ffi::OsStr;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::{android_socket_directory, android_temporary_root};

#[test]
fn android_uses_termux_prefix_tmp_even_if_tmpdir_differs() {
    let root = android_temporary_root(
        Some(OsStr::new("/data/data/com.termux/files/usr")),
        Some(OsStr::new("/var/tmp")),
    )
    .expect("Termux temporary root");

    assert_eq!(root, Path::new("/data/data/com.termux/files/usr/tmp"));
}

#[test]
fn android_can_fall_back_to_tmpdir_without_prefix() {
    let root = android_temporary_root(None, Some(OsStr::new("/data/local/tmp")))
        .expect("Android temporary root");

    assert_eq!(root, Path::new("/data/local/tmp"));
}

#[test]
fn android_rejects_tmp_as_a_temporary_root() {
    let error = android_temporary_root(None, Some(OsStr::new("/tmp")))
        .expect_err("Android must not use /tmp");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn android_places_sockets_in_a_short_codex_subdirectory() {
    let executable = std::env::current_exe().expect("test executable");
    let temporary_base = executable.parent().expect("test executable directory");
    let prefix = tempfile::tempdir_in(temporary_base).expect("Termux prefix");
    let temporary_root = prefix.path().join("tmp");
    std::fs::create_dir(&temporary_root).expect("Termux temporary root");

    let socket_directory = android_socket_directory(Some(prefix.path().as_os_str()), None)
        .expect("Android socket directory");

    assert_eq!(socket_directory, temporary_root.join("codex"));
}
