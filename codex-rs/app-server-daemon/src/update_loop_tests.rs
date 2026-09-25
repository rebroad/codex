use super::install_latest_standalone;
use super::update_modes_for_identities;
#[cfg(unix)]
use crate::Daemon;
use crate::RestartMode;
use crate::UpdaterRefreshMode;
use crate::managed_install::executable_identity_from_reader;
use pretty_assertions::assert_eq;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tempfile::TempDir;

#[test]
fn unchanged_updater_uses_version_based_restart() -> std::io::Result<()> {
    assert_eq!(
        update_modes_for_identities(
            &executable_identity_from_reader(&b"same"[..])?,
            &executable_identity_from_reader(&b"same"[..])?,
        ),
        (RestartMode::IfVersionChanged, UpdaterRefreshMode::None)
    );
    Ok(())
}

#[test]
fn changed_updater_forces_refresh_even_when_version_may_match() -> std::io::Result<()> {
    assert_eq!(
        update_modes_for_identities(
            &executable_identity_from_reader(&b"old"[..])?,
            &executable_identity_from_reader(&b"new"[..])?,
        ),
        (
            RestartMode::Always,
            UpdaterRefreshMode::ReexecIfManagedBinaryChanged,
        )
    );
    Ok(())
}

#[tokio::test]
async fn standalone_installer_is_intentionally_a_noop() {
    // Termux packages update through the fork-owned release channel. The
    // upstream standalone installer must never fetch or execute here.
    install_latest_standalone()
        .await
        .expect("Termux standalone updater must fail closed as a no-op");
}

#[cfg(unix)]
fn test_daemon_with_managed_install(home: &TempDir) -> (Daemon, String) {
    let target = if cfg!(target_os = "macos") {
        format!("{}-apple-darwin", std::env::consts::ARCH)
    } else {
        format!("{}-unknown-linux-musl", std::env::consts::ARCH)
    };
    let release = format!("1.0.0-{target}");
    let standalone = home.path().join("packages/standalone");
    let bin = standalone.join("releases").join(&release).join("codex");
    std::fs::create_dir_all(bin.parent().expect("binary parent")).expect("release directory");
    codex_utils_cargo_bin::write_executable(
        &bin,
        "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo codex 1.0.0; else exec sleep 30; fi\n",
    )
    .expect("managed binary");
    std::os::unix::fs::symlink(format!("releases/{release}"), standalone.join("current"))
        .expect("current release");
    let state = home.path().join("app-server-daemon");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(state.join("app-server.stderr.log"), b"").unwrap();
    (
        Daemon {
            log_diagnostics: false,
            socket_path: home.path().join("app-server-control/server.sock"),
            pid_file: state.join("app-server.pid"),
            update_pid_file: state.join("app-server-updater.pid"),
            operation_lock_file: state.join("daemon.lock"),
            settings_file: state.join("settings.json"),
            managed_codex_bin: standalone.join("current/codex"),
            invoking_codex_bin: None,
        },
        release,
    )
}

