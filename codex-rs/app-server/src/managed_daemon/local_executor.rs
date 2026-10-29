//! Starts or reconnects to the local exec-server that owns managed-daemon processes.

use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::info;
use tracing::warn;

const EXECUTOR_EXECUTABLE_ENV: &str = "CODEX_MANAGED_EXECUTOR_EXECUTABLE";
const EXECUTOR_STATE_FILE: &str = "local-executor.json";
const EXECUTOR_READY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize, Serialize)]
struct ExecutorState {
    url: String,
    pid: u32,
}

pub(crate) struct LocalExecutorEndpoint {
    pub(crate) url: String,
    pub(crate) session_id_file: PathBuf,
}

/// Reuses a live local executor or starts a detached one for this CODEX_HOME.
pub(crate) async fn ensure(
    codex_home: &Path,
    codex_executable: Option<&Path>,
) -> io::Result<LocalExecutorEndpoint> {
    let directory = codex_home.join("app-server-daemon");
    tokio::fs::create_dir_all(&directory).await?;
    let state_file = directory.join(EXECUTOR_STATE_FILE);
    let session_id_file = directory.join("local-executor-session-id");

    if let Some(state) = read_state(&state_file).await?
        && executor_is_ready(&state.url).await
    {
        info!(url = %state.url, pid = state.pid, "reattaching to managed local executor");
        return Ok(LocalExecutorEndpoint {
            url: state.url,
            session_id_file,
        });
    }
    let _ = tokio::fs::remove_file(&session_id_file).await;

    let executable_override = std::env::var_os(EXECUTOR_EXECUTABLE_ENV).map(PathBuf::from);
    let executable = executable_override
        .as_deref()
        .or(codex_executable)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Codex executable is unavailable")
        })?;
    let mut command = Command::new(executable);
    if executable_override.is_none() {
        command.args(["exec-server", "--listen", "ws://127.0.0.1:0"]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(false);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn()?;
    let pid = match child.id() {
        Some(pid) => pid,
        None => {
            let _ = child.kill().await;
            return Err(io::Error::other("managed local executor has no process id"));
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            return Err(io::Error::other(
                "managed local executor stdout is unavailable",
            ));
        }
    };
    let mut lines = BufReader::new(stdout).lines();
    let url = match timeout(EXECUTOR_READY_TIMEOUT, lines.next_line()).await {
        Err(_) => {
            let _ = child.kill().await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "managed local executor did not start",
            ));
        }
        Ok(Err(error)) => {
            let _ = child.kill().await;
            return Err(error);
        }
        Ok(Ok(None)) => {
            let _ = child.kill().await;
            return Err(io::Error::other(
                "managed local executor exited before reporting its URL",
            ));
        }
        Ok(Ok(Some(url))) => url,
    };
    if !url.starts_with("ws://127.0.0.1:") {
        let _ = child.kill().await;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("managed local executor returned an unexpected URL: {url}"),
        ));
    }
    let state = ExecutorState { url, pid };
    if let Err(error) = write_state(&state_file, &state).await {
        let _ = child.kill().await;
        return Err(error);
    }
    let endpoint = LocalExecutorEndpoint {
        url: state.url,
        session_id_file,
    };

    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    tokio::spawn(async move {
        if let Err(error) = child.wait().await {
            warn!(%error, pid, "failed while waiting for managed local executor");
        }
    });
    info!(url = %endpoint.url, pid, "started detached managed local executor");
    Ok(endpoint)
}

async fn read_state(path: &Path) -> io::Result<Option<ExecutorState>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn write_state(path: &Path, state: &ExecutorState) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec(state).map_err(io::Error::other)?;
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, path).await
}

async fn executor_is_ready(url: &str) -> bool {
    let Some(address) = url.strip_prefix("ws://") else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect(address).await else {
        return false;
    };
    if stream
        .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .is_err()
    {
        return false;
    }
    let mut response = [0; 64];
    let Ok(read) = timeout(Duration::from_secs(1), stream.read(&mut response)).await else {
        return false;
    };
    let Ok(read) = read else {
        return false;
    };
    response[..read].starts_with(b"HTTP/1.1 200") || response[..read].starts_with(b"HTTP/1.0 200")
}
