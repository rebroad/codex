use super::ExecutorSupervisor;

#[tokio::test]
async fn repeated_rotation_keeps_each_executor_generation_available_for_handoff() {
    let directory = test_tempdir("codex-executor-test-");
    let executable = directory.path().join("fake-codex");
    std::fs::write(
        &executable,
        "#!/bin/sh\n\
         exec /usr/bin/python3 -c 'import signal, socket; signal.signal(signal.SIGUSR1, signal.SIG_IGN); listener = socket.socket(); listener.bind((\"127.0.0.1\", 0)); listener.listen(); print(\"ws://%s:%s\" % listener.getsockname(), flush=True); exec(\"while True:\\n connection, _ = listener.accept()\\n connection.close()\")'\n",
    )
    .expect("fake Codex executable should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("fake Codex executable should be executable");
    }

    let state_file = directory.path().join("exec-server.json");
    let supervisor = ExecutorSupervisor::new(executable, state_file.clone());
    let old_url = supervisor
        .ensure_running()
        .await
        .expect("first executor should start");
    let middle_url = supervisor
        .rotate()
        .await
        .expect("replacement executor should start");
    let new_url = supervisor
        .rotate()
        .await
        .expect("second replacement executor should start");

    assert_ne!(old_url, new_url);
    assert_ne!(old_url, middle_url);
    assert_ne!(middle_url, new_url);
    assert_eq!(
        ExecutorSupervisor::endpoint_from_state(&state_file)
            .await
            .as_deref(),
        Some(new_url.as_str()),
        "new app-server launches should resolve to the newest executor generation"
    );
    assert!(super::tcp_connects(&old_url).await);
    assert!(super::tcp_connects(&middle_url).await);
    assert!(super::tcp_connects(&new_url).await);
    let draining: Vec<super::ExecutorRecord> = serde_json::from_slice(
        &tokio::fs::read(directory.path().join("exec-server-draining.json"))
            .await
            .expect("old executor record should be retained"),
    )
    .expect("draining executor state should be valid");
    assert_eq!(
        draining
            .iter()
            .map(|record| record.url.as_str())
            .collect::<Vec<_>>(),
        vec![old_url.as_str(), middle_url.as_str()]
    );
    let handoff = ExecutorSupervisor::handoff_manifest_from_state(&state_file)
        .await
        .expect("handoff manifest should include draining generations");
    let handoff: Vec<super::ExecutorRecord> =
        serde_json::from_str(&handoff).expect("handoff manifest should be valid");
    assert_eq!(
        serde_json::to_value(handoff).expect("handoff should serialize"),
        serde_json::to_value(draining).expect("draining records should serialize")
    );

    supervisor
        .stop()
        .await
        .expect("all executor generations should stop cleanly");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn legacy_executor_waits_for_its_child_before_retiring() {
    let directory = test_tempdir("codex-legacy-executor-test-");
    let executable = directory.path().join("fake-codex");
    std::fs::write(
        &executable,
        "#!/bin/sh\n\
         exec /usr/bin/python3 -c 'import signal, socket, subprocess; signal.signal(signal.SIGUSR1, signal.SIG_DFL); signal.signal(signal.SIGCHLD, signal.SIG_IGN); subprocess.Popen([\"/bin/sleep\", \"0.8\"]); listener = socket.socket(); listener.bind((\"127.0.0.1\", 0)); listener.listen(); print(\"ws://%s:%s\" % listener.getsockname(), flush=True); exec(\"while True:\\n connection, _ = listener.accept()\\n connection.close()\")'\n",
    )
    .expect("fake Codex executable should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("fake Codex executable should be executable");
    }

    let state_file = directory.path().join("exec-server.json");
    let supervisor = ExecutorSupervisor::new(executable, state_file.clone());
    let old_url = supervisor
        .ensure_running()
        .await
        .expect("legacy executor should start");
    let mut record: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&state_file).await.expect("state exists"))
            .expect("state should be valid");
    record["retirementSupported"] = serde_json::Value::Bool(false);
    tokio::fs::write(
        &state_file,
        serde_json::to_vec(&record).expect("legacy state should serialize"),
    )
    .await
    .expect("legacy state should be updated");

    let new_url = supervisor
        .rotate()
        .await
        .expect("replacement executor should start");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        super::tcp_connects(&old_url).await,
        "legacy exec-server must survive while its child process is active"
    );
    assert!(super::tcp_connects(&new_url).await);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline && super::tcp_connects(&old_url).await {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !super::tcp_connects(&old_url).await,
        "legacy exec-server should retire after its child and replay grace finish"
    );
    assert!(
        ExecutorSupervisor::handoff_manifest_from_state(&state_file)
            .await
            .is_none(),
        "retired executor should be omitted from later handoff manifests"
    );

    supervisor
        .stop()
        .await
        .expect("current and draining executor generations should stop cleanly");
}

fn test_tempdir(prefix: &str) -> tempfile::TempDir {
    let root = std::env::var_os("TMPDIR")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            #[cfg(target_os = "android")]
            {
                std::env::var_os("PREFIX")
                    .map(|prefix| std::path::PathBuf::from(prefix).join("tmp"))
            }
            #[cfg(not(target_os = "android"))]
            {
                None
            }
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/var/tmp"));
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .expect("temporary directory should be created")
}
