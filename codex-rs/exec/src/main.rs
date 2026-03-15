//! Entry-point for the `codex-exec` binary.
//!
//! When this CLI is invoked normally, it parses the standard `codex-exec` CLI
//! options and launches the non-interactive Codex agent. However, if it is
//! invoked with arg0 as `codex-linux-sandbox`, we instead treat the invocation
//! as a request to run the logic for the standalone `codex-linux-sandbox`
//! executable (i.e., parse any -s args and then run a *sandboxed* command under
//! Landlock + seccomp.
//!
//! This allows us to ship a completely separate set of functionality as part
//! of the `codex-exec` binary.
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_exec::Cli;
use codex_exec::run_main;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_cli::CliConfigOverrides;
use std::env;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct TopCli {
    #[clap(flatten)]
    config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    inner: Cli,
}

fn set_linux_sandbox_self_exe_from_argv0() {
    let mut debug_lines: Vec<String> = Vec::new();
    let debug_paths = env::var("CODEX_SANDBOX_DEBUG")
        .map(|v| !matches!(v.as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true);

    let existing = env::var("CODEX_LINUX_SANDBOX_SELF_EXE").ok();
    let arg0 = env::args().next().unwrap_or_default();

    if debug_paths {
        debug_lines.push(format!("self_exe_arg0={}", arg0));
        debug_lines.push(format!("self_exe_path_env={:?}", env::var("PATH")));
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

    if existing.is_some() {
        return;
    }
    if arg0.is_empty() {
        return;
    }

    let argv0_path = PathBuf::from(&arg0);
    let argv0_name = argv0_path.file_name().map(|s| s.to_os_string());

    if let Some(argv0_name) = argv0_name {
        if let Some(path_var) = env::var_os("PATH") {
            for dir in env::split_paths(&path_var) {
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
                        env::set_var(
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
            env::set_var(
                "CODEX_LINUX_SANDBOX_SELF_EXE",
                argv0_path.to_string_lossy().to_string(),
            );
        }
        return;
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let abs = AbsolutePathBuf::resolve_path_against_base(&arg0, &cwd)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(arg0));
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
        env::set_var(
            "CODEX_LINUX_SANDBOX_SELF_EXE",
            abs.to_string_lossy().to_string(),
        );
    }
}

fn main() -> anyhow::Result<()> {
    set_linux_sandbox_self_exe_from_argv0();
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let top_cli = TopCli::parse();
        // Merge root-level overrides into inner CLI struct so downstream logic remains unchanged.
        let mut inner = top_cli.inner;
        inner
            .config_overrides
            .raw_overrides
            .splice(0..0, top_cli.config_overrides.raw_overrides);

        run_main(inner, arg0_paths).await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn top_cli_parses_resume_prompt_after_config_flag() {
        const PROMPT: &str = "echo resume-with-global-flags-after-subcommand";
        let cli = TopCli::parse_from([
            "codex-exec",
            "resume",
            "--last",
            "--json",
            "--model",
            "gpt-5.2-codex",
            "--config",
            "reasoning_level=xhigh",
            "--dangerously-bypass-approvals-and-sandbox",
            "--skip-git-repo-check",
            PROMPT,
        ]);

        let Some(codex_exec::Command::Resume(args)) = cli.inner.command else {
            panic!("expected resume command");
        };
        let effective_prompt = args.prompt.clone().or_else(|| {
            if args.last {
                args.session_id.clone()
            } else {
                None
            }
        });
        assert_eq!(effective_prompt.as_deref(), Some(PROMPT));
        assert_eq!(cli.config_overrides.raw_overrides.len(), 1);
        assert_eq!(
            cli.config_overrides.raw_overrides[0],
            "reasoning_level=xhigh"
        );
    }
}
