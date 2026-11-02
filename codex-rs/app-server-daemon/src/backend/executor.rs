//! A profile-scoped local exec-server that survives managed app-server replacement.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::sleep;

use super::pid::ProcessIdentity;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const PRESERVE_SESSIONS_ENV: &str = "CODEX_EXEC_SERVER_PRESERVE_SESSIONS";
const MANAGED_EXEC_SERVER_URL_ENV: &str = "CODEX_MANAGED_EXEC_SERVER_URL";
const SESSION_ID_FILE_ENV: &str = "CODEX_EXEC_SERVER_SESSION_ID_FILE";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecutorRecord {
    pub(super) pid: u32,
    pub(super) url: String,
    #[serde(default)]
    pub(super) retirement_supported: bool,
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) process_identity: Option<ProcessIdentity>,
}

pub(crate) struct ExecutorSupervisor {
    codex_bin: PathBuf,
    state_file: PathBuf,
}

impl ExecutorSupervisor {
    pub(crate) fn new(codex_bin: PathBuf, state_file: PathBuf) -> Self {
        Self {
            codex_bin,
            state_file,
        }
    }

    pub(crate) async fn ensure_running(&self) -> Result<String> {
        self.resume_legacy_retirements().await?;
        if let Some(record) = self.read_record().await? {
            if self.record_is_active(&record).await {
                return Ok(record.url);
            }
            self.terminate_record(&record).await?;
            let _ = tokio::fs::remove_file(&self.state_file).await;
        }

        let _ = tokio::fs::remove_file(self.session_id_file()).await;

        if let Some(parent) = self.state_file.parent() {
            codex_uds::prepare_private_socket_directory(parent)
                .await
                .with_context(|| {
                    format!(
                        "failed to create executor state directory {}",
                        parent.display()
                    )
                })?;
        }

        let mut command = Command::new(&self.codex_bin);
        command
            .args(["exec-server", "--listen", "ws://127.0.0.1:0"])
            .env(PRESERVE_SESSIONS_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to start managed local exec-server using {}",
                self.codex_bin.display()
            )
        })?;
        let pid = child
            .id()
            .context("managed exec-server has no process id")?;
        let stdout = child
            .stdout
            .take()
            .context("managed exec-server stdout is unavailable")?;
        let mut line = String::new();
        let read =
            tokio::time::timeout(READY_TIMEOUT, BufReader::new(stdout).read_line(&mut line)).await;
        match read {
            Ok(Ok(read)) if read > 0 => read,
            Ok(Ok(_)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                bail!("managed exec-server exited before reporting its listen URL");
            }
            Ok(Err(err)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(err).context("failed to read managed exec-server listen URL");
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                bail!("timed out waiting for managed exec-server to report its listen URL");
            }
        };
        let url = line.trim().to_string();
        if !url.starts_with("ws://127.0.0.1:") {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("managed exec-server reported an unexpected listen URL: {url}");
        }
        drop(child);

        let process_identity = read_process_identity(pid).await;
        let record = ExecutorRecord {
            pid,
            url: url.clone(),
            retirement_supported: true,
            session_id: None,
            process_identity,
        };
        let temp_file = self.state_file.with_extension("json.tmp");
        tokio::fs::write(&temp_file, serde_json::to_vec(&record)?)
            .await
            .with_context(|| format!("failed to write executor state {}", temp_file.display()))?;
        tokio::fs::rename(&temp_file, &self.state_file)
            .await
            .with_context(|| {
                format!(
                    "failed to publish executor state {}",
                    self.state_file.display()
                )
            })?;

        if let Err(err) = self.wait_until_listening(&url).await {
            self.terminate_record(&record).await?;
            let _ = tokio::fs::remove_file(&self.state_file).await;
            return Err(err);
        }
        Ok(url)
    }

    /// Starts a new executor generation and leaves the previous one available for handoff.
    pub(crate) async fn rotate(&self) -> Result<String> {
        if let Some(mut record) = self.read_record().await?
            && self.record_is_active(&record).await
        {
            record.session_id = tokio::fs::read_to_string(self.session_id_file())
                .await
                .ok()
                .map(|session_id| session_id.trim().to_string())
                .filter(|session_id| !session_id.is_empty());
            self.write_draining_record(&record).await?;
            if record.retirement_supported {
                signal_retirement(&record)?;
            }
        }
        let _ = tokio::fs::remove_file(&self.state_file).await;
        self.ensure_running().await
    }

    pub(crate) async fn stop(&self) -> Result<()> {
        if let Some(record) = self.read_record().await? {
            self.terminate_record(&record).await?;
        }
        for path in [&self.state_file, &self.session_id_file()] {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to remove {}", path.display()));
                }
            }
        }
        let draining_file = self.draining_state_file();
        match tokio::fs::read(&draining_file).await {
            Ok(contents) => {
                let records = serde_json::from_slice::<Vec<ExecutorRecord>>(&contents)
                    .context("invalid draining exec-server state")?;
                for record in records {
                    self.terminate_record(&record).await?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to read {}", draining_file.display()));
            }
        }
        match tokio::fs::remove_file(&draining_file).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to remove {}", draining_file.display()));
            }
        }
        Ok(())
    }

    async fn write_draining_record(&self, record: &ExecutorRecord) -> Result<()> {
        let path = self.draining_state_file();
        let mut records = match tokio::fs::read(&path).await {
            Ok(contents) => serde_json::from_slice::<Vec<ExecutorRecord>>(&contents)
                .context("invalid draining exec-server state")?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let mut active_records = Vec::with_capacity(records.len() + 1);
        for existing in records.drain(..) {
            if self.record_is_active(&existing).await {
                active_records.push(existing);
            }
        }
        records = active_records;
        if !records.iter().any(|existing| existing.pid == record.pid) {
            records.push(record.clone());
        }
        let temporary = path.with_extension("json.tmp");
        tokio::fs::write(&temporary, serde_json::to_vec(&records)?)
            .await
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        tokio::fs::rename(&temporary, &path)
            .await
            .with_context(|| format!("failed to publish {}", path.display()))
    }

    async fn resume_legacy_retirements(&self) -> Result<()> {
        let path = self.draining_state_file();
        let contents = match tokio::fs::read(&path).await {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let records = serde_json::from_slice::<Vec<ExecutorRecord>>(&contents)
            .context("invalid draining exec-server state")?;
        for record in records {
            if !record.retirement_supported && self.record_is_active(&record).await {
                super::legacy_retirement::spawn(record);
            }
        }
        Ok(())
    }

    fn draining_state_file(&self) -> PathBuf {
        self.state_file.with_file_name("exec-server-draining.json")
    }

    pub(crate) async fn endpoint_from_state(state_file: &Path) -> Option<String> {
        let contents = tokio::fs::read(state_file).await.ok()?;
        serde_json::from_slice::<ExecutorRecord>(&contents)
            .ok()
            .map(|record| record.url)
    }

    pub(crate) async fn handoff_manifest_from_state(state_file: &Path) -> Option<String> {
        let draining_file = state_file.with_file_name("exec-server-draining.json");
        let contents = tokio::fs::read(draining_file).await.ok()?;
        let records = serde_json::from_slice::<Vec<ExecutorRecord>>(&contents).ok()?;
        let supervisor = Self::new(PathBuf::new(), state_file.to_path_buf());
        let mut active_records = Vec::with_capacity(records.len());
        for record in records {
            if supervisor.record_is_active(&record).await {
                active_records.push(record);
            }
        }
        if active_records.is_empty() {
            None
        } else {
            serde_json::to_string(&active_records).ok()
        }
    }

    pub(crate) fn session_id_file(&self) -> PathBuf {
        self.state_file.with_file_name("exec-server-session-id")
    }

    async fn read_record(&self) -> Result<Option<ExecutorRecord>> {
        match tokio::fs::read(&self.state_file).await {
            Ok(contents) => Ok(Some(serde_json::from_slice(&contents).with_context(
                || {
                    format!(
                        "invalid managed exec-server state in {}",
                        self.state_file.display()
                    )
                },
            )?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => {
                Err(err).with_context(|| format!("failed to read {}", self.state_file.display()))
            }
        }
    }

    async fn record_is_active(&self, record: &ExecutorRecord) -> bool {
        if !self.process_is_same(record).await {
            return false;
        }
        tcp_connects(&record.url).await
    }

    async fn terminate_record(&self, record: &ExecutorRecord) -> Result<()> {
        if !self.process_is_same(record).await {
            return Ok(());
        }
        let pid =
            libc::pid_t::try_from(record.pid).context("managed exec-server pid is out of range")?;
        // SAFETY: signaling a positive PID is safe; the persisted identity is checked above.
        let result = unsafe { libc::kill(pid, libc::SIGTERM) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error).context("failed to stop managed exec-server");
            }
        }
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        while tokio::time::Instant::now() < deadline && self.process_is_same(record).await {
            sleep(POLL_INTERVAL).await;
        }
        Ok(())
    }

    async fn process_is_same(&self, record: &ExecutorRecord) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        if let Some(identity) = &record.process_identity {
            return identity
                .matches_process(record.pid)
                .await
                .is_ok_and(|(zombie, matches)| !zombie && matches);
        }
        process_exists(record.pid)
    }

    async fn wait_until_listening(&self, url: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if tcp_connects(url).await {
                return Ok(());
            }
            sleep(POLL_INTERVAL).await;
        }
        bail!("managed exec-server did not become reachable at {url}");
    }
}

