use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use tempfile::NamedTempFile;

use crate::env;

pub fn normalize_for_path_comparison(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let canonical = path.as_ref().canonicalize()?;
    Ok(normalize_for_wsl(canonical))
}

pub fn normalize_for_native_workdir(path: impl AsRef<Path>) -> PathBuf {
    normalize_for_native_workdir_with_flag(path.as_ref().to_path_buf(), cfg!(windows))
}

pub struct SymlinkWritePaths {
    pub read_path: Option<PathBuf>,
    pub write_path: PathBuf,
}

/// Resolve the final filesystem target for `path` while retaining a safe write path.
///
/// This follows symlink chains (including relative symlink targets) until it reaches a
/// non-symlink path. If the chain cycles or any metadata/link resolution fails, it
/// returns `read_path: None` and uses the original absolute path as `write_path`.
/// There is no fixed max-resolution count; cycles are detected via a visited set.
pub fn resolve_symlink_write_paths(path: &Path) -> io::Result<SymlinkWritePaths> {
    let root = AbsolutePathBuf::from_absolute_path(path)
        .map(AbsolutePathBuf::into_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    let mut current = root.clone();
    let mut visited = HashSet::new();

    // Follow symlink chains while guarding against cycles.
    loop {
        let meta = match std::fs::symlink_metadata(&current) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(SymlinkWritePaths {
                    read_path: Some(current.clone()),
                    write_path: current,
                });
            }
            Err(_) => {
                return Ok(SymlinkWritePaths {
                    read_path: None,
                    write_path: root,
                });
            }
        };

        if !meta.file_type().is_symlink() {
            return Ok(SymlinkWritePaths {
                read_path: Some(current.clone()),
                write_path: current,
            });
        }

        // If we've already seen this path, the chain cycles.
        if !visited.insert(current.clone()) {
            return Ok(SymlinkWritePaths {
                read_path: None,
                write_path: root,
            });
        }

        let target = match std::fs::read_link(&current) {
            Ok(target) => target,
            Err(_) => {
                return Ok(SymlinkWritePaths {
                    read_path: None,
                    write_path: root,
                });
            }
        };

        let next = if target.is_absolute() {
            AbsolutePathBuf::from_absolute_path(&target)
        } else if let Some(parent) = current.parent() {
            AbsolutePathBuf::resolve_path_against_base(&target, parent)
        } else {
            return Ok(SymlinkWritePaths {
                read_path: None,
                write_path: root,
            });
        };

        let next = match next {
            Ok(path) => path.into_path_buf(),
            Err(_) => {
                return Ok(SymlinkWritePaths {
                    read_path: None,
                    write_path: root,
                });
            }
        };

        current = next;
    }
}

pub fn write_atomically(write_path: &Path, contents: &str) -> io::Result<()> {
    let parent = write_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path {} has no parent directory", write_path.display()),
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let tmp = NamedTempFile::new_in(parent)?;
    std::fs::write(tmp.path(), contents)?;
    tmp.persist(write_path)?;
    Ok(())
}

fn normalize_for_wsl(path: PathBuf) -> PathBuf {
    normalize_for_wsl_with_flag(path, env::is_wsl())
}

fn normalize_for_native_workdir_with_flag(path: PathBuf, is_windows: bool) -> PathBuf {
    if is_windows {
        dunce::simplified(&path).to_path_buf()
    } else {
        path
    }
}

fn normalize_for_wsl_with_flag(path: PathBuf, is_wsl: bool) -> PathBuf {
    if !is_wsl {
        return path;
    }

    if !is_wsl_case_insensitive_path(&path) {
        return path;
    }

    lower_ascii_path(path)
}

