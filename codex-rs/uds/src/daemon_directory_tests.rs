use std::ffi::OsStr;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::android_temporary_root;

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
