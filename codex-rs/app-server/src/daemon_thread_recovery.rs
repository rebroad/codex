//! Saves persistent root thread IDs and restores them through the shared internal resume path.
//! Recovery uses normal cold-resume semantics without delaying readiness for runtime loading.
//! Already-loaded runtimes remain owned by their current clients.

use std::io;
use std::path::PathBuf;

use codex_app_server_transport::daemon_recovery;

pub(crate) async fn snapshot(
    path: PathBuf,
    saved: daemon_recovery::RecoverySnapshot,
) -> io::Result<()> {
    // A forced exit must not wait for file I/O in Tokio's blocking pool.
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("daemon-snapshot".into())
        .spawn(move || {
            let result = daemon_recovery::write_snapshot(&path, &saved);
            let _ = result_tx.send(result);
        })?;
    result_rx.await.map_err(io::Error::other)?
}

/// Restore a handoff in the background and consume it only after all candidates load.
pub(crate) async fn start_recovery(
    path: PathBuf,
    processor: std::sync::Arc<crate::message_processor::MessageProcessor>,
) -> io::Result<tokio::task::JoinHandle<()>> {
    let candidates = tokio::task::spawn_blocking({
        let path = path.clone();
        move || daemon_recovery::read_snapshot(&path)
    })
    .await
    .map_err(io::Error::other)??;
    Ok(tokio::spawn(async move {
        processor.restore_daemon_threads(candidates).await;
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!("failed to clear restored daemon recovery snapshot: {err}");
        }
    }))
}
