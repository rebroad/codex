use std::env;
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

fn codex_binary_path() -> anyhow::Result<PathBuf> {
    let current_exe = env::current_exe()?;
    let parent = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable has no parent directory"))?;

    #[cfg(target_os = "windows")]
    let codex_name = "codex.exe";
    #[cfg(not(target_os = "windows"))]
    let codex_name = "codex";

    Ok(parent.join(codex_name))
}

fn main() -> anyhow::Result<()> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    if args
        .iter()
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("responses-api-proxy {}", env!("CODEX_PROXY_VERSION"));
        return Ok(());
    }

    let codex = codex_binary_path()?;
    let mut command = Command::new(codex);
    command.arg("responses-api-proxy");
    command.args(args.drain(..));

    #[cfg(unix)]
    {
        let error = command.exec();
        Err(error.into())
    }

    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
