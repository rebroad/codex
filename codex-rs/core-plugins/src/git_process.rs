use std::process::Command;

/// Put Git commands in a process group and kill descendants if their owner exits.
pub(crate) fn supervise_git_command(command: &mut Command) {
    codex_utils_pty::process_group::supervise_process_tree_on_parent_exit(command);
}
