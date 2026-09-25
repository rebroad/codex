use super::protected_socket_path_in;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[test]
fn protected_socket_path_fits_android_sun_path_limit() {
    const ANDROID_SUN_PATH_BYTES: usize = 108;

    let rendezvous_dir = tempfile::tempdir().expect("rendezvous directory");
    let rendezvous_path = rendezvous_dir.path().join("app-server-control.sock");
    let socket_directory = Path::new("/data/data/com.termux/files/usr/tmp/codex");
    let protected_path =
        protected_socket_path_in(socket_directory, &rendezvous_path).expect("protected path");

    assert!(
        protected_path.as_os_str().as_bytes().len() < ANDROID_SUN_PATH_BYTES,
        "{} is too long for Android's sockaddr_un.sun_path",
        protected_path.display()
    );
    assert_eq!(
        protected_path
            .file_name()
            .expect("socket name")
            .as_bytes()
            .len(),
        64,
        "the protected path should retain the full SHA-256 digest"
    );
}