#[cfg(unix)]
fn signal_retirement(record: &ExecutorRecord) -> Result<()> {
    let pid =
        libc::pid_t::try_from(record.pid).context("managed exec-server pid is out of range")?;
    // SAFETY: the caller verified the persisted process identity immediately before signaling.
    if unsafe { libc::kill(pid, libc::SIGUSR1) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("failed to mark managed exec-server as draining");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn signal_retirement(_record: &ExecutorRecord) -> Result<()> {
    Ok(())
}

pub(super) async fn tcp_connects(url: &str) -> bool {
    let Some(address) = url.strip_prefix("ws://") else {
        return false;
    };
    TcpStream::connect(address).await.is_ok()
}

fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
async fn read_process_identity(pid: u32) -> Option<ProcessIdentity> {
    super::pid::read_process_identity_details(pid)
        .await
        .ok()
        .map(|(_, identity)| identity)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
async fn read_process_identity(_pid: u32) -> Option<ProcessIdentity> {
    None
}

pub(crate) const fn managed_exec_server_url_env() -> &'static str {
    MANAGED_EXEC_SERVER_URL_ENV
}

pub(crate) const fn session_id_file_env() -> &'static str {
    SESSION_ID_FILE_ENV
}

pub(crate) const fn handoff_executor_manifest_env() -> &'static str {
    "CODEX_EXEC_SERVER_HANDOFF_EXECUTORS"
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
