use std::process::Command;

/// Put Git commands in a process group and reap descendants if their owner exits.
pub(crate) fn supervise_git_command(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
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
        // SAFETY: configures a parent-death signal before exec; the helper
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
}
