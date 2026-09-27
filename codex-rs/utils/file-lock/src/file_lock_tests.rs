use super::*;
use std::fs::OpenOptions;

fn reopen(file: &tempfile::NamedTempFile) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(file.path())
        .unwrap()
}

#[test]
fn exclusive_lock_blocks_other_handles_until_close() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let owner = reopen(&file);
    let contender = reopen(&file);
    lock(&owner).unwrap();
    assert!(matches!(
        try_lock(&contender),
        Err(TryLockError::WouldBlock)
    ));
    assert!(matches!(
        try_lock_shared(&contender),
        Err(TryLockError::WouldBlock)
    ));
    drop(owner);
    try_lock(&contender).unwrap();
}

#[test]
fn shared_readers_exclude_writer_until_all_release() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let first = reopen(&file);
    let second = reopen(&file);
    let writer = reopen(&file);
    lock_shared(&first).unwrap();
    try_lock_shared(&second).unwrap();
    assert!(matches!(try_lock(&writer), Err(TryLockError::WouldBlock)));
    unlock(&first).unwrap();
    assert!(matches!(try_lock(&writer), Err(TryLockError::WouldBlock)));
    drop(second);
    try_lock(&writer).unwrap();
}

#[test]
fn exclusive_lock_excludes_another_process() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let owner = reopen(&file);
    lock(&owner).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "tests::child_observes_parent_lock"])
        .env("CODEX_FILE_LOCK_TEST_PATH", file.path())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
#[ignore = "subprocess helper launched with an explicitly supplied lock path"]
fn child_observes_parent_lock() {
    let path = std::env::var_os("CODEX_FILE_LOCK_TEST_PATH").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    assert!(matches!(try_lock(&file), Err(TryLockError::WouldBlock)));
    assert!(matches!(
        try_lock_shared(&file),
        Err(TryLockError::WouldBlock)
    ));
}
