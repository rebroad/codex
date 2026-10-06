use super::*;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use time::OffsetDateTime;

fn rollout_path(
    codex_home: &Path,
    thread_id: ThreadId,
    age: Duration,
) -> io::Result<std::path::PathBuf> {
    let timestamp = OffsetDateTime::now_utc();
    let filename = RolloutFileName::new(timestamp, thread_id, thread_id)
        .render()
        .map_err(io::Error::other)?;
    let path = codex_home
        .join(EPHEMERAL_SESSIONS_SUBDIR)
        .join(timestamp.year().to_string())
        .join(format!("{:02}", u8::from(timestamp.month())))
        .join(format!("{:02}", timestamp.day()))
        .join(filename);
    fs::create_dir_all(path.parent().expect("rollout parent"))?;
    let file = fs::File::create(&path)?;
    file.set_times(fs::FileTimes::new().set_modified(SystemTime::now() - age))?;
    Ok(path)
}

#[test]
fn removes_expired_ephemeral_rollouts_only() -> io::Result<()> {
    let home = TempDir::new()?;
    let old = rollout_path(
        home.path(),
        ThreadId::new(),
        Duration::from_secs(48 * 60 * 60),
    )?;
    let fresh = rollout_path(home.path(), ThreadId::new(), Duration::from_secs(60))?;
    let ordinary = home.path().join("sessions/2026/01/01/old.jsonl");
    fs::create_dir_all(ordinary.parent().expect("ordinary parent"))?;
    fs::write(&ordinary, "ordinary session")?;

    assert_eq!(
        cleanup_expired_ephemeral_rollouts(home.path(), Duration::from_secs(24 * 60 * 60))?,
        1
    );
    assert!(!old.exists());
    assert!(fresh.exists());
    assert!(ordinary.exists());
    Ok(())
}

#[test]
fn skips_expired_rollouts_with_an_active_writer() -> io::Result<()> {
    let home = TempDir::new()?;
    let thread_id = ThreadId::new();
    let old = rollout_path(home.path(), thread_id, Duration::from_secs(48 * 60 * 60))?;
    let coordinator = Arc::new(WriterLockCoordinator::new(home.path()));
    let writer_lock = coordinator.acquire(thread_id)?;

    let retention = Duration::from_secs(24 * 60 * 60);
    assert_eq!(
        cleanup_expired_ephemeral_rollouts(home.path(), retention)?,
        0
    );
    assert!(old.exists());

    drop(writer_lock);
    assert_eq!(
        cleanup_expired_ephemeral_rollouts(home.path(), retention)?,
        1
    );
    assert!(!old.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn does_not_follow_an_ephemeral_tree_symlink() -> io::Result<()> {
    let home = TempDir::new()?;
    let external = TempDir::new()?;
    let old = rollout_path(
        external.path(),
        ThreadId::new(),
        Duration::from_secs(48 * 60 * 60),
    )?;
    std::os::unix::fs::symlink(external.path(), home.path().join(EPHEMERAL_SESSIONS_SUBDIR))?;

    assert_eq!(
        cleanup_expired_ephemeral_rollouts(home.path(), Duration::from_secs(24 * 60 * 60))?,
        0
    );
    assert!(old.exists());
    Ok(())
}