fn is_wsl_case_insensitive_path(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        use std::path::Component;

        let mut components = path.components();
        let Some(Component::RootDir) = components.next() else {
            return false;
        };
        let Some(Component::Normal(mnt)) = components.next() else {
            return false;
        };
        if !ascii_eq_ignore_case(mnt.as_bytes(), b"mnt") {
            return false;
        }
        let Some(Component::Normal(drive)) = components.next() else {
            return false;
        };
        let drive_bytes = drive.as_bytes();
        drive_bytes.len() == 1 && drive_bytes[0].is_ascii_alphabetic()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

#[cfg(target_os = "linux")]
fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(lhs, rhs)| lhs.to_ascii_lowercase() == *rhs)
}

#[cfg(target_os = "linux")]
fn lower_ascii_path(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::ffi::OsStringExt;

    // WSL mounts Windows drives under /mnt/<drive>, which are case-insensitive.
    let bytes = path.as_os_str().as_bytes();
    let mut lowered = Vec::with_capacity(bytes.len());
    for byte in bytes {
        lowered.push(byte.to_ascii_lowercase());
    }
    PathBuf::from(OsString::from_vec(lowered))
}

#[cfg(not(target_os = "linux"))]
fn lower_ascii_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
#[path = "path_utils_tests.rs"]
mod tests;

pub fn set_linux_sandbox_self_exe_from_argv0() {
    let mut debug_lines: Vec<String> = Vec::new();
    let debug_paths = std::env::var("CODEX_SANDBOX_DEBUG")
        .map(|v| !matches!(v.as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true);

    let existing = std::env::var("CODEX_LINUX_SANDBOX_SELF_EXE").ok();
    let arg0 = std::env::args().next().unwrap_or_default();

    if debug_paths {
        debug_lines.push(format!("self_exe_arg0={}", arg0));
        debug_lines.push(format!("self_exe_path_env={:?}", std::env::var("PATH")));
        debug_lines.push(format!("self_exe_env_before={:?}", existing));
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/codex-sandbox-debug.log")
            .and_then(|mut f| {
                use std::io::Write;
                for line in &debug_lines {
                    let _ = writeln!(f, "{}", line);
                }
                Ok(())
            });
    }

    if arg0.is_empty() {
        return;
    }

    let argv0_path = std::path::PathBuf::from(&arg0);
    let argv0_name = argv0_path.file_name().map(|s| s.to_os_string());

    if let Some(argv0_name) = argv0_name {
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(&argv0_name);
                if candidate.is_file() {
                    if debug_paths {
                        let _ = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("/tmp/codex-sandbox-debug.log")
                            .and_then(|mut f| {
                                use std::io::Write;
                                let _ = writeln!(
                                    f,
                                    "self_exe_path_pick={}",
                                    candidate.to_string_lossy()
                                );
                                Ok(())
                            });
                    }
                    unsafe {
                        std::env::set_var(
                            "CODEX_LINUX_SANDBOX_SELF_EXE",
                            candidate.to_string_lossy().to_string(),
                        );
                    }
                    return;
                }
            }
        }
    }

    if argv0_path.is_absolute() {
        if debug_paths {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/codex-sandbox-debug.log")
                .and_then(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(f, "self_exe_path_pick={}", argv0_path.to_string_lossy());
                    Ok(())
                });
        }
        unsafe {
            std::env::set_var(
                "CODEX_LINUX_SANDBOX_SELF_EXE",
                argv0_path.to_string_lossy().to_string(),
            );
        }
        return;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    let abs = codex_utils_absolute_path::AbsolutePathBuf::resolve_path_against_base(&arg0, &cwd)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| std::path::PathBuf::from(arg0));
    if debug_paths {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/codex-sandbox-debug.log")
            .and_then(|mut f| {
                use std::io::Write;
                let _ = writeln!(f, "self_exe_path_pick={}", abs.to_string_lossy());
                Ok(())
            });
    }
    unsafe {
        std::env::set_var(
            "CODEX_LINUX_SANDBOX_SELF_EXE",
            abs.to_string_lossy().to_string(),
        );
    }
}
