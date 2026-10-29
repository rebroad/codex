//! Persists the managed local exec-server session so a replacement app-server can reattach.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::environment::CODEX_EXEC_SERVER_URL_ENV_VAR;

pub(crate) const SESSION_ID_FILE_ENV_VAR: &str = "CODEX_EXEC_SERVER_SESSION_ID_FILE";

pub(crate) fn session_id_file_for_url(websocket_url: &str) -> Option<PathBuf> {
    let managed_url = std::env::var(CODEX_EXEC_SERVER_URL_ENV_VAR).ok()?;
    if managed_url != websocket_url {
        return None;
    }
    std::env::var_os(SESSION_ID_FILE_ENV_VAR).map(PathBuf::from)
}

pub(crate) async fn resume_session_id(
    explicit_session_id: Option<String>,
    session_id_file: Option<&Path>,
) -> io::Result<Option<String>> {
    if explicit_session_id.is_some() {
        return Ok(explicit_session_id);
    }
    let Some(path) = session_id_file else {
        return Ok(None);
    };
    load(path).await
}

pub(crate) async fn load(path: &Path) -> io::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(session_id) => {
            let session_id = session_id.trim();
            if session_id.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "exec-server session ID file is empty",
                ));
            }
            Ok(Some(session_id.to_string()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "failed to read exec-server session ID file {}",
                path.display()
            ),
        )),
    }
}

pub(crate) async fn save(path: &Path, session_id: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to create exec-server session ID directory {}",
                    parent.display()
                ),
            )
        })?;
    }
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, session_id)
        .await
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to write exec-server session ID file {}",
                    temporary.display()
                ),
            )
        })?;
    tokio::fs::rename(&temporary, path).await.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to publish exec-server session ID file {}",
                path.display()
            ),
        )
    })
}

#[cfg(test)]
#[path = "session_id_store_tests.rs"]
mod tests;
