#[cfg(not(target_os = "android"))]
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Result as IoResult;
#[cfg(target_os = "android")]
use std::os::unix::fs::symlink;
use std::path::Path;
#[cfg(target_os = "android")]
use std::path::PathBuf;
#[cfg(target_os = "android")]
use std::thread;
#[cfg(target_os = "android")]
use std::time::Duration as StdDuration;

use super::TransportEvent;
use crate::transport::websocket::run_websocket_connection;
use codex_uds::UnixListener;
use codex_uds::UnixStream;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tracing::info;
use tracing::warn;

#[cfg(unix)]
const CONTROL_SOCKET_MODE: u32 = 0o600;

pub async fn start_control_socket_acceptor(
    socket_path: AbsolutePathBuf,
    transport_event_tx: mpsc::Sender<TransportEvent>,
    shutdown_token: CancellationToken,
) -> IoResult<JoinHandle<()>> {
    prepare_control_socket_path(socket_path.as_path()).await?;
    let listener = UnixListener::bind(socket_path.as_path()).await?;
    let socket_guard = ControlSocketFileGuard { socket_path };
    set_control_socket_permissions(socket_guard.socket_path.as_path()).await?;
    info!(
        socket_path = %socket_guard.socket_path.display(),
        "app-server control socket listening"
    );

    Ok(tokio::spawn(run_control_socket_acceptor(
        listener,
        transport_event_tx,
        shutdown_token,
        socket_guard,
    )))
}

async fn run_control_socket_acceptor(
    mut listener: UnixListener,
    transport_event_tx: mpsc::Sender<TransportEvent>,
    shutdown_token: CancellationToken,
    socket_guard: ControlSocketFileGuard,
) {
    let _socket_guard = socket_guard;
    loop {
        let stream = tokio::select! {
            _ = shutdown_token.cancelled() => {
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok(stream) => stream,
                    Err(err) => {
                        if matches!(
                            err.kind(),
                            ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::Interrupted
                        ) {
                            warn!("recoverable control socket accept error: {err}");
                            continue;
                        }
                        error!("control socket accept error: {err}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                }
            }
        };

        let transport_event_tx = transport_event_tx.clone();
        tokio::spawn(async move {
            let websocket_stream = match accept_async(stream).await {
                Ok(websocket_stream) => websocket_stream,
                Err(err) => {
                    warn!("failed to upgrade control socket websocket connection: {err}");
                    return;
                }
            };
            let (websocket_writer, websocket_reader) = websocket_stream.split();
            run_websocket_connection(websocket_writer, websocket_reader, transport_event_tx).await;
        });
    }
    info!("control socket acceptor shutting down");
}

pub async fn prepare_control_socket_path(socket_path: &Path) -> IoResult<()> {
    if let Some(parent) = socket_path.parent() {
        codex_uds::prepare_private_socket_directory(parent).await?;
    }

    match UnixStream::connect(socket_path).await {
        Ok(_stream) => {
            return Err(std::io::Error::new(
                ErrorKind::AddrInUse,
                format!(
                    "app-server control socket is already in use at {}",
                    socket_path.display()
                ),
            ));
        }
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) if err.kind() == ErrorKind::ConnectionRefused => {}
        Err(err) => {
            if !socket_path.exists() {
                return Ok(());
            }
            return Err(err);
        }
    }

    if !socket_path.try_exists()? {
        return Ok(());
    }

    if !codex_uds::is_stale_socket_path(socket_path).await? {
        return Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "app-server control socket path exists and is not a socket: {}",
                socket_path.display()
            ),
        ));
    }
    tokio::fs::remove_file(socket_path).await
}

pub struct AppServerStartupLock {
    #[cfg(not(target_os = "android"))]
    _file: std::fs::File,
    #[cfg(target_os = "android")]
    owner_token: String,
    #[cfg(target_os = "android")]
    socket_path: PathBuf,
}

pub async fn acquire_app_server_startup_lock(
    startup_lock_path: AbsolutePathBuf,
) -> IoResult<AppServerStartupLock> {
    if let Some(parent) = startup_lock_path.as_path().parent() {
        codex_uds::prepare_private_socket_directory(parent).await?;
    }
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        {
            let socket_path = startup_lock_path.to_path_buf().with_extension("lock.owner");
            let owner_token = process_lock_owner_token()?;
            loop {
                match symlink(&owner_token, &socket_path) {
                    Ok(()) => {
                        return Ok(AppServerStartupLock {
                            owner_token,
                            socket_path,
                        });
                    }
                    Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                        let existing_owner = match std::fs::read_link(&socket_path) {
                            Ok(owner) => owner,
                            Err(err) if err.kind() == ErrorKind::NotFound => continue,
                            Err(err) => return Err(err),
                        };
                        if process_lock_owner_is_alive(existing_owner.to_str().unwrap_or_default())
                        {
                            thread::sleep(StdDuration::from_millis(25));
                        } else {
                            match std::fs::remove_file(&socket_path) {
                                Ok(()) => {}
                                Err(err) if err.kind() == ErrorKind::NotFound => {}
                                Err(err) => return Err(err),
                            }
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        #[cfg(not(target_os = "android"))]
        {
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(startup_lock_path.as_path())?;
            file.lock()?;
            Ok(AppServerStartupLock { _file: file })
        }
    })
    .await
    .map_err(|err| std::io::Error::other(format!("startup lock task failed: {err}")))?
}

#[cfg(target_os = "android")]
impl Drop for AppServerStartupLock {
    fn drop(&mut self) {
        if std::fs::read_link(&self.socket_path)
            .ok()
            .and_then(|owner| owner.into_os_string().into_string().ok())
            .as_deref()
            == Some(self.owner_token.as_str())
        {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

#[cfg(target_os = "android")]
fn process_lock_owner_token() -> IoResult<String> {
    let pid = std::process::id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let start_time = process_start_time_from_stat(&stat)
        .ok_or_else(|| std::io::Error::other(format!("malformed /proc/{pid}/stat")))?;
    Ok(format!("{pid}:{start_time}"))
}

#[cfg(target_os = "android")]
fn process_lock_owner_is_alive(owner: &str) -> bool {
    let Some((pid, expected_start_time)) = owner.split_once(':') else {
        return false;
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return false;
    };
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    process_start_time_from_stat(&stat) == Some(expected_start_time)
}

#[cfg(target_os = "android")]
fn process_start_time_from_stat(stat: &str) -> Option<&str> {
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)
}

#[cfg(unix)]
async fn set_control_socket_permissions(socket_path: &Path) -> IoResult<()> {
    use std::os::unix::fs::PermissionsExt;

    tokio::fs::set_permissions(
        socket_path,
        std::fs::Permissions::from_mode(CONTROL_SOCKET_MODE),
    )
    .await
}

#[cfg(not(unix))]
async fn set_control_socket_permissions(_socket_path: &Path) -> IoResult<()> {
    Ok(())
}

struct ControlSocketFileGuard {
    socket_path: AbsolutePathBuf,
}

impl Drop for ControlSocketFileGuard {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(self.socket_path.as_path())
            && err.kind() != ErrorKind::NotFound
        {
            warn!(
                socket_path = %self.socket_path.display(),
                %err,
                "failed to remove app-server control socket file"
            );
        }
    }
}
