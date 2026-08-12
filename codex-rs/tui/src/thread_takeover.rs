use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use color_eyre::eyre::Result;
use color_eyre::eyre::bail;
use tokio::time::sleep;

const WRITER_LOCK_DIR: &str = "thread-writer-locks";

pub(crate) async fn offer(thread_id: codex_protocol::ThreadId, codex_home: &Path) -> Result<bool> {
    let path = codex_home
        .join(WRITER_LOCK_DIR)
        .join(format!("{thread_id}.lock"));
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(err) => return Err(err.into()),
    };
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<u32>().ok())
    else {
        bail!("the active writer did not publish an owner PID; stop it manually and retry")
    };
    if pid == std::process::id() {
        bail!("the active writer belongs to this process")
    }
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        bail!("cannot offer session takeover without an interactive terminal")
    }

    let raw_mode_was_enabled = crossterm::terminal::disable_raw_mode().is_ok();
    let answer = {
        let mut stderr = std::io::stderr().lock();
        writeln!(
            stderr,
            "Session {thread_id} is open in another Codex process (PID {pid})."
        )?;
        write!(
            stderr,
            "Terminate that process and take over this session? [y/N]: "
        )?;
        stderr.flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        input.trim().eq_ignore_ascii_case("y") || input.trim().eq_ignore_ascii_case("yes")
    };
    if raw_mode_was_enabled {
        let _ = crossterm::terminal::enable_raw_mode();
    }
    if !answer {
        return Ok(false);
    }

    terminate_process(pid).await?;
    for _ in 0..50 {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => match file.try_lock() {
                Ok(()) => {
                    file.unlock()?;
                    return Ok(true);
                }
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(err)) => return Err(err.into()),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(err) => return Err(err.into()),
        }
        sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for the other Codex process to release the session")
}

#[cfg(unix)]
async fn terminate_process(pid: u32) -> Result<()> {
    let pid = libc::pid_t::try_from(pid)?;
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err.into());
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn terminate_process(pid: u32) -> Result<()> {
    let status = tokio::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .await?;
    if !status.success() {
        bail!("taskkill failed for Codex process {pid}")
    }
    Ok(())
}