#[cfg(unix)]
async fn test_control_server(
    daemon: &Daemon,
    home: &std::path::Path,
) -> tokio::task::JoinHandle<()> {
    use futures::SinkExt;
    use futures::StreamExt;
    std::fs::create_dir_all(daemon.socket_path.parent().expect("socket parent"))
        .expect("socket directory");
    let mut listener = codex_uds::UnixListener::bind(&daemon.socket_path)
        .await
        .expect("control listener");
    let codex_home = home.to_path_buf();
    tokio::spawn(async move {
        loop {
            let connection = listener.accept().await.expect("control connection");
            let mut websocket = tokio_tungstenite::accept_async(connection)
                .await
                .expect("websocket handshake");
            websocket
                .next()
                .await
                .expect("initialize request")
                .expect("frame");
            let version = if std::fs::read_to_string(
                crate::managed_install::package_root(&codex_home).join("auto-update-version"),
            )
            .unwrap_or_default()
            .starts_with("1.1.0")
            {
                "1.1.0"
            } else {
                "1.0.0"
            };
            websocket.send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"id": 1, "result": {
                    "userAgent": format!("codex_app_server_daemon/{version}"),
                    "codexHome": codex_home, "platformFamily": "unix", "platformOs": std::env::consts::OS,
                }}).to_string().into(),
            )).await.expect("initialize response");
            websocket
                .next()
                .await
                .expect("initialized notification")
                .expect("frame");
        }
    })
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_start_and_restart_preserve_launch_features() {
    for features in [
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::from([
            ("api_key_model_discovery".to_string(), true),
            ("code_mode_host".to_string(), false),
        ]),
    ] {
        let home = TempDir::new().unwrap();
        let (daemon, _) = test_daemon_with_managed_install(&home);
        let args_path = home.path().join("launch-args");
        std::fs::write(
            &daemon.settings_file,
            r#"{"featureOverrides":{"auth_elicitation":true},"shutdownGraceSeconds":0}"#,
        )
        .unwrap();
        codex_utils_cargo_bin::write_executable(&daemon.managed_codex_bin, &format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo codex 1.0.0; exit; fi\nif [ \"$3\" = --help ]; then exit; fi\nprintf '%s\\n' \"$@\" > '{}'\nexec sleep 30\n",
        args_path.display(),
    )).unwrap();
        let control = async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(/*secs*/ 10);
            while !args_path.exists() {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "daemon did not launch"
                );
                tokio::time::sleep(Duration::from_millis(/*millis*/ 20)).await;
            }
            test_control_server(&daemon, home.path())
                .await
                .abort_handle()
        };
        let (started, server) = tokio::join!(daemon.start(&features), control);
        assert_eq!(started.unwrap().status, crate::LifecycleStatus::Started);
        assert_eq!(
            daemon.load_settings().await.unwrap().feature_overrides,
            features
        );
        let expected = if features.is_empty() {
            "app-server\n--listen\nunix://\n--analytics-default-enabled\n--managed-daemon\n"
        } else {
            "app-server\n--listen\nunix://\n--analytics-default-enabled\n-c\nfeatures.api_key_model_discovery=true\n-c\nfeatures.code_mode_host=false\n--managed-daemon\n"
        };
        assert_eq!(std::fs::read_to_string(&args_path).unwrap(), expected);
        let reused = daemon
            .start(&std::collections::BTreeMap::from([(
                "api_key_model_discovery".to_string(),
                false,
            )]))
            .await
            .unwrap();
        assert_eq!(reused.status, crate::LifecycleStatus::AlreadyRunning);
        assert_eq!(
            daemon.load_settings().await.unwrap().feature_overrides,
            features
        );
        assert_eq!(std::fs::read_to_string(&args_path).unwrap(), expected);
        std::fs::remove_file(&args_path).unwrap();
        let restarted = daemon.restart().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(/*secs*/ 10);
        while std::fs::read_to_string(&args_path).ok().as_deref() != Some(expected)
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(/*millis*/ 20)).await;
        }
        let args = std::fs::read_to_string(args_path);
        daemon.stop().await.unwrap();
        server.abort();
        assert_eq!(restarted.unwrap().status, crate::LifecycleStatus::Restarted);
        assert_eq!(args.unwrap(), expected);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn confirmed_feature_restart_preserves_ownership_and_skips_matching_settings() {
    use crate::LifecycleStatus;
    use std::collections::BTreeMap;

    for managed in [true, false] {
        let home = TempDir::new().unwrap();
        let (daemon, _) = test_daemon_with_managed_install(&home);
        std::fs::write(&daemon.settings_file,
            r#"{"featureOverrides":{"auth_elicitation":true,"api_key_model_discovery":true},"shutdownGraceSeconds":0}"#
        ).unwrap();
        let original = daemon.load_settings().await.unwrap();
        if managed {
            daemon.start_managed_backend(&original).await.unwrap();
        }
        let server = test_control_server(&daemon, home.path()).await;
        let _lock = daemon.acquire_operation_lock().await.unwrap();
        let requested = BTreeMap::from([
            ("api_key_model_discovery".to_string(), false),
            ("mcp_oauth_refresh_coordination".to_string(), true),
        ]);
        if managed {
            // Hide the selection without removing the script the spawned shell still needs.
            let selected_package = daemon.managed_codex_bin.parent().unwrap();
            let saved_package = selected_package.with_extension("saved");
            std::fs::rename(selected_package, &saved_package).unwrap();
            let error = daemon
                .restart_with_features_locked(&requested)
                .await
                .unwrap_err();
            std::fs::rename(saved_package, selected_package).unwrap();
            assert!(
                error
                    .to_string()
                    .contains("managed Codex install not found"),
                "{error:#}"
            );
            assert_eq!(daemon.load_settings().await.unwrap(), original);
        }
        let result = daemon.restart_with_features_locked(&requested).await;
        if managed {
            assert_eq!(result.unwrap().status, LifecycleStatus::Restarted);
            let pid = std::fs::read(&daemon.pid_file).unwrap();
            let mut expected = original;
            expected.feature_overrides.extend(requested.clone());
            assert_eq!(daemon.load_settings().await.unwrap(), expected);
            assert_eq!(
                daemon
                    .restart_with_features_locked(&requested)
                    .await
                    .unwrap()
                    .status,
                LifecycleStatus::AlreadyRunning
            );
            assert_eq!(std::fs::read(&daemon.pid_file).unwrap(), pid);
            daemon.stop().await.unwrap();
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("no running managed daemon")
            );
            assert_eq!(daemon.load_settings().await.unwrap(), original);
        }
        server.abort();
    }
}
