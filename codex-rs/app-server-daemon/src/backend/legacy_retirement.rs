#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::collections::HashMap;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::process::Stdio;
use std::time::Duration;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use tokio::process::Command;
use tokio::time::sleep;

use super::executor::ExecutorRecord;
use super::executor::tcp_connects;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(test)]
const OUTPUT_REPLAY_GRACE: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const OUTPUT_REPLAY_GRACE: Duration = Duration::from_secs(30);

pub(super) fn spawn(record: ExecutorRecord) {
    tokio::spawn(async move {
        retire_when_idle(record).await;
    });
}

async fn retire_when_idle(record: ExecutorRecord) {
    let mut idle_since = None;
    loop {
        if !same_process(&record).await || !tcp_connects(&record.url).await {
            return;
        }

        match has_descendants(record.pid).await {
            Ok(true) | Err(_) => idle_since = None,
            Ok(false) => {
                let since = idle_since.get_or_insert_with(tokio::time::Instant::now);
                if since.elapsed() >= OUTPUT_REPLAY_GRACE {
                    if same_process(&record).await && tcp_connects(&record.url).await {
                        terminate(&record).await;
                    }
                    return;
                }
            }
        }
        sleep(POLL_INTERVAL).await;
    }
}

async fn same_process(record: &ExecutorRecord) -> bool {
    if let Some(identity) = &record.process_identity {
        return identity
            .matches_process(record.pid)
            .await
            .is_ok_and(|(zombie, matches)| !zombie && matches);
    }
    let Ok(pid) = libc::pid_t::try_from(record.pid) else {
        return false;
    };
    // SAFETY: signal zero only checks whether the recorded PID is alive.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

async fn terminate(record: &ExecutorRecord) {
    let Ok(pid) = libc::pid_t::try_from(record.pid) else {
        return;
    };
    // SAFETY: process identity and the server endpoint are rechecked immediately before signaling.
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::warn!(pid = record.pid, %error, "failed to retire legacy exec-server");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
async fn has_descendants(root_pid: u32) -> std::io::Result<bool> {
    let mut pending = vec![root_pid];
    while let Some(pid) = pending.pop() {
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        let children = match tokio::fs::read_to_string(children_path).await {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for child in children.split_whitespace() {
            let child = child.parse::<u32>().map_err(std::io::Error::other)?;
            pending.push(child);
            if child != root_pid {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
async fn has_descendants(root_pid: u32) -> std::io::Result<bool> {
    let output = Command::new("/bin/ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Err(std::io::Error::other("ps could not enumerate processes"));
    }

    let mut children_by_parent = HashMap::<u32, Vec<u32>>::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(parent)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(pid), Ok(parent)) = (pid.parse::<u32>(), parent.parse::<u32>()) else {
            continue;
        };
        children_by_parent.entry(parent).or_default().push(pid);
    }
    Ok(children_by_parent
        .get(&root_pid)
        .is_some_and(|children| !children.is_empty()))
}
