use clap::Parser;
use codex_protocol::models::PermissionProfile;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use crate::landlock::LocalIpcPolicy;
use crate::landlock::apply_permission_profile_to_current_thread;

#[derive(Debug, Parser)]
struct AndroidSandboxCommand {
    #[arg(long = "sandbox-policy-cwd")]
    sandbox_policy_cwd: PathBuf,

    #[arg(long = "command-cwd", hide = true)]
    command_cwd: Option<PathBuf>,

    #[arg(
        long = "permission-profile",
        hide = true,
        value_parser = parse_permission_profile
    )]
    permission_profile: PermissionProfile,

    #[arg(long = "use-legacy-landlock", hide = true, default_value_t = false)]
    _use_legacy_landlock: bool,

    #[arg(long = "allow-network-for-proxy", hide = true, default_value_t = false)]
    allow_network_for_proxy: bool,

    #[arg(
        long = "allow-isolated-local-ipc",
        hide = true,
        default_value_t = false
    )]
    _allow_isolated_local_ipc: bool,

    #[arg(long = "proxy-route-spec", hide = true)]
    _proxy_route_spec: Option<String>,

    #[arg(long = "no-proc", hide = true, default_value_t = false)]
    _no_proc: bool,

    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

fn parse_permission_profile(value: &str) -> std::result::Result<PermissionProfile, String> {
    serde_json::from_str(value).map_err(|err| format!("invalid permission profile JSON: {err}"))
}

pub fn run_main() -> ! {
    let AndroidSandboxCommand {
        sandbox_policy_cwd,
        command_cwd,
        permission_profile,
        allow_network_for_proxy,
        command,
        ..
    } = AndroidSandboxCommand::parse();

    if command.is_empty() {
        panic!("No command specified to execute.");
    }

    let command_cwd = command_cwd.as_deref().unwrap_or(&sandbox_policy_cwd);
    if let Err(error) = apply_permission_profile_to_current_thread(
        &permission_profile,
        &sandbox_policy_cwd,
        // Android kernels used by Termux do not expose Landlock. Keep the
        // network seccomp restrictions, but do not fail the executor when
        // the optional filesystem backend reports NotEnforced.
        false,
        allow_network_for_proxy,
        false,
        LocalIpcPolicy::Disabled,
    ) {
        panic!("failed to apply Android sandbox: {error}");
    }

    let error = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(command_cwd)
        .exec();
    panic!("failed to execute sandboxed command: {error}");
}
