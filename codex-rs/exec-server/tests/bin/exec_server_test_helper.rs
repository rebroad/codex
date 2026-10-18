use std::ffi::OsStr;
use std::path::PathBuf;

use codex_exec_server::ExecServerRuntimeOptions;
use codex_exec_server::ExecServerTelemetry;
use codex_exec_server::RequestDispatchMode;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;

const TEST_BUILD_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const CODEX_LINUX_SANDBOX_EXE_ENV_VAR: &str = "CODEX_TEST_LINUX_SANDBOX_EXE";
const SYSTEM_PROXY_REQUEST_URL_ENV: &str = "CODEX_EXEC_SERVER_TEST_SYSTEM_PROXY_REQUEST_URL";
const SYSTEM_PROXY_URL_ENV: &str = "CODEX_EXEC_SERVER_TEST_SYSTEM_PROXY_URL";

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "android")]
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .try_init()?;
    }
    let mut args = std::env::args_os();
    let _program = args.next();
    let Some(command) = args.next() else {
        return Err(invalid_input("expected exec-server command"));
    };
    if command == OsStr::new(codex_exec_server::CODEX_ARG0_EXEC_HELPER_ARG1) {
        codex_exec_server::run_arg0_exec_helper_main();
    }
    if command == OsStr::new(codex_exec_server::CODEX_FS_HELPER_ARG1) {
        codex_exec_server::run_fs_helper_main();
    }
    if command != OsStr::new("exec-server") {
        return Err(invalid_input(format!("unexpected command: {command:?}")));
    }

    let Some(flag) = args.next() else {
        return Err(invalid_input("expected --listen"));
    };
    if flag != OsStr::new("--listen") {
        return Err(invalid_input(format!("expected --listen, got {flag:?}")));
    }
    let Some(listen_url) = args.next() else {
        return Err(invalid_input("expected listen URL"));
    };
    let listen_url = listen_url
        .into_string()
        .map_err(|value| invalid_input(format!("listen URL is not UTF-8: {value:?}")))?;
    let remaining_args = args
        .map(|arg| {
            arg.into_string()
                .map_err(|value| invalid_input(format!("argument is not UTF-8: {value:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let request_dispatch_mode = match remaining_args.as_slice() {
        [] => RequestDispatchMode::Inline,
        [flag, value] if flag == "--concurrent-requests" => value.parse()?,
        args => {
            return Err(invalid_input(format!(
                "unexpected exec-server arguments: {args:?}"
            )));
        }
    };

    codex_build_info::BuildInfo::initialize(TEST_BUILD_COMMIT);
    let current_exe = std::env::current_exe()?;
    let codex_linux_sandbox_exe =
        std::env::var_os(CODEX_LINUX_SANDBOX_EXE_ENV_VAR).map(PathBuf::from);
    let mut runtime_paths = ExecServerRuntimeOptions::new(current_exe, codex_linux_sandbox_exe)?;
    runtime_paths.prefer_mxc = std::env::var("CODEX_EXEC_SERVER_TEST_PREFER_MXC")
        .ok()
        .map(|value| value.parse())
        .transpose()?;
    let http_client_factory = match (
        std::env::var(SYSTEM_PROXY_REQUEST_URL_ENV),
        std::env::var(SYSTEM_PROXY_URL_ENV),
    ) {
        (Ok(request_url), Ok(proxy_url)) => {
            codex_http_client::cache_system_proxy_route_for_test(&request_url, proxy_url);
            HttpClientFactory::new(OutboundProxyPolicy::RespectSystemProxy)
        }
        (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent)) => {
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
        }
        _ => {
            return Err(invalid_input(
                "system proxy test configuration requires both URLs",
            ));
        }
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(codex_exec_server::run_main_with_telemetry(
            &listen_url,
            runtime_paths,
            ExecServerTelemetry::default(),
            http_client_factory,
            request_dispatch_mode,
            codex_websocket_auth::WebsocketAuthSettings::default(),
        ))?;
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()).into()
}
