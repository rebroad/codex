use super::AppServerTransport;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const OWNER_FILE_NAME: &str = "app-server-owner.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppServerOwnerEndpoint {
    UnixSocket { socket_path: String },
    WebSocket { websocket_url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerOwnerRecord {
    pub pid: u32,
    pub app_server_version: String,
    pub endpoint: AppServerOwnerEndpoint,
}

pub struct AppServerOwnerGuard {
    path: PathBuf,
    pid: u32,
}

pub fn app_server_owner_record_path_for_profile(
    codex_home: &Path,
    profile: Option<&str>,
) -> PathBuf {
    let mut directory = codex_home.join("app-server-control");
    if let Some(profile) = profile {
        directory = directory.join(profile);
    }
    directory.join(OWNER_FILE_NAME)
}

pub fn read_app_server_owner(
    codex_home: &Path,
    profile: Option<&str>,
) -> io::Result<Option<AppServerOwnerRecord>> {
    let path = app_server_owner_record_path_for_profile(codex_home, profile);
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).map(Some).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "failed to parse app-server owner record {}: {err}",
                    path.display()
                ),
            )
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

pub fn register_app_server_owner(
    codex_home: &Path,
    profile: Option<&str>,
    transport: &AppServerTransport,
    app_server_version: &str,
) -> io::Result<Option<AppServerOwnerGuard>> {
    let endpoint = match transport {
        AppServerTransport::Stdio => AppServerOwnerEndpoint::UnixSocket {
            socket_path: super::app_server_control_socket_path_for_profile(codex_home, profile)?
                .as_path()
                .display()
                .to_string(),
        },
        AppServerTransport::UnixSocket { socket_path } => AppServerOwnerEndpoint::UnixSocket {
            socket_path: socket_path.display().to_string(),
        },
        AppServerTransport::WebSocket { bind_address } => AppServerOwnerEndpoint::WebSocket {
            websocket_url: format!("ws://{bind_address}"),
        },
        AppServerTransport::Off => return Ok(None),
    };

    let path = app_server_owner_record_path_for_profile(codex_home, profile);
    let Some(parent) = path.parent() else {
        return Err(io::Error::other(
            "app-server owner record has no parent directory",
        ));
    };
    fs::create_dir_all(parent)?;
    let temporary_path = path.with_extension(format!("json.{}", std::process::id()));
    let record = AppServerOwnerRecord {
        pid: std::process::id(),
        app_server_version: app_server_version.to_string(),
        endpoint,
    };
    let contents = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&temporary_path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary_path);
            options.open(&temporary_path)?
        }
        Err(err) => return Err(err),
    };
    file.write_all(&contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary_path, &path)?;

    Ok(Some(AppServerOwnerGuard {
        path,
        pid: record.pid,
    }))
}

impl Drop for AppServerOwnerGuard {
    fn drop(&mut self) {
        let Ok(contents) = fs::read_to_string(&self.path) else {
            return;
        };
        let Ok(record) = serde_json::from_str::<AppServerOwnerRecord>(&contents) else {
            return;
        };
        if record.pid == self.pid {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tempfile::TempDir;

    #[test]
    fn owner_record_round_trips_and_is_removed_by_guard() {
        let codex_home = TempDir::new().expect("create temp CODEX_HOME");
        let transport = AppServerTransport::WebSocket {
            bind_address: "127.0.0.1:4500"
                .parse::<SocketAddr>()
                .expect("socket address"),
        };
        let path = app_server_owner_record_path_for_profile(codex_home.path(), Some("work"));
        let guard =
            register_app_server_owner(codex_home.path(), Some("work"), &transport, "test-version")
                .expect("register owner")
                .expect("network transport should register");

        let record = read_app_server_owner(codex_home.path(), Some("work"))
            .expect("read owner")
            .expect("owner should exist");
        assert_eq!(record.pid, std::process::id());
        assert_eq!(record.app_server_version, "test-version");
        assert_eq!(
            record.endpoint,
            AppServerOwnerEndpoint::WebSocket {
                websocket_url: "ws://127.0.0.1:4500".to_string()
            }
        );

        drop(guard);
        assert!(!path.exists());
    }
}
