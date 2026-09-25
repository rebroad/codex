#![cfg(target_os = "android")]

use std::io::BufRead;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use codex_utils_pty::process_group::supervise_process_tree_on_parent_exit;
use codex_utils_pty::wait_for_child_exit;

const CHILD_MODE: &str = "CODEX_TEST_ANDROID_PARENT_DEATH_CHILD";

#[test]
fn parent_death_terminates_supervised_process_group() {
    if std::env::var_os(CHILD_MODE).is_some() {
        let mut command = Command::new("/system/bin/sleep");
        command.arg("30");
        supervise_process_tree_on_parent_exit(&mut command);
        let mut child = command.spawn().expect("start supervised child");
        writeln!(std::io::stdout().lock(), "{}", child.id()).expect("write child process ID");
        let _ = child.wait();
        return;
    }

    let mut fixture = Command::new(std::env::current_exe().expect("test executable path"))
        .args([
            "--exact",
            "parent_death_terminates_supervised_process_group",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start parent-death fixture");
    let stdout = fixture.stdout.take().expect("fixture stdout is piped");
    let mut output = std::io::BufReader::new(stdout);
    let process_group_id = loop {
        let mut line = String::new();
        if output
            .read_line(&mut line)
            .expect("read supervised process group ID")
            == 0
        {
            drop(output);
            let output = fixture.wait_with_output().expect("read fixture output");
            panic!(
                "fixture exited before reporting its process ID: status={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if let Ok(process_group_id) = line.trim().parse::<libc::pid_t>() {
            break process_group_id;
        }
    };

    fixture.kill().expect("kill owning test process");
    let _ = fixture.wait();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: Signal zero checks whether this process group still exists.
        let result = unsafe { libc::killpg(process_group_id, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "supervised process group {process_group_id} remained after its owner exited"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tokio_child_waits_from_a_background_task() {
    let mut command = tokio::process::Command::new("/system/bin/sleep");
    command.arg("1");
    let child = command.spawn().expect("start Tokio child");
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::spawn(async move {
            let mut child = child;
            wait_for_child_exit(&mut child).await
        }),
    )
    .await
    .expect("Tokio child exits before timeout")
    .expect("wait task joins")
    .expect("wait for Tokio child");
    assert!(
        status.success(),
        "Tokio child exited unexpectedly: {status}"
    );
}
