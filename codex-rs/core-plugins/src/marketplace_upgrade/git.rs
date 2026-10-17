use crate::PluginGitMode;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;

pub(super) fn git_remote_revision(
    codex_home: &Path,
    source: &str,
    ref_name: Option<&str>,
    timeout: Duration,
    mode: PluginGitMode,
) -> Result<String, String> {
    if let Some(ref_name) = ref_name
        && is_full_git_sha(ref_name)
    {
        return Ok(ref_name.to_string());
    }

    let ref_name = ref_name.unwrap_or("HEAD");
    let mut command = git_command(mode);
    let _trusted_repository = matches!(mode, PluginGitMode::Automatic)
        .then(|| crate::configure_trusted_git_repository(&mut command, codex_home))
        .transpose()?;
    let output = run_git_command_with_timeout(
        command.arg("ls-remote").arg(source).arg(ref_name),
        "git ls-remote marketplace source",
        timeout,
    )?;
    ensure_git_success(&output, "git ls-remote marketplace source")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(first_line) = stdout.lines().next() else {
        return Err("git ls-remote returned empty output for marketplace source".to_string());
    };
    let Some((revision, _)) = first_line.split_once('\t') else {
        return Err(format!(
            "unexpected git ls-remote output for marketplace source: {first_line}"
        ));
    };
    let revision = revision.trim();
    if revision.is_empty() {
        return Err("git ls-remote returned empty revision for marketplace source".to_string());
    }
    Ok(revision.to_string())
}

pub(super) fn clone_git_source(
    codex_home: &Path,
    source: &str,
    ref_name: Option<&str>,
    sparse_paths: &[String],
    destination: &Path,
    timeout: Duration,
    mode: PluginGitMode,
) -> Result<String, String> {
    let git_destination = git_path_arg(destination);
    let mut command = git_command(mode);
    let _trusted_repository = matches!(mode, PluginGitMode::Automatic)
        .then(|| crate::configure_trusted_git_repository(&mut command, codex_home))
        .transpose()?;
    if sparse_paths.is_empty() {
        let output = run_git_command_with_timeout(
            command.arg("clone").arg(source).arg(&git_destination),
            "git clone marketplace source",
            timeout,
        )?;
        ensure_git_success(&output, "git clone marketplace source")?;
        if let Some(ref_name) = ref_name {
            let output = run_git_command_with_timeout(
                git_command(mode)
                    .arg("-C")
                    .arg(&git_destination)
                    .arg("checkout")
                    .arg(ref_name),
                "git checkout marketplace ref",
                timeout,
            )?;
            ensure_git_success(&output, "git checkout marketplace ref")?;
        }
        return git_worktree_revision(&git_destination, timeout, mode);
    }

    let output = run_git_command_with_timeout(
        command
            .arg("clone")
            .arg("--filter=blob:none")
            .arg("--no-checkout")
            .arg(source)
            .arg(&git_destination),
        "git clone marketplace source",
        timeout,
    )?;
    ensure_git_success(&output, "git clone marketplace source")?;

    let mut sparse_checkout = git_command(mode);
    sparse_checkout
        .arg("-C")
        .arg(&git_destination)
        .arg("sparse-checkout")
        .arg("set")
        .args(sparse_paths);
    let output = run_git_command_with_timeout(
        &mut sparse_checkout,
        "git sparse-checkout marketplace source",
        timeout,
    )?;
    ensure_git_success(&output, "git sparse-checkout marketplace source")?;

    let output = run_git_command_with_timeout(
        git_command(mode)
            .arg("-C")
            .arg(&git_destination)
            .arg("checkout")
            .arg(ref_name.unwrap_or("HEAD")),
        "git checkout marketplace ref",
        timeout,
    )?;
    ensure_git_success(&output, "git checkout marketplace ref")?;
    git_worktree_revision(&git_destination, timeout, mode)
}

fn git_worktree_revision(
    destination: &Path,
    timeout: Duration,
    mode: PluginGitMode,
) -> Result<String, String> {
    let output = run_git_command_with_timeout(
        git_command(mode)
            .arg("-C")
            .arg(destination)
            .arg("rev-parse")
            .arg("HEAD"),
        "git rev-parse marketplace revision",
        timeout,
    )?;
    ensure_git_success(&output, "git rev-parse marketplace revision")?;

    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if revision.is_empty() {
        Err("git rev-parse returned empty revision for marketplace source".to_string())
    } else {
        Ok(revision)
    }
}

