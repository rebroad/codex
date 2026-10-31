use super::ExecutorSupervisor;

#[tokio::test]
async fn repeated_rotation_keeps_each_executor_generation_available_for_handoff() {
    let directory = tempfile::Builder::new()
        .prefix("codex-executor-test-")
        .tempdir_in("/var/tmp")
        .expect("executor state directory should be created");
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
