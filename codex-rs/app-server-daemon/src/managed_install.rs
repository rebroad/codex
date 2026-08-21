use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use anyhow::anyhow;

#[cfg(unix)]
use anyhow::Context;
#[cfg(unix)]
use sha2::Digest;
#[cfg(unix)]
use sha2::Sha256;
#[cfg(unix)]
use tokio::fs;
#[cfg(unix)]
use tokio::process::Command;

pub(crate) fn cargo_codex_bin() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("failed to resolve HOME for Cargo-installed Codex"))?;
    Ok(home.join(".cargo").join("bin").join("codex"))
}

#[cfg(unix)]
pub(crate) async fn resolved_managed_codex_bin(codex_bin: &Path) -> Result<PathBuf> {
    fs::canonicalize(codex_bin).await.with_context(|| {
        format!(
            "failed to resolve Cargo-installed Codex binary {}",
            codex_bin.display()
        )
    })
}

#[cfg(unix)]
pub(crate) async fn managed_codex_version(codex_bin: &Path) -> Result<String> {
    let output = Command::new(codex_bin)
        .arg("--version")
        .output()
        .await
        .with_context(|| {
            format!(
                "failed to invoke managed Codex binary {}",
                codex_bin.display()
            )
        })?;
    if !output.status.success() {
        return Err(anyhow!(
            "managed Codex binary {} exited with status {}",
            codex_bin.display(),
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "managed Codex version was not utf-8: {}",
            codex_bin.display()
        )
    })?;
    parse_codex_version(&stdout)
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutableIdentity {
    digest: [u8; 32],
}

#[cfg(unix)]
pub(crate) async fn executable_identity(executable: &Path) -> Result<ExecutableIdentity> {
    let bytes = fs::read(executable)
        .await
        .with_context(|| format!("failed to read executable {}", executable.display()))?;
    Ok(executable_identity_from_bytes(&bytes))
}

#[cfg(unix)]
pub(crate) fn executable_identity_from_bytes(bytes: &[u8]) -> ExecutableIdentity {
    ExecutableIdentity {
        digest: Sha256::digest(bytes).into(),
    }
}

#[cfg(unix)]
fn parse_codex_version(output: &str) -> Result<String> {
    let version = output
        .split_whitespace()
        .nth(1)
        .filter(|version| !version.is_empty())
        .ok_or_else(|| anyhow!("managed Codex version output was malformed"))?;
    Ok(version.to_string())
}

#[cfg(all(test, unix))]
#[path = "managed_install_tests.rs"]
mod tests;