fn is_full_git_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn git_command(mode: PluginGitMode) -> Command {
    let mut command = mode.command(Path::new("git"));
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

#[cfg(windows)]
fn git_path_arg(path: &Path) -> PathBuf {
    strip_windows_verbatim_path_prefix(&path.to_string_lossy())
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(not(windows))]
fn git_path_arg(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(any(windows, test))]
fn strip_windows_verbatim_path_prefix(path: &str) -> Option<String> {
    let stripped = path.strip_prefix(r"\\?\")?;
    let stripped = stripped
        .strip_prefix(r"UNC\")
        .map(|unc_path| format!(r"\\{unc_path}"))
        .unwrap_or_else(|| stripped.to_string());
    Some(stripped)
}

fn run_git_command_with_timeout(
    command: &mut Command,
    context: &str,
    timeout: Duration,
) -> Result<Output, String> {
    #[cfg(target_os = "linux")]
    {
        // Keep a supervisor as the process-group leader so its parent-death
        // signal can clean up Git and every descendant if this process exits.
        let original = std::mem::replace(command, Command::new("/bin/sh"));
        let program = original.get_program().to_os_string();
        let args = original
            .get_args()
            .map(std::ffi::OsStr::to_os_string)
            .collect::<Vec<_>>();
        let envs = original
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(std::ffi::OsStr::to_os_string)))
            .collect::<Vec<_>>();
        let current_dir = original.get_current_dir().map(std::path::Path::to_path_buf);
        command
            .args([
                "-c",
                "trap 'kill -KILL 0' TERM; \"$@\" & wait $!",
                "codex-git-supervisor",
            ])
            .arg(program)
            .args(args);
        for (key, value) in envs {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let parent_pid = std::process::id() as i32;
        // SAFETY: this configures a parent-death signal before exec; the helper
        // checks the captured PID too, closing the fork/exec race.
        unsafe {
            command.pre_exec(move || {
                codex_utils_pty::process_group::set_parent_death_signal(parent_pid)
            });
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run {context}: {err}"))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|err| format!("failed to wait for {context}: {err}"));
            }
            Ok(None) => {}
            Err(err) => return Err(format!("failed to poll {context}: {err}")),
        }

        if start.elapsed() >= timeout {
            #[cfg(unix)]
            let _ = codex_utils_pty::process_group::kill_process_group(child.id());
            #[cfg(not(unix))]
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .map_err(|err| format!("failed to wait for {context} after timeout: {err}"))?;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return if stderr.is_empty() {
                Err(format!("{context} timed out after {}s", timeout.as_secs()))
            } else {
                Err(format!(
                    "{context} timed out after {}s: {stderr}",
                    timeout.as_secs()
                ))
            };
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn ensure_git_success(output: &Output, context: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("{context} failed with status {}", output.status))
    } else {
        Err(format!(
            "{context} failed with status {}: {stderr}",
            output.status
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::git_command;
    use super::is_full_git_sha;
    use super::strip_windows_verbatim_path_prefix;
    use pretty_assertions::assert_eq;
    use std::ffi::OsStr;

    #[test]
    fn full_git_sha_ref_is_already_a_remote_revision() {
        assert!(is_full_git_sha("0123456789abcdef0123456789abcdef01234567"));
        assert!(!is_full_git_sha("main"));
        assert!(!is_full_git_sha("0123456"));
    }

    #[test]
    fn git_command_uses_path_lookup_with_stable_noninteractive_env() {
        let command = git_command(crate::PluginGitMode::Automatic);

        assert_eq!(command.get_program(), OsStr::new("git"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("-c"),
                OsStr::new(codex_git_utils::SAFE_BARE_REPOSITORY_CONFIG),
            ]
        );
        assert_eq!(
            command_env(&command, "GIT_OPTIONAL_LOCKS"),
            Some(Some(OsStr::new("0")))
        );
        assert_eq!(
            command_env(&command, "GIT_TERMINAL_PROMPT"),
            Some(Some(OsStr::new("0")))
        );
        assert_eq!(command_env(&command, "PATH"), None);
    }

    #[test]
    fn strips_windows_verbatim_disk_prefix_for_git() {
        assert_eq!(
            strip_windows_verbatim_path_prefix(r"\\?\C:\Users\alice\marketplace"),
            Some(r"C:\Users\alice\marketplace".to_string())
        );
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix_for_git() {
        assert_eq!(
            strip_windows_verbatim_path_prefix(r"\\?\UNC\server\share\marketplace"),
            Some(r"\\server\share\marketplace".to_string())
        );
    }

    #[test]
    fn leaves_non_verbatim_path_without_rewrite() {
        assert_eq!(strip_windows_verbatim_path_prefix(r"C:\Users\alice"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn git_descendant_dies_when_owner_exits() {
        const MARKER_ENV: &str = "CODEX_TEST_MARKETPLACE_GIT_PARENT_DEATH_MARKER";
        if let Some(marker_path) = std::env::var_os(MARKER_ENV) {
            let marker_path = std::path::PathBuf::from(marker_path);
            let mut command = std::process::Command::new("/bin/sh");
            command.args([
                "-c",
                &format!("sleep 30 & echo $! > '{}'; wait", marker_path.display()),
            ]);
            let _ = super::run_git_command_with_timeout(
                &mut command,
                "marketplace parent-death test",
                std::time::Duration::from_secs(60),
            );
            return;
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let marker_path = tmp.path().join("git.pid");
        let test_name = std::thread::current()
            .name()
            .expect("test thread name")
            .to_string();
        let mut owner = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", &test_name])
            .env(MARKER_ENV, &marker_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn owner process");

        let mut descendant_pid = None;
        for _ in 0..200 {
            if let Ok(pid) = std::fs::read_to_string(&marker_path).and_then(|pid| {
                pid.trim()
                    .parse::<libc::pid_t>()
                    .map_err(std::io::Error::other)
            }) {
                descendant_pid = Some(pid);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let descendant_pid = descendant_pid.expect("Git descendant should report its PID");

        owner.kill().expect("kill owner process");
        owner.wait().expect("wait for owner process");
        for _ in 0..200 {
            let is_terminated = std::fs::read_to_string(format!("/proc/{descendant_pid}/stat"))
                .map(|stat| {
                    stat.rsplit_once(')')
                        .and_then(|(_, rest)| rest.split_whitespace().next())
                        == Some("Z")
                })
                .unwrap_or(true);
            if is_terminated {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("Git descendant process {descendant_pid} survived its owner");
    }

    fn command_env<'a>(
        command: &'a std::process::Command,
        name: &str,
    ) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(key, _)| key == &OsStr::new(name))
            .map(|(_, value)| value)
    }
}
