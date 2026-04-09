use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use clap_complete::Shell;
use clap_complete::generate;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_build_info::CODEX_BUILD_VERSION;
use codex_chatgpt::apply_command::ApplyCommand;
use codex_chatgpt::apply_command::run_apply_command;
use codex_cli::LandlockCommand;
use codex_cli::SeatbeltCommand;
use codex_cli::WindowsCommand;
use codex_cli::login::read_api_key_from_stdin;
use codex_cli::login::run_login_status;
use codex_cli::login::run_login_with_api_key;
use codex_cli::login::run_login_with_chatgpt;
use codex_cli::login::run_login_with_device_code;
use codex_cli::login::run_logout;
use codex_cli::login::run_tlogin_complete;
use codex_cli::login::run_tlogin_start;
use codex_cloud_tasks::Cli as CloudTasksCli;
use codex_exec::Cli as ExecCli;
use codex_exec::Command as ExecCommand;
use codex_exec::ReviewArgs;
use codex_execpolicy::ExecPolicyCheckCommand;
use codex_responses_api_proxy::Args as ResponsesApiProxyArgs;
use codex_state::StateRuntime;
use codex_state::account_usage_key;
use codex_state::state_db_path;
use codex_state::usage_db_path;
use codex_tui::AppExitInfo;
use codex_tui::Cli as TuiCli;
use codex_tui::CliStatusRateLimitMode;
use codex_tui::ExitReason;
use codex_tui::update_action::UpdateAction;
use codex_utils_cli::CliConfigOverrides;
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::PathBuf;
use supports_color::Stream;

#[cfg(target_os = "macos")]
mod app_cmd;
#[cfg(target_os = "macos")]
mod desktop_app;
mod mcp_cmd;
#[cfg(not(windows))]
mod wsl_paths;

use crate::mcp_cmd::McpCli;

use codex_core::AuthManager;
use codex_core::INTERACTIVE_SESSION_SOURCES;
use codex_core::RolloutRecorder;
use codex_core::ThreadSortKey;
use codex_core::auth::RefreshTokenError;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::find_codex_home;
use codex_core::find_thread_names_by_ids;
use codex_core::find_thread_path_by_id_str;
use codex_core::find_thread_path_by_name_str;
use codex_core::parse_turn_item;
use codex_core::prompt_preview_line;
use codex_features::FEATURES;
use codex_features::Stage;
use codex_features::is_known_feature_key;
use codex_protocol::ThreadId;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_terminal_detection::TerminalName;
use std::collections::HashSet;

/// Codex CLI
///
/// If no subcommand is specified, options will be forwarded to the interactive CLI.
#[derive(Debug, Parser)]
#[clap(
    author,
    version = CODEX_BUILD_VERSION,
    // If a sub‑command is given, ignore requirements of the default args.
    subcommand_negates_reqs = true,
    // The executable is sometimes invoked via a platform‑specific name like
    // `codex-x86_64-unknown-linux-musl`, but the help output should always use
    // the generic `codex` command name that users run.
    bin_name = "codex",
    override_usage = "codex [OPTIONS] [PROMPT]\n       codex [OPTIONS] <COMMAND> [ARGS]"
)]
struct MultitoolCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    pub feature_toggles: FeatureToggles,

    #[clap(flatten)]
    remote: InteractiveRemoteOptions,

    #[clap(flatten)]
    interactive: TuiCli,

    /// Override the auth file location (defaults to `$CODEX_HOME/auth.json`).
    #[arg(long = "auth-file", value_name = "FILE", global = true)]
    auth_file: Option<PathBuf>,

    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    /// Run Codex non-interactively.
    #[clap(visible_alias = "e")]
    Exec(ExecCli),

    /// Run a code review non-interactively.
    Review(ReviewArgs),

    /// Manage login.
    Login(LoginCommand),

    /// Telegram-oriented two-phase login (for bot orchestration).
    Tlogin(TloginCommand),

    /// Remove stored authentication credentials.
    Logout(LogoutCommand),

    /// Manage external MCP servers for Codex.
    Mcp(McpCli),

    /// Start Codex as an MCP server (stdio).
    McpServer,

    /// Show local session configuration status and exit.
    Status(StatusCommand),

    /// Manage local account usage tracking data.
    Usage(UsageCommand),

    /// [experimental] Run the app server or related tooling.
    AppServer(AppServerCommand),

    /// Launch the Codex desktop app (downloads the macOS installer if missing).
    #[cfg(target_os = "macos")]
    App(app_cmd::AppCommand),

    /// Generate shell completion scripts.
    Completion(CompletionCommand),

    /// Run commands within a Codex-provided sandbox.
    Sandbox(SandboxArgs),

    /// Debugging tools.
    Debug(DebugCommand),

    /// Execpolicy tooling.
    #[clap(hide = true)]
    Execpolicy(ExecpolicyCommand),

    /// Apply the latest diff produced by Codex agent as a `git apply` to your local working tree.
    #[clap(visible_alias = "a")]
    Apply(ApplyCommand),

    /// Resume a previous interactive session (picker by default; use --last to continue the most recent).
    Resume(ResumeCommand),

    /// Fork a previous interactive session (picker by default; use --last to fork the most recent).
    Fork(ForkCommand),

    /// [EXPERIMENTAL] Browse tasks from Codex Cloud and apply changes locally.
    #[clap(name = "cloud", alias = "cloud-tasks")]
    Cloud(CloudTasksCli),

    /// Internal: run the responses API proxy.
    #[clap(hide = true)]
    ResponsesApiProxy(ResponsesApiProxyArgs),

    /// Internal: relay stdio to a Unix domain socket.
    #[clap(hide = true, name = "stdio-to-uds")]
    StdioToUds(StdioToUdsCommand),

    /// Inspect feature flags.
    Features(FeaturesCli),
}

#[derive(Debug, Parser)]
struct StatusCommand {
    /// Use locally cached usage/rate-limit values when available.
    #[arg(long, default_value_t = false)]
    cached: bool,

    /// Extra arguments accepted after `--` for machine-readable output mode.
    #[arg(last = true, allow_hyphen_values = true, value_name = "ARG")]
    trailing_args: Vec<String>,
}

#[derive(Debug, Parser)]
struct CompletionCommand {
    /// Shell to generate completions for
    #[clap(value_enum, default_value_t = Shell::Bash)]
    shell: Shell,
}

#[derive(Debug, Parser)]
struct UsageCommand {
    #[command(subcommand)]
    subcommand: UsageSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum UsageSubcommand {
    /// Clear locally tracked account usage samples and totals.
    Clear(UsageClearCommand),
}

#[derive(Debug, Parser)]
struct UsageClearCommand {
    /// Clear usage for all locally tracked accounts on the active provider.
    #[arg(long = "all-accounts", default_value_t = false)]
    all_accounts: bool,

    /// Skip the interactive confirmation prompt.
    #[arg(long = "yes", short = 'y', default_value_t = false)]
    yes: bool,
}

#[derive(Debug, Parser)]
struct DebugCommand {
    #[command(subcommand)]
    subcommand: DebugSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum DebugSubcommand {
    /// Tooling: helps debug the app server.
    AppServer(DebugAppServerCommand),

    /// Internal: reset local memory state for a fresh start.
    #[clap(hide = true)]
    ClearMemories,
}

#[derive(Debug, Parser)]
struct DebugAppServerCommand {
    #[command(subcommand)]
    subcommand: DebugAppServerSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum DebugAppServerSubcommand {
    // Send message to app server V2.
    SendMessageV2(DebugAppServerSendMessageV2Command),
}

#[derive(Debug, Parser)]
struct DebugAppServerSendMessageV2Command {
    #[arg(value_name = "USER_MESSAGE", required = true)]
    user_message: String,
}

#[derive(Debug, Parser)]
struct ResumeCommand {
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Continue the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false)]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    /// Include non-interactive sessions in the resume picker and --last selection.
    #[arg(
        long = "include-non-interactive",
        default_value_t = true,
        overrides_with = "exclude_non_interactive"
    )]
    include_non_interactive: bool,

    /// Exclude non-interactive sessions from the resume picker and --last selection.
    #[arg(
        long = "exclude-non-interactive",
        default_value_t = false,
        overrides_with = "include_non_interactive"
    )]
    exclude_non_interactive: bool,

    #[clap(flatten)]
    remote: InteractiveRemoteOptions,

    #[clap(flatten)]
    config_overrides: TuiCli,
}

#[derive(Debug, Parser)]
struct ForkCommand {
    /// Conversation/session id (UUID). When provided, forks this session.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Fork the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false, conflicts_with = "session_id")]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    /// Show available fork points (numbered) for the selected session and exit.
    #[arg(long = "show", default_value_t = false, conflicts_with = "pick")]
    show: bool,

    /// Fork before the Nth user prompt in the selected session (1-based).
    #[arg(long = "pick", value_name = "POINT", conflicts_with = "show")]
    pick: Option<usize>,

    #[clap(flatten)]
    remote: InteractiveRemoteOptions,

    #[clap(flatten)]
    config_overrides: TuiCli,
}

#[derive(Debug, Parser)]
struct SandboxArgs {
    #[command(subcommand)]
    cmd: SandboxCommand,
}

#[derive(Debug, clap::Subcommand)]
enum SandboxCommand {
    /// Run a command under Seatbelt (macOS only).
    #[clap(visible_alias = "seatbelt")]
    Macos(SeatbeltCommand),

    /// Run a command under the Linux sandbox (bubblewrap by default).
    #[clap(visible_alias = "landlock")]
    Linux(LandlockCommand),

    /// Run a command under Windows restricted token (Windows only).
    Windows(WindowsCommand),
}

#[derive(Debug, Parser)]
struct ExecpolicyCommand {
    #[command(subcommand)]
    sub: ExecpolicySubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum ExecpolicySubcommand {
    /// Check execpolicy files against a command.
    #[clap(name = "check")]
    Check(ExecPolicyCheckCommand),
}

#[derive(Debug, Parser)]
struct LoginCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,

    #[arg(
        long = "with-api-key",
        help = "Read the API key from stdin (e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`)"
    )]
    with_api_key: bool,

    #[arg(
        long = "api-key",
        value_name = "API_KEY",
        help = "(deprecated) Previously accepted the API key directly; now exits with guidance to use --with-api-key",
        hide = true
    )]
    api_key: Option<String>,

    #[arg(long = "device-auth")]
    use_device_code: bool,

    /// EXPERIMENTAL: Use custom OAuth issuer base URL (advanced)
    /// Override the OAuth issuer base URL (advanced)
    #[arg(long = "experimental_issuer", value_name = "URL", hide = true)]
    issuer_base_url: Option<String>,

    /// EXPERIMENTAL: Use custom OAuth client ID (advanced)
    #[arg(long = "experimental_client-id", value_name = "CLIENT_ID", hide = true)]
    client_id: Option<String>,

    #[command(subcommand)]
    action: Option<LoginSubcommand>,
}

#[derive(Debug, clap::Subcommand)]
enum LoginSubcommand {
    /// Show login status.
    Status,
}

#[derive(Debug, Parser)]
struct LogoutCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,
}

#[derive(Debug, Parser)]
struct TloginCommand {
    #[command(subcommand)]
    action: TloginSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum TloginSubcommand {
    /// Start a Telegram login flow and print the device verification details.
    Start(TloginStartCommand),
    /// Complete a Telegram login flow after the user approves device auth.
    Complete(TloginCompleteCommand),
}

#[derive(Debug, Parser)]
struct TloginStartCommand {
    /// Stable bot user identifier for caching pending device auth state.
    #[arg(long = "user-id", value_name = "USER_ID")]
    user_id: String,
    /// Emit machine-readable JSON.
    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Parser)]
struct TloginCompleteCommand {
    /// Stable bot user identifier used during `tlogin start`.
    #[arg(long = "user-id", value_name = "USER_ID")]
    user_id: String,
}

#[derive(Debug, Parser)]
struct AppServerCommand {
    /// Omit to run the app server; specify a subcommand for tooling.
    #[command(subcommand)]
    subcommand: Option<AppServerSubcommand>,

    /// Transport endpoint URL. Supported values: `stdio://` (default),
    /// `ws://IP:PORT`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = codex_app_server::AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: codex_app_server::AppServerTransport,

    /// Controls whether analytics are enabled by default.
    ///
    /// Analytics are disabled by default for app-server. Users have to explicitly opt in
    /// via the `analytics` section in the config.toml file.
    ///
    /// However, for first-party use cases like the VSCode IDE extension, we default analytics
    /// to be enabled by default by setting this flag. Users can still opt out by setting this
    /// in their config.toml:
    ///
    /// ```toml
    /// [analytics]
    /// enabled = false
    /// ```
    ///
    /// See https://developers.openai.com/codex/config-advanced/#metrics for more details.
    #[arg(long = "analytics-default-enabled")]
    analytics_default_enabled: bool,

    #[command(flatten)]
    auth: codex_app_server::AppServerWebsocketAuthArgs,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::enum_variant_names)]
enum AppServerSubcommand {
    /// [experimental] Generate TypeScript bindings for the app server protocol.
    GenerateTs(GenerateTsCommand),

    /// [experimental] Generate JSON Schema for the app server protocol.
    GenerateJsonSchema(GenerateJsonSchemaCommand),

    /// [internal] Generate internal JSON Schema artifacts for Codex tooling.
    #[clap(hide = true)]
    GenerateInternalJsonSchema(GenerateInternalJsonSchemaCommand),
}

#[derive(Debug, Args)]
struct GenerateTsCommand {
    /// Output directory where .ts files will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out_dir: PathBuf,

    /// Optional path to the Prettier executable to format generated files
    #[arg(short = 'p', long = "prettier", value_name = "PRETTIER_BIN")]
    prettier: Option<PathBuf>,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    experimental: bool,
}

#[derive(Debug, Args)]
struct GenerateJsonSchemaCommand {
    /// Output directory where the schema bundle will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out_dir: PathBuf,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    experimental: bool,
}

#[derive(Debug, Args)]
struct GenerateInternalJsonSchemaCommand {
    /// Output directory where internal JSON Schema artifacts will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out_dir: PathBuf,
}

#[derive(Debug, Parser)]
struct StdioToUdsCommand {
    /// Path to the Unix domain socket to connect to.
    #[arg(value_name = "SOCKET_PATH")]
    socket_path: PathBuf,
}

fn format_exit_messages(exit_info: AppExitInfo, color_enabled: bool) -> Vec<String> {
    let AppExitInfo {
        token_usage,
        thread_id: conversation_id,
        thread_name,
        ..
    } = exit_info;

    if token_usage.is_zero() {
        return Vec::new();
    }

    let mut lines = vec![format!(
        "{}",
        codex_protocol::protocol::FinalOutput::from(token_usage)
    )];

    if let Some(resume_cmd) =
        codex_core::util::resume_command(thread_name.as_deref(), conversation_id)
    {
        let command = if color_enabled {
            resume_cmd.cyan().to_string()
        } else {
            resume_cmd
        };
        lines.push(format!("To continue this session, run {command}"));
    }

    lines
}

/// Handle the app exit and print the results. Optionally run the update action.
fn handle_app_exit(exit_info: AppExitInfo) -> anyhow::Result<()> {
    match exit_info.exit_reason {
        ExitReason::Fatal(message) => {
            eprintln!("ERROR: {message}");
            std::process::exit(1);
        }
        ExitReason::UserRequested => { /* normal exit */ }
    }

    let update_action = exit_info.update_action;
    let color_enabled = supports_color::on(Stream::Stdout).is_some();
    for line in format_exit_messages(exit_info, color_enabled) {
        println!("{line}");
    }
    if let Some(action) = update_action {
        run_update_action(action)?;
    }
    Ok(())
}

/// Run the update action and print the result.
fn run_update_action(action: UpdateAction) -> anyhow::Result<()> {
    println!();
    let cmd_str = action.command_str();
    println!("Updating Codex via `{cmd_str}`...");

    let status = {
        #[cfg(windows)]
        {
            // On Windows, run via cmd.exe so .CMD/.BAT are correctly resolved (PATHEXT semantics).
            std::process::Command::new("cmd")
                .args(["/C", &cmd_str])
                .status()?
        }
        #[cfg(not(windows))]
        {
            let (cmd, args) = action.command_args();
            let command_path = crate::wsl_paths::normalize_for_wsl(cmd);
            let normalized_args: Vec<String> = args
                .iter()
                .map(crate::wsl_paths::normalize_for_wsl)
                .collect();
            std::process::Command::new(&command_path)
                .args(&normalized_args)
                .status()?
        }
    };
    if !status.success() {
        anyhow::bail!("`{cmd_str}` failed with status {status}");
    }
    println!("\n🎉 Update ran successfully! Please restart Codex.");
    Ok(())
}

fn run_execpolicycheck(cmd: ExecPolicyCheckCommand) -> anyhow::Result<()> {
    cmd.run()
}

async fn run_debug_app_server_command(cmd: DebugAppServerCommand) -> anyhow::Result<()> {
    match cmd.subcommand {
        DebugAppServerSubcommand::SendMessageV2(cmd) => {
            let codex_bin = std::env::current_exe()?;
            codex_app_server_test_client::send_message_v2(&codex_bin, &[], cmd.user_message, &None)
                .await
        }
    }
}

#[derive(Debug, Default, Parser, Clone)]
struct FeatureToggles {
    /// Enable a feature (repeatable). Equivalent to `-c features.<name>=true`.
    #[arg(long = "enable", value_name = "FEATURE", action = clap::ArgAction::Append, global = true)]
    enable: Vec<String>,

    /// Disable a feature (repeatable). Equivalent to `-c features.<name>=false`.
    #[arg(long = "disable", value_name = "FEATURE", action = clap::ArgAction::Append, global = true)]
    disable: Vec<String>,
}

#[derive(Debug, Default, Parser, Clone)]
struct InteractiveRemoteOptions {
    /// Connect the TUI to a remote app server websocket endpoint.
    ///
    /// Accepted forms: `ws://host:port` or `wss://host:port`.
    #[arg(long = "remote", value_name = "ADDR")]
    remote: Option<String>,

    /// Name of the environment variable containing the bearer token to send to
    /// a remote app server websocket.
    #[arg(long = "remote-auth-token-env", value_name = "ENV_VAR")]
    remote_auth_token_env: Option<String>,
}

impl FeatureToggles {
    fn to_overrides(&self) -> anyhow::Result<Vec<String>> {
        let mut v = Vec::new();
        for feature in &self.enable {
            Self::validate_feature(feature)?;
            v.push(format!("features.{feature}=true"));
        }
        for feature in &self.disable {
            Self::validate_feature(feature)?;
            v.push(format!("features.{feature}=false"));
        }
        Ok(v)
    }

    fn validate_feature(feature: &str) -> anyhow::Result<()> {
        if is_known_feature_key(feature) {
            Ok(())
        } else {
            anyhow::bail!("Unknown feature flag: {feature}")
        }
    }
}

#[derive(Debug, Parser)]
struct FeaturesCli {
    #[command(subcommand)]
    sub: FeaturesSubcommand,
}

#[derive(Debug, Parser)]
enum FeaturesSubcommand {
    /// List known features with their stage and effective state.
    List,
    /// Enable a feature in config.toml.
    Enable(FeatureSetArgs),
    /// Disable a feature in config.toml.
    Disable(FeatureSetArgs),
}

#[derive(Debug, Parser)]
struct FeatureSetArgs {
    /// Feature key to update (for example: unified_exec).
    feature: String,
}

fn stage_str(stage: Stage) -> &'static str {
    match stage {
        Stage::UnderDevelopment => "under development",
        Stage::Experimental { .. } => "experimental",
        Stage::Stable => "stable",
        Stage::Deprecated => "deprecated",
        Stage::Removed => "removed",
    }
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        cli_main(arg0_paths).await?;
        Ok(())
    })
}

async fn cli_main(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    let raw_argv: Vec<String> = std::env::args().collect();
    let MultitoolCli {
        config_overrides: mut root_config_overrides,
        feature_toggles,
        remote,
        mut interactive,
        auth_file,
        subcommand,
    } = MultitoolCli::parse();

    codex_login::set_auth_file_override(auth_file);

    // Fold --enable/--disable into config overrides so they flow to all subcommands.
    let toggle_overrides = feature_toggles.to_overrides()?;
    root_config_overrides.raw_overrides.extend(toggle_overrides);
    let root_remote = remote.remote;
    let root_remote_auth_token_env = remote.remote_auth_token_env;

    match subcommand {
        None => {
            if let Some(prompt) = interactive.prompt.as_deref() {
                if is_status_shortcut_prompt(prompt) && interactive.images.is_empty() {
                    run_status_command(
                        &root_config_overrides,
                        &interactive,
                        StatusOutputMode::default(),
                        CliStatusRateLimitMode::LiveOnly,
                    )
                    .await?;
                    return Ok(());
                }
                let candidate = prompt.trim();
                anyhow::bail!(
                    "Unknown command `{candidate}`. Positional prompts are only supported via `codex exec`."
                );
            }
            prepend_config_flags(
                &mut interactive.config_overrides,
                root_config_overrides.clone(),
            );
            let exit_info = run_interactive_tui(
                interactive,
                root_remote.clone(),
                root_remote_auth_token_env.clone(),
                arg0_paths.clone(),
            )
            .await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Exec(mut exec_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "exec",
            )?;
            prepend_config_flags(
                &mut exec_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_exec::run_main(exec_cli, arg0_paths.clone()).await?;
        }
        Some(Subcommand::Review(review_args)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "review",
            )?;
            let mut exec_cli = ExecCli::try_parse_from(["codex", "exec"])?;
            exec_cli.command = Some(ExecCommand::Review(review_args));
            prepend_config_flags(
                &mut exec_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_exec::run_main(exec_cli, arg0_paths.clone()).await?;
        }
        Some(Subcommand::McpServer) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "mcp-server",
            )?;
            codex_mcp_server::run_main(arg0_paths.clone(), root_config_overrides).await?;
        }
        Some(Subcommand::Mcp(mut mcp_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "mcp",
            )?;
            // Propagate any root-level config overrides (e.g. `-c key=value`).
            prepend_config_flags(&mut mcp_cli.config_overrides, root_config_overrides.clone());
            mcp_cli.run().await?;
        }
        Some(Subcommand::AppServer(app_server_cli)) => {
            let AppServerCommand {
                subcommand,
                listen,
                analytics_default_enabled,
                auth,
            } = app_server_cli;
            reject_remote_mode_for_app_server_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                subcommand.as_ref(),
            )?;
            match subcommand {
                None => {
                    let transport = listen;
                    let auth = auth.try_into_settings()?;
                    codex_app_server::run_main_with_transport(
                        arg0_paths.clone(),
                        root_config_overrides,
                        codex_core::config_loader::LoaderOverrides::default(),
                        analytics_default_enabled,
                        transport,
                        codex_protocol::protocol::SessionSource::VSCode,
                        auth,
                    )
                    .await?;
                }
                Some(AppServerSubcommand::GenerateTs(gen_cli)) => {
                    let options = codex_app_server_protocol::GenerateTsOptions {
                        experimental_api: gen_cli.experimental,
                        ..Default::default()
                    };
                    codex_app_server_protocol::generate_ts_with_options(
                        &gen_cli.out_dir,
                        gen_cli.prettier.as_deref(),
                        options,
                    )?;
                }
                Some(AppServerSubcommand::GenerateJsonSchema(gen_cli)) => {
                    codex_app_server_protocol::generate_json_with_experimental(
                        &gen_cli.out_dir,
                        gen_cli.experimental,
                    )?;
                }
                Some(AppServerSubcommand::GenerateInternalJsonSchema(gen_cli)) => {
                    codex_app_server_protocol::generate_internal_json_schema(&gen_cli.out_dir)?;
                }
            }
        }
        Some(Subcommand::Status(status_command)) => {
            let default_rate_limit_mode = if status_command.cached {
                CliStatusRateLimitMode::AllowCached
            } else {
                CliStatusRateLimitMode::LiveOnly
            };
            let status_options = parse_status_invocation_options(
                &raw_argv,
                default_rate_limit_mode,
                &status_command.trailing_args,
            )?;
            if let Some(auth_file_override) = status_options.auth_file_override {
                codex_login::set_auth_file_override(Some(auth_file_override));
            }
            run_status_command(
                &root_config_overrides,
                &interactive,
                status_options.output_mode,
                status_options.rate_limit_mode,
            )
            .await?;
        }
        Some(Subcommand::Usage(usage_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "usage",
            )?;
            match usage_cli.subcommand {
                UsageSubcommand::Clear(clear_command) => {
                    run_usage_clear_command(&root_config_overrides, &interactive, clear_command)
                        .await?;
                }
            }
        }
        #[cfg(target_os = "macos")]
        Some(Subcommand::App(app_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "app",
            )?;
            app_cmd::run_app(app_cli).await?;
        }
        Some(Subcommand::Resume(ResumeCommand {
            session_id,
            last,
            all,
            include_non_interactive,
            exclude_non_interactive,
            remote,
            config_overrides,
        })) => {
            let include_non_interactive = resolve_resume_include_non_interactive(
                include_non_interactive,
                exclude_non_interactive,
            );
            if all && !std::io::stdout().is_terminal() {
                print_resume_sessions_non_interactive(all, include_non_interactive).await?;
                return Ok(());
            }
            interactive = finalize_resume_interactive(
                interactive,
                root_config_overrides.clone(),
                session_id,
                last,
                all,
                include_non_interactive,
                config_overrides,
            );
            let exit_info = run_interactive_tui(
                interactive,
                remote.remote.or(root_remote.clone()),
                remote
                    .remote_auth_token_env
                    .or(root_remote_auth_token_env.clone()),
                arg0_paths.clone(),
            )
            .await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Fork(ForkCommand {
            session_id,
            last,
            all,
            show,
            pick,
            remote,
            config_overrides,
        })) => {
            if show {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "fork --show",
                )?;
                reject_remote_mode_for_subcommand(
                    remote.remote.as_deref(),
                    remote.remote_auth_token_env.as_deref(),
                    "fork --show",
                )?;
                if last {
                    anyhow::bail!(
                        "`codex fork --show` requires SESSION_ID and does not support `--last`."
                    );
                }
                let Some(session_id) = session_id else {
                    anyhow::bail!("`codex fork --show` requires SESSION_ID.");
                };
                let path = resolve_fork_rollout_path(&session_id)
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No saved session found with ID {session_id}. Run `codex fork` without an ID to choose from existing sessions."
                        )
                    })?;
                let prompts = load_user_prompt_points(path.as_path()).await?;
                if let Some(item) = load_thread_item_by_rollout_path(path.as_path()).await? {
                    let conversation_name = if let Some(thread_id) = item.thread_id {
                        let mut ids = HashSet::new();
                        ids.insert(thread_id);
                        find_thread_names_by_ids(find_codex_home()?.as_path(), &ids)
                            .await
                            .ok()
                            .and_then(|names| names.get(&thread_id).cloned())
                            .unwrap_or_else(|| {
                                item.first_user_message
                                    .clone()
                                    .unwrap_or_else(|| "(no message yet)".to_string())
                            })
                    } else {
                        item.first_user_message
                            .clone()
                            .unwrap_or_else(|| "(no message yet)".to_string())
                    };
                    println!("Conversation: {conversation_name}");
                    println!("Created at: {}", item.created_at.as_deref().unwrap_or("-"));
                    println!("Updated at: {}", item.updated_at.as_deref().unwrap_or("-"));
                    println!("Branch: {}", item.git_branch.as_deref().unwrap_or("-"));
                    println!(
                        "CWD: {}",
                        item.cwd
                            .as_deref()
                            .map(|cwd| cwd.display().to_string())
                            .unwrap_or_else(|| "-".to_string())
                    );
                    println!();
                }
                if prompts.is_empty() {
                    println!("No fork points found for session {session_id}.");
                } else {
                    for (idx, prompt) in prompts.iter().enumerate() {
                        let preview = prompt_preview_line(prompt);
                        println!("{}. {preview}", idx + 1);
                    }
                }
                return Ok(());
            }

            let fork_nth_user_message = if let Some(point) = pick {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "fork --pick",
                )?;
                reject_remote_mode_for_subcommand(
                    remote.remote.as_deref(),
                    remote.remote_auth_token_env.as_deref(),
                    "fork --pick",
                )?;
                if last {
                    anyhow::bail!(
                        "`codex fork --pick` requires SESSION_ID and does not support `--last`."
                    );
                }
                let Some(session_id) = session_id.as_deref() else {
                    anyhow::bail!("`codex fork --pick` requires SESSION_ID.");
                };
                let path = resolve_fork_rollout_path(session_id).await?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "No saved session found with ID {session_id}. Run `codex fork` without an ID to choose from existing sessions."
                    )
                })?;
                let prompts = load_user_prompt_points(path.as_path()).await?;
                if prompts.is_empty() {
                    anyhow::bail!("Session {session_id} has no fork points.");
                }
                if point == 0 {
                    anyhow::bail!("`--pick` must be a positive integer (1-based).");
                }
                if point > prompts.len() {
                    anyhow::bail!(
                        "`--pick {point}` is out of range for session {session_id}. Valid range is 1..={}.",
                        prompts.len()
                    );
                }
                Some(point - 1)
            } else {
                None
            };

            interactive = finalize_fork_interactive(
                interactive,
                root_config_overrides.clone(),
                session_id,
                last,
                all,
                fork_nth_user_message,
                config_overrides,
            );
            let exit_info = run_interactive_tui(
                interactive,
                remote.remote.or(root_remote.clone()),
                remote
                    .remote_auth_token_env
                    .or(root_remote_auth_token_env.clone()),
                arg0_paths.clone(),
            )
            .await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Login(mut login_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "login",
            )?;
            prepend_config_flags(
                &mut login_cli.config_overrides,
                root_config_overrides.clone(),
            );
            match login_cli.action {
                Some(LoginSubcommand::Status) => {
                    run_login_status(login_cli.config_overrides).await;
                }
                None => {
                    if login_cli.use_device_code {
                        run_login_with_device_code(
                            login_cli.config_overrides,
                            login_cli.issuer_base_url,
                            login_cli.client_id,
                        )
                        .await;
                    } else if login_cli.api_key.is_some() {
                        eprintln!(
                            "The --api-key flag is no longer supported. Pipe the key instead, e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`."
                        );
                        std::process::exit(1);
                    } else if login_cli.with_api_key {
                        let api_key = read_api_key_from_stdin();
                        run_login_with_api_key(login_cli.config_overrides, api_key).await;
                    } else {
                        run_login_with_chatgpt(login_cli.config_overrides).await;
                    }
                }
            }
        }
        Some(Subcommand::Tlogin(tlogin_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "tlogin",
            )?;
            match tlogin_cli.action {
                TloginSubcommand::Start(start) => {
                    let result =
                        run_tlogin_start(root_config_overrides.clone(), start.user_id).await?;
                    if start.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "verificationUrl": result.verification_url,
                                "userCode": result.user_code,
                                "message": result.message,
                            }))?
                        );
                    } else {
                        println!("{}", result.message);
                    }
                }
                TloginSubcommand::Complete(complete) => {
                    run_tlogin_complete(root_config_overrides.clone(), complete.user_id).await?;
                    eprintln!("Successfully logged in");
                }
            }
        }
        Some(Subcommand::Logout(mut logout_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "logout",
            )?;
            prepend_config_flags(
                &mut logout_cli.config_overrides,
                root_config_overrides.clone(),
            );
            run_logout(logout_cli.config_overrides).await;
        }
        Some(Subcommand::Completion(completion_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "completion",
            )?;
            print_completion(completion_cli);
        }
        Some(Subcommand::Cloud(mut cloud_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "cloud",
            )?;
            prepend_config_flags(
                &mut cloud_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_cloud_tasks::run_main(cloud_cli, arg0_paths.codex_linux_sandbox_exe.clone())
                .await?;
        }
        Some(Subcommand::Sandbox(sandbox_args)) => match sandbox_args.cmd {
            SandboxCommand::Macos(mut seatbelt_cli) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "sandbox macos",
                )?;
                prepend_config_flags(
                    &mut seatbelt_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_seatbelt(
                    seatbelt_cli,
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )
                .await?;
            }
            SandboxCommand::Linux(mut landlock_cli) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "sandbox linux",
                )?;
                prepend_config_flags(
                    &mut landlock_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_landlock(
                    landlock_cli,
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )
                .await?;
            }
            SandboxCommand::Windows(mut windows_cli) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "sandbox windows",
                )?;
                prepend_config_flags(
                    &mut windows_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_windows(
                    windows_cli,
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )
                .await?;
            }
        },
        Some(Subcommand::Debug(DebugCommand { subcommand })) => match subcommand {
            DebugSubcommand::AppServer(cmd) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "debug app-server",
                )?;
                run_debug_app_server_command(cmd).await?;
            }
            DebugSubcommand::ClearMemories => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "debug clear-memories",
                )?;
                run_debug_clear_memories_command(&root_config_overrides, &interactive).await?;
            }
        },
        Some(Subcommand::Execpolicy(ExecpolicyCommand { sub })) => match sub {
            ExecpolicySubcommand::Check(cmd) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "execpolicy check",
                )?;
                run_execpolicycheck(cmd)?
            }
        },
        Some(Subcommand::Apply(mut apply_cli)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "apply",
            )?;
            prepend_config_flags(
                &mut apply_cli.config_overrides,
                root_config_overrides.clone(),
            );
            run_apply_command(apply_cli, /*cwd*/ None).await?;
        }
        Some(Subcommand::ResponsesApiProxy(args)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "responses-api-proxy",
            )?;
            tokio::task::spawn_blocking(move || codex_responses_api_proxy::run_main(args))
                .await??;
        }
        Some(Subcommand::StdioToUds(cmd)) => {
            reject_remote_mode_for_subcommand(
                root_remote.as_deref(),
                root_remote_auth_token_env.as_deref(),
                "stdio-to-uds",
            )?;
            let socket_path = cmd.socket_path;
            tokio::task::spawn_blocking(move || codex_stdio_to_uds::run(socket_path.as_path()))
                .await??;
        }
        Some(Subcommand::Features(FeaturesCli { sub })) => match sub {
            FeaturesSubcommand::List => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "features list",
                )?;
                // Respect root-level `-c` overrides plus top-level flags like `--profile`.
                let mut cli_kv_overrides = root_config_overrides
                    .parse_overrides()
                    .map_err(anyhow::Error::msg)?;

                // Honor `--search` via the canonical web_search mode.
                if interactive.web_search {
                    cli_kv_overrides.push((
                        "web_search".to_string(),
                        toml::Value::String("live".to_string()),
                    ));
                }

                // Thread through relevant top-level flags (at minimum, `--profile`).
                let overrides = ConfigOverrides {
                    config_profile: interactive.config_profile.clone(),
                    ..Default::default()
                };

                let config = Config::load_with_cli_overrides_and_harness_overrides(
                    cli_kv_overrides,
                    overrides,
                )
                .await?;
                let mut rows = Vec::with_capacity(FEATURES.len());
                let mut name_width = 0;
                let mut stage_width = 0;
                for def in FEATURES {
                    let name = def.key;
                    let stage = stage_str(def.stage);
                    let enabled = config.features.enabled(def.id);
                    name_width = name_width.max(name.len());
                    stage_width = stage_width.max(stage.len());
                    rows.push((name, stage, enabled));
                }
                rows.sort_unstable_by_key(|(name, _, _)| *name);

                for (name, stage, enabled) in rows {
                    println!("{name:<name_width$}  {stage:<stage_width$}  {enabled}");
                }
            }
            FeaturesSubcommand::Enable(FeatureSetArgs { feature }) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "features enable",
                )?;
                enable_feature_in_config(&interactive, &feature).await?;
            }
            FeaturesSubcommand::Disable(FeatureSetArgs { feature }) => {
                reject_remote_mode_for_subcommand(
                    root_remote.as_deref(),
                    root_remote_auth_token_env.as_deref(),
                    "features disable",
                )?;
                disable_feature_in_config(&interactive, &feature).await?;
            }
        },
    }

    Ok(())
}

async fn enable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, /*enabled*/ true)
        .apply()
        .await?;
    println!("Enabled feature `{feature}` in config.toml.");
    maybe_print_under_development_feature_warning(&codex_home, interactive, feature);
    Ok(())
}

async fn disable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, /*enabled*/ false)
        .apply()
        .await?;
    println!("Disabled feature `{feature}` in config.toml.");
    Ok(())
}

fn maybe_print_under_development_feature_warning(
    codex_home: &std::path::Path,
    interactive: &TuiCli,
    feature: &str,
) {
    if interactive.config_profile.is_some() {
        return;
    }

    let Some(spec) = FEATURES.iter().find(|spec| spec.key == feature) else {
        return;
    };
    if !matches!(spec.stage, Stage::UnderDevelopment) {
        return;
    }

    let config_path = codex_home.join(codex_config::CONFIG_TOML_FILE);
    eprintln!(
        "Under-development features enabled: {feature}. Under-development features are incomplete and may behave unpredictably. To suppress this warning, set `suppress_unstable_features_warning = true` in {}.",
        config_path.display()
    );
}

async fn run_debug_clear_memories_command(
    root_config_overrides: &CliConfigOverrides,
    interactive: &TuiCli,
) -> anyhow::Result<()> {
    let cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    let overrides = ConfigOverrides {
        config_profile: interactive.config_profile.clone(),
        ..Default::default()
    };
    let config =
        Config::load_with_cli_overrides_and_harness_overrides(cli_kv_overrides, overrides).await?;

    let state_path = state_db_path(config.sqlite_home.as_path());
    let mut cleared_state_db = false;
    if tokio::fs::try_exists(&state_path).await? {
        let state_db =
            StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                .await?;
        state_db.reset_memory_data_for_fresh_start().await?;
        cleared_state_db = true;
    }

    let memory_root = config.codex_home.join("memories");
    let removed_memory_root = match tokio::fs::remove_dir_all(&memory_root).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err.into()),
    };

    let mut message = if cleared_state_db {
        format!("Cleared memory state from {}.", state_path.display())
    } else {
        format!("No state db found at {}.", state_path.display())
    };

    if removed_memory_root {
        message.push_str(&format!(" Removed {}.", memory_root.display()));
    } else {
        message.push_str(&format!(
            " No memory directory found at {}.",
            memory_root.display()
        ));
    }

    println!("{message}");

    Ok(())
}

/// Prepend root-level overrides so they have lower precedence than
/// CLI-specific ones specified after the subcommand (if any).
fn prepend_config_flags(
    subcommand_config_overrides: &mut CliConfigOverrides,
    cli_config_overrides: CliConfigOverrides,
) {
    subcommand_config_overrides
        .raw_overrides
        .splice(0..0, cli_config_overrides.raw_overrides);
}

fn reject_remote_mode_for_subcommand(
    remote: Option<&str>,
    remote_auth_token_env: Option<&str>,
    subcommand: &str,
) -> anyhow::Result<()> {
    if let Some(remote) = remote {
        anyhow::bail!(
            "`--remote {remote}` is only supported for interactive TUI commands, not `codex {subcommand}`"
        );
    }
    if remote_auth_token_env.is_some() {
        anyhow::bail!(
            "`--remote-auth-token-env` is only supported for interactive TUI commands, not `codex {subcommand}`"
        );
    }
    Ok(())
}

fn reject_remote_mode_for_app_server_subcommand(
    remote: Option<&str>,
    remote_auth_token_env: Option<&str>,
    subcommand: Option<&AppServerSubcommand>,
) -> anyhow::Result<()> {
    let subcommand_name = match subcommand {
        None => "app-server",
        Some(AppServerSubcommand::GenerateTs(_)) => "app-server generate-ts",
        Some(AppServerSubcommand::GenerateJsonSchema(_)) => "app-server generate-json-schema",
        Some(AppServerSubcommand::GenerateInternalJsonSchema(_)) => {
            "app-server generate-internal-json-schema"
        }
    };
    reject_remote_mode_for_subcommand(remote, remote_auth_token_env, subcommand_name)
}

fn read_remote_auth_token_from_env_var_with<F>(
    env_var_name: &str,
    get_var: F,
) -> anyhow::Result<String>
where
    F: FnOnce(&str) -> Result<String, std::env::VarError>,
{
    let auth_token = get_var(env_var_name)
        .map_err(|_| anyhow::anyhow!("environment variable `{env_var_name}` is not set"))?;
    let auth_token = auth_token.trim().to_string();
    if auth_token.is_empty() {
        anyhow::bail!("environment variable `{env_var_name}` is empty");
    }
    Ok(auth_token)
}

fn read_remote_auth_token_from_env_var(env_var_name: &str) -> anyhow::Result<String> {
    read_remote_auth_token_from_env_var_with(env_var_name, |name| std::env::var(name))
}

fn is_status_shortcut_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    trimmed == "/status" || trimmed == "status"
}

async fn resolve_fork_rollout_path(session_id: &str) -> anyhow::Result<Option<PathBuf>> {
    let codex_home = find_codex_home().map_err(anyhow::Error::from)?;
    if ThreadId::from_string(session_id).is_ok() {
        find_thread_path_by_id_str(codex_home.as_path(), session_id)
            .await
            .map_err(anyhow::Error::from)
    } else {
        find_thread_path_by_name_str(codex_home.as_path(), session_id)
            .await
            .map_err(anyhow::Error::from)
    }
}

async fn load_user_prompt_points(rollout_path: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let history = RolloutRecorder::get_rollout_history(rollout_path)
        .await
        .map_err(anyhow::Error::from)?;
    let items = history.get_rollout_items();
    let mut prompts = Vec::new();
    for item in items {
        match item {
            RolloutItem::ResponseItem(response_item) => {
                if let Some(TurnItem::UserMessage(user_message)) = parse_turn_item(&response_item) {
                    prompts.push(user_message.message());
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                let num_turns = usize::try_from(rollback.num_turns).unwrap_or(usize::MAX);
                let new_len = prompts.len().saturating_sub(num_turns);
                prompts.truncate(new_len);
            }
            RolloutItem::SessionMeta(_)
            | RolloutItem::Compacted(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::EventMsg(_) => {}
        }
    }
    Ok(prompts)
}

async fn load_thread_item_by_rollout_path(
    rollout_path: &std::path::Path,
) -> anyhow::Result<Option<codex_core::ThreadItem>> {
    let codex_home = find_codex_home().map_err(anyhow::Error::from)?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .build()
        .await
        .map_err(anyhow::Error::from)?;

    let mut cursor = None;
    loop {
        let page = RolloutRecorder::list_threads(
            &config,
            /*page_size*/ 100,
            cursor.as_ref(),
            ThreadSortKey::UpdatedAt,
            INTERACTIVE_SESSION_SOURCES.as_slice(),
            /*model_providers*/ None,
            &config.model_provider_id,
            /*search_term*/ None,
        )
        .await
        .map_err(anyhow::Error::from)?;
        if let Some(item) = page
            .items
            .into_iter()
            .find(|item| item.path.as_path() == rollout_path)
        {
            return Ok(Some(item));
        }
        if page.next_cursor.is_none() {
            return Ok(None);
        }
        cursor = page.next_cursor;
    }
}

async fn print_resume_sessions_non_interactive(
    show_all: bool,
    include_non_interactive: bool,
) -> anyhow::Result<()> {
    let codex_home = find_codex_home().map_err(anyhow::Error::from)?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .build()
        .await
        .map_err(anyhow::Error::from)?;

    let provider_filter = vec![config.model_provider_id.clone()];
    let allowed_sources = if include_non_interactive {
        &[][..]
    } else {
        INTERACTIVE_SESSION_SOURCES.as_slice()
    };
    let filter_cwd = if show_all {
        None
    } else {
        Some(config.cwd.as_path())
    };

    let mut cursor = None;
    loop {
        let page = RolloutRecorder::list_threads(
            &config,
            /*page_size*/ 100,
            cursor.as_ref(),
            ThreadSortKey::UpdatedAt,
            allowed_sources,
            Some(provider_filter.as_slice()),
            &config.model_provider_id,
            /*search_term*/ None,
        )
        .await
        .map_err(anyhow::Error::from)?;

        for item in page.items.into_iter().filter(|item| {
            filter_cwd.is_none()
                || item
                    .cwd
                    .as_deref()
                    .is_some_and(|cwd| cwd == config.cwd.as_path())
        }) {
            let thread = item
                .thread_id
                .map(|thread_id| thread_id.to_string())
                .unwrap_or_else(|| "-".to_string());
            let updated_at = item.updated_at.unwrap_or_else(|| "-".to_string());
            let cwd = item
                .cwd
                .as_deref()
                .map(|cwd| cwd.display().to_string())
                .unwrap_or_else(|| "-".to_string());
            let preview = item.first_user_message.unwrap_or_else(|| "-".to_string());
            println!("{thread}\t{updated_at}\t{cwd}\t{preview}");
        }

        if page.next_cursor.is_none() {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StatusOutputMode {
    compact: bool,
    utc: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StatusInvocationOptions {
    output_mode: StatusOutputMode,
    auth_file_override: Option<PathBuf>,
    rate_limit_mode: CliStatusRateLimitMode,
}

fn parse_status_invocation_options(
    raw_argv: &[String],
    default_rate_limit_mode: CliStatusRateLimitMode,
    trailing_args: &[String],
) -> anyhow::Result<StatusInvocationOptions> {
    let Some(status_index) = raw_argv.iter().position(|arg| arg == "status") else {
        if trailing_args.is_empty() {
            return Ok(StatusInvocationOptions {
                rate_limit_mode: default_rate_limit_mode,
                ..StatusInvocationOptions::default()
            });
        }
        anyhow::bail!("Unknown arguments for `codex status`.");
    };
    let has_double_dash = raw_argv[status_index + 1..].iter().any(|arg| arg == "--");
    if !has_double_dash {
        if trailing_args.is_empty() {
            return Ok(StatusInvocationOptions {
                rate_limit_mode: default_rate_limit_mode,
                ..StatusInvocationOptions::default()
            });
        }
        let provided = trailing_args.join(" ");
        anyhow::bail!(
            "Unknown arguments for `codex status`: {provided}. Use `codex status [--cached]` or `codex status -- [--utc] [--cached]`."
        );
    }

    let mut options = StatusInvocationOptions {
        output_mode: StatusOutputMode {
            compact: true,
            utc: false,
        },
        auth_file_override: None,
        rate_limit_mode: default_rate_limit_mode,
    };
    let mut i = 0usize;
    while i < trailing_args.len() {
        match trailing_args[i].as_str() {
            "--utc" => {
                options.output_mode.utc = true;
            }
            "--cached" => {
                options.rate_limit_mode = CliStatusRateLimitMode::AllowCached;
            }
            flag if flag.starts_with("--auth-file=") => {
                let value = flag.trim_start_matches("--auth-file=");
                if !value.is_empty() {
                    options.auth_file_override = Some(PathBuf::from(value));
                }
            }
            "--auth-file" => {
                if let Some(value) = trailing_args.get(i + 1) {
                    options.auth_file_override = Some(PathBuf::from(value));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    Ok(options)
}

async fn run_status_command(
    root_config_overrides: &CliConfigOverrides,
    interactive: &TuiCli,
    status_output_mode: StatusOutputMode,
    rate_limit_mode: CliStatusRateLimitMode,
) -> anyhow::Result<()> {
    let mut cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    if interactive.web_search {
        cli_kv_overrides.push((
            "web_search".to_string(),
            toml::Value::String("live".to_string()),
        ));
    }

    let overrides = ConfigOverrides {
        config_profile: interactive.config_profile.clone(),
        ..Default::default()
    };
    let config =
        Config::load_with_cli_overrides_and_harness_overrides(cli_kv_overrides, overrides).await?;
    let auth_manager = std::sync::Arc::new(AuthManager::new(
        config.codex_home.clone(),
        /*enable_codex_api_key_env*/ false,
        config.cli_auth_credentials_store_mode,
    ));
    let (auth, compact_output_mode) = match auth_manager.auth_with_refresh_if_expired_strict().await
    {
        Ok(auth) => (auth, codex_tui::CompactStatusOutputMode::Normal),
        Err(err) => {
            tracing::warn!("proactive auth refresh failed: {err}");
            let compact_mode = match err {
                RefreshTokenError::Permanent(_) => codex_tui::CompactStatusOutputMode::UnknownUsage,
                RefreshTokenError::Transient(_) => codex_tui::CompactStatusOutputMode::Normal,
            };
            (auth_manager.auth_cached(), compact_mode)
        }
    };
    if status_output_mode.compact {
        let compact_line = codex_tui::render_compact_status_for_cli(
            &config,
            auth.as_ref(),
            status_output_mode.utc,
            compact_output_mode,
            rate_limit_mode,
        )
        .await;
        println!("{compact_line}");
        return Ok(());
    }

    let model_name = config.model.as_deref().unwrap_or("<unknown>");
    let lines = codex_tui::render_status_for_cli(
        &config,
        auth,
        model_name,
        /*width*/ 80,
        rate_limit_mode,
    )
    .await;
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

async fn run_usage_clear_command(
    root_config_overrides: &CliConfigOverrides,
    interactive: &TuiCli,
    clear_command: UsageClearCommand,
) -> anyhow::Result<()> {
    let cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    let overrides = ConfigOverrides {
        config_profile: interactive.config_profile.clone(),
        ..Default::default()
    };
    let config =
        Config::load_with_cli_overrides_and_harness_overrides(cli_kv_overrides, overrides).await?;
    let usage_path = usage_db_path(config.sqlite_home.as_path());
    let current_account = if clear_command.all_accounts {
        None
    } else {
        let auth_manager = AuthManager::new(
            config.codex_home.clone(),
            /*enable_codex_api_key_env*/ false,
            config.cli_auth_credentials_store_mode,
        );
        let auth = auth_manager.auth().await;
        let account_id = auth
            .as_ref()
            .and_then(|auth| {
                account_usage_key(
                    auth.get_account_id().as_deref(),
                    auth.get_account_email().as_deref(),
                )
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No current account ID could be resolved from active credentials. Use `codex usage clear --all-accounts` to clear all locally tracked accounts."
                )
            })?;
        let account_email = auth.as_ref().and_then(|auth| auth.get_account_email());
        Some((account_id, account_email))
    };

    if !clear_command.yes {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to clear usage data without confirmation in a non-interactive shell. Re-run with `--yes`."
            );
        }

        let scope = if clear_command.all_accounts {
            "all accounts".to_string()
        } else if let Some((_, Some(account_email))) = current_account.as_ref() {
            format!("the account `{account_email}`")
        } else {
            "the current account".to_string()
        };
        let provider = config.model_provider_id.as_str();
        let prompt = format!(
            "This will clear local usage tracking for {scope} on provider `{provider}` from {}. Continue? [y/N]: ",
            usage_path.display()
        );
        if !confirm(&prompt)? {
            eprintln!("Usage clear aborted.");
            return Ok(());
        }
    }

    let usage_store = codex_state::AccountUsageStore::init_with_estimator_config(
        config.sqlite_home.clone(),
        config.model_provider_id,
        codex_state::AccountUsageEstimatorConfig {
            min_usage_pct_sample_count: config.account_usage_estimator.min_usage_pct_sample_count,
            max_usage_pct_display_percent_before_full: config
                .account_usage_estimator
                .max_usage_pct_display_percent_before_full,
            stable_backend_percent_window: config
                .account_usage_estimator
                .stable_backend_percent_window,
        },
    )
    .await?;

    let (usage_rows_deleted, sample_rows_deleted, scope) = if clear_command.all_accounts {
        let (usage_rows_deleted, sample_rows_deleted) =
            usage_store.clear_usage_for_all_accounts().await?;
        (
            usage_rows_deleted,
            sample_rows_deleted,
            "all accounts".to_string(),
        )
    } else {
        let (account_id, account_email) = current_account
            .ok_or_else(|| anyhow::anyhow!("Missing current account resolution state."))?;
        let (usage_rows_deleted, sample_rows_deleted) = usage_store
            .clear_usage_for_account(account_id.as_str())
            .await?;
        let scope = if let Some(account_email) = account_email {
            format!("account `{account_email}`")
        } else {
            format!("account `{account_id}`")
        };
        (usage_rows_deleted, sample_rows_deleted, scope)
    };

    println!(
        "Cleared usage tracking for {scope} from {} (account_usage rows: {usage_rows_deleted}, account_usage_samples rows: {sample_rows_deleted}).",
        usage_path.display()
    );

    Ok(())
}

async fn run_interactive_tui(
    mut interactive: TuiCli,
    remote: Option<String>,
    remote_auth_token_env: Option<String>,
    arg0_paths: Arg0DispatchPaths,
) -> std::io::Result<AppExitInfo> {
    if let Some(prompt) = interactive.prompt.take() {
        // Normalize CRLF/CR to LF so CLI-provided text can't leak `\r` into TUI state.
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    let terminal_info = codex_terminal_detection::terminal_info();
    if terminal_info.name == TerminalName::Dumb {
        if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
            return Ok(AppExitInfo::fatal(
                "TERM is set to \"dumb\". Refusing to start the interactive TUI because no terminal is available for a confirmation prompt (stdin/stderr is not a TTY). Run in a supported terminal or unset TERM.",
            ));
        }

        eprintln!(
            "WARNING: TERM is set to \"dumb\". Codex's interactive TUI may not work in this terminal."
        );
        if !confirm("Continue anyway? [y/N]: ")? {
            return Ok(AppExitInfo::fatal(
                "Refusing to start the interactive TUI because TERM is set to \"dumb\". Run in a supported terminal or unset TERM.",
            ));
        }
    }

    let normalized_remote = remote
        .as_deref()
        .map(codex_tui::normalize_remote_addr)
        .transpose()
        .map_err(std::io::Error::other)?;
    if remote_auth_token_env.is_some() && normalized_remote.is_none() {
        return Ok(AppExitInfo::fatal(
            "`--remote-auth-token-env` requires `--remote`.",
        ));
    }
    let remote_auth_token = remote_auth_token_env
        .as_deref()
        .map(read_remote_auth_token_from_env_var)
        .transpose()
        .map_err(std::io::Error::other)?;
    codex_tui::run_main(
        interactive,
        arg0_paths,
        codex_core::config_loader::LoaderOverrides::default(),
        normalized_remote,
        remote_auth_token,
    )
    .await
}

fn confirm(prompt: &str) -> std::io::Result<bool> {
    eprintln!("{prompt}");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let answer = input.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

/// Build the final `TuiCli` for a `codex resume` invocation.
fn finalize_resume_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    include_non_interactive: bool,
    resume_cli: TuiCli,
) -> TuiCli {
    // Start with the parsed interactive CLI so resume shares the same
    // configuration surface area as `codex` without additional flags.
    let resume_session_id = session_id;
    interactive.resume_picker = resume_session_id.is_none() && !last;
    interactive.resume_last = last;
    interactive.resume_session_id = resume_session_id;
    interactive.resume_show_all = show_all;
    interactive.resume_include_non_interactive = include_non_interactive;

    // Merge resume-scoped flags and overrides with highest precedence.
    merge_interactive_cli_flags(&mut interactive, resume_cli);

    // Propagate any root-level config overrides (e.g. `-c key=value`).
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

fn resolve_resume_include_non_interactive(
    include_non_interactive: bool,
    exclude_non_interactive: bool,
) -> bool {
    include_non_interactive && !exclude_non_interactive
}

/// Build the final `TuiCli` for a `codex fork` invocation.
fn finalize_fork_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    fork_nth_user_message: Option<usize>,
    fork_cli: TuiCli,
) -> TuiCli {
    // Start with the parsed interactive CLI so fork shares the same
    // configuration surface area as `codex` without additional flags.
    let fork_session_id = session_id;
    interactive.fork_picker = fork_session_id.is_none() && !last;
    interactive.fork_last = last;
    interactive.fork_session_id = fork_session_id;
    interactive.fork_show_all = show_all;
    interactive.fork_nth_user_message = fork_nth_user_message;

    // Merge fork-scoped flags and overrides with highest precedence.
    merge_interactive_cli_flags(&mut interactive, fork_cli);

    // Propagate any root-level config overrides (e.g. `-c key=value`).
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

/// Merge flags provided to `codex resume`/`codex fork` so they take precedence over any
/// root-level flags. Only overrides fields explicitly set on the subcommand-scoped
/// CLI. Also appends `-c key=value` overrides with highest precedence.
fn merge_interactive_cli_flags(interactive: &mut TuiCli, subcommand_cli: TuiCli) {
    if let Some(model) = subcommand_cli.model {
        interactive.model = Some(model);
    }
    if subcommand_cli.oss {
        interactive.oss = true;
    }
    if let Some(profile) = subcommand_cli.config_profile {
        interactive.config_profile = Some(profile);
    }
    if let Some(sandbox) = subcommand_cli.sandbox_mode {
        interactive.sandbox_mode = Some(sandbox);
    }
    if let Some(approval) = subcommand_cli.approval_policy {
        interactive.approval_policy = Some(approval);
    }
    if subcommand_cli.full_auto {
        interactive.full_auto = true;
    }
    if subcommand_cli.dangerously_bypass_approvals_and_sandbox {
        interactive.dangerously_bypass_approvals_and_sandbox = true;
    }
    if let Some(cwd) = subcommand_cli.cwd {
        interactive.cwd = Some(cwd);
    }
    if subcommand_cli.web_search {
        interactive.web_search = true;
    }
    if subcommand_cli.bare_prompt {
        interactive.bare_prompt = true;
    }
    if !subcommand_cli.images.is_empty() {
        interactive.images = subcommand_cli.images;
    }
    if !subcommand_cli.add_dir.is_empty() {
        interactive.add_dir.extend(subcommand_cli.add_dir);
    }
    if let Some(prompt) = subcommand_cli.prompt {
        // Normalize CRLF/CR to LF so CLI-provided text can't leak `\r` into TUI state.
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    interactive
        .config_overrides
        .raw_overrides
        .extend(subcommand_cli.config_overrides.raw_overrides);
}

fn print_completion(cmd: CompletionCommand) {
    let mut app = MultitoolCli::command();
    let name = "codex";
    generate(cmd.shell, &mut app, name, &mut std::io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::TokenUsage;
    use pretty_assertions::assert_eq;

    fn finalize_resume_from_args(args: &[&str]) -> TuiCli {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let MultitoolCli {
            interactive,
            config_overrides: root_overrides,
            subcommand,
            feature_toggles: _,
            remote: _,
            auth_file: _,
        } = cli;

        let Subcommand::Resume(ResumeCommand {
            session_id,
            last,
            all,
            include_non_interactive,
            exclude_non_interactive,
            remote: _,
            config_overrides: resume_cli,
        }) = subcommand.expect("resume present")
        else {
            unreachable!()
        };
        let include_non_interactive = resolve_resume_include_non_interactive(
            include_non_interactive,
            exclude_non_interactive,
        );

        finalize_resume_interactive(
            interactive,
            root_overrides,
            session_id,
            last,
            all,
            include_non_interactive,
            resume_cli,
        )
    }

    fn finalize_fork_from_args(args: &[&str]) -> TuiCli {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let MultitoolCli {
            interactive,
            config_overrides: root_overrides,
            subcommand,
            feature_toggles: _,
            remote: _,
            auth_file: _,
        } = cli;

        let Subcommand::Fork(ForkCommand {
            session_id,
            last,
            all,
            show: _,
            pick: _,
            remote: _,
            config_overrides: fork_cli,
        }) = subcommand.expect("fork present")
        else {
            unreachable!()
        };

        finalize_fork_interactive(
            interactive,
            root_overrides,
            session_id,
            last,
            all,
            None,
            fork_cli,
        )
    }

    #[test]
    fn status_shortcut_prompt_matches_expected_values() {
        assert_eq!(is_status_shortcut_prompt("/status"), true);
        assert_eq!(is_status_shortcut_prompt("status"), true);
        assert_eq!(is_status_shortcut_prompt(" /status "), true);
    }

    #[test]
    fn status_shortcut_prompt_rejects_other_inputs() {
        assert_eq!(is_status_shortcut_prompt("/status now"), false);
        assert_eq!(is_status_shortcut_prompt("status please"), false);
        assert_eq!(is_status_shortcut_prompt("/model"), false);
    }

    #[test]
    fn status_output_mode_defaults_without_double_dash() {
        let options = parse_status_invocation_options(
            &["codex".to_string(), "status".to_string()],
            CliStatusRateLimitMode::LiveOnly,
            &[],
        )
        .expect("status mode");
        assert_eq!(options.output_mode, StatusOutputMode::default());
        assert_eq!(options.auth_file_override, None);
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::LiveOnly);
    }

    #[test]
    fn status_output_mode_compact_with_double_dash() {
        let options = parse_status_invocation_options(
            &["codex".to_string(), "status".to_string(), "--".to_string()],
            CliStatusRateLimitMode::LiveOnly,
            &[],
        )
        .expect("status mode");
        assert_eq!(
            options.output_mode,
            StatusOutputMode {
                compact: true,
                utc: false
            }
        );
        assert_eq!(options.auth_file_override, None);
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::LiveOnly);
    }

    #[test]
    fn status_output_mode_compact_with_utc() {
        let options = parse_status_invocation_options(
            &[
                "codex".to_string(),
                "status".to_string(),
                "--".to_string(),
                "--utc".to_string(),
            ],
            CliStatusRateLimitMode::LiveOnly,
            &["--utc".to_string()],
        )
        .expect("status mode");
        assert_eq!(
            options.output_mode,
            StatusOutputMode {
                compact: true,
                utc: true
            }
        );
        assert_eq!(options.auth_file_override, None);
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::LiveOnly);
    }

    #[test]
    fn status_output_mode_compact_ignores_other_trailing_args() {
        let options = parse_status_invocation_options(
            &[
                "codex".to_string(),
                "status".to_string(),
                "--".to_string(),
                "--auth-file=roger@gmail.com".to_string(),
                "--utc".to_string(),
            ],
            CliStatusRateLimitMode::LiveOnly,
            &[
                "--auth-file=roger@gmail.com".to_string(),
                "--utc".to_string(),
            ],
        )
        .expect("status mode");
        assert_eq!(
            options.output_mode,
            StatusOutputMode {
                compact: true,
                utc: true
            }
        );
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::LiveOnly);
    }

    #[test]
    fn status_output_mode_compact_parses_auth_file() {
        let options = parse_status_invocation_options(
            &[
                "codex".to_string(),
                "status".to_string(),
                "--".to_string(),
                "--auth-file=/tmp/alt-auth.json".to_string(),
            ],
            CliStatusRateLimitMode::LiveOnly,
            &["--auth-file=/tmp/alt-auth.json".to_string()],
        )
        .expect("status mode");
        assert_eq!(
            options.auth_file_override,
            Some(std::path::PathBuf::from("/tmp/alt-auth.json"))
        );
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::LiveOnly);
    }

    #[test]
    fn status_output_mode_compact_parses_cached_flag() {
        let options = parse_status_invocation_options(
            &[
                "codex".to_string(),
                "status".to_string(),
                "--".to_string(),
                "--cached".to_string(),
            ],
            CliStatusRateLimitMode::LiveOnly,
            &["--cached".to_string()],
        )
        .expect("status mode");
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::AllowCached);
    }

    #[test]
    fn status_output_mode_uses_cached_flag_from_subcommand() {
        let options = parse_status_invocation_options(
            &[
                "codex".to_string(),
                "status".to_string(),
                "--cached".to_string(),
            ],
            CliStatusRateLimitMode::AllowCached,
            &[],
        )
        .expect("status mode");
        assert_eq!(options.rate_limit_mode, CliStatusRateLimitMode::AllowCached);
    }

    #[test]
    fn status_subcommand_accepts_trailing_args_after_double_dash() {
        let cli = MultitoolCli::try_parse_from(["codex", "status", "--", "--utc"]).expect("parse");
        let Some(Subcommand::Status(StatusCommand {
            cached,
            trailing_args,
        })) = cli.subcommand
        else {
            panic!("expected status subcommand");
        };
        assert_eq!(cached, false);
        assert_eq!(trailing_args, vec!["--utc".to_string()]);
    }

    #[test]
    fn status_subcommand_accepts_cached_flag() {
        let cli = MultitoolCli::try_parse_from(["codex", "status", "--cached"]).expect("parse");
        let Some(Subcommand::Status(StatusCommand {
            cached,
            trailing_args,
        })) = cli.subcommand
        else {
            panic!("expected status subcommand");
        };
        assert_eq!(cached, true);
        assert_eq!(trailing_args, Vec::<String>::new());
    }

    #[test]
    fn root_prompt_is_rejected_outside_exec() {
        let err =
            MultitoolCli::try_parse_from(["codex", "hello world"]).expect("parse should succeed");
        let MultitoolCli {
            subcommand,
            interactive,
            ..
        } = err;
        assert!(subcommand.is_none());
        assert_eq!(interactive.prompt.as_deref(), Some("hello world"));
    }

    #[test]
    fn exec_resume_last_accepts_prompt_positional() {
        let cli =
            MultitoolCli::try_parse_from(["codex", "exec", "--json", "resume", "--last", "2+2"])
                .expect("parse should succeed");

        let Some(Subcommand::Exec(exec)) = cli.subcommand else {
            panic!("expected exec subcommand");
        };
        let Some(codex_exec::Command::Resume(args)) = exec.command else {
            panic!("expected exec resume");
        };

        assert!(args.last);
        assert_eq!(args.session_id, None);
        assert_eq!(args.prompt.as_deref(), Some("2+2"));
    }

    #[test]
    fn exec_resume_accepts_output_last_message_flag_after_subcommand() {
        let cli = MultitoolCli::try_parse_from([
            "codex",
            "exec",
            "resume",
            "session-123",
            "-o",
            "/tmp/resume-output.md",
            "re-review",
        ])
        .expect("parse should succeed");

        let Some(Subcommand::Exec(exec)) = cli.subcommand else {
            panic!("expected exec subcommand");
        };
        let Some(codex_exec::Command::Resume(args)) = exec.command else {
            panic!("expected exec resume");
        };

        assert_eq!(
            exec.last_message_file,
            Some(std::path::PathBuf::from("/tmp/resume-output.md"))
        );
        assert_eq!(args.session_id.as_deref(), Some("session-123"));
        assert_eq!(args.prompt.as_deref(), Some("re-review"));
    }

    fn app_server_from_args(args: &[&str]) -> AppServerCommand {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let Subcommand::AppServer(app_server) = cli.subcommand.expect("app-server present") else {
            unreachable!()
        };
        app_server
    }

    fn sample_exit_info(conversation_id: Option<&str>, thread_name: Option<&str>) -> AppExitInfo {
        let token_usage = TokenUsage {
            output_tokens: 2,
            total_tokens: 2,
            ..Default::default()
        };
        AppExitInfo {
            token_usage,
            thread_id: conversation_id
                .map(ThreadId::from_string)
                .map(Result::unwrap),
            thread_name: thread_name.map(str::to_string),
            update_action: None,
            exit_reason: ExitReason::UserRequested,
        }
    }

    #[test]
    fn format_exit_messages_skips_zero_usage() {
        let exit_info = AppExitInfo {
            token_usage: TokenUsage::default(),
            thread_id: None,
            thread_name: None,
            update_action: None,
            exit_reason: ExitReason::UserRequested,
        };
        let lines = format_exit_messages(exit_info, /*color_enabled*/ false);
        assert!(lines.is_empty());
    }

    #[test]
    fn format_exit_messages_includes_resume_hint_without_color() {
        let exit_info = sample_exit_info(
            Some("123e4567-e89b-12d3-a456-426614174000"),
            /*thread_name*/ None,
        );
        let lines = format_exit_messages(exit_info, /*color_enabled*/ false);
        assert_eq!(
            lines,
            vec![
                "Token usage: total=2 input=0 output=2".to_string(),
                "To continue this session, run codex resume 123e4567-e89b-12d3-a456-426614174000"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn format_exit_messages_applies_color_when_enabled() {
        let exit_info = sample_exit_info(
            Some("123e4567-e89b-12d3-a456-426614174000"),
            /*thread_name*/ None,
        );
        let lines = format_exit_messages(exit_info, /*color_enabled*/ true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\u{1b}[36m"));
    }

    #[test]
    fn format_exit_messages_prefers_thread_name() {
        let exit_info = sample_exit_info(
            Some("123e4567-e89b-12d3-a456-426614174000"),
            Some("my-thread"),
        );
        let lines = format_exit_messages(exit_info, /*color_enabled*/ false);
        assert_eq!(
            lines,
            vec![
                "Token usage: total=2 input=0 output=2".to_string(),
                "To continue this session, run codex resume my-thread".to_string(),
            ]
        );
    }

    #[test]
    fn resume_model_flag_applies_when_no_root_flags() {
        let interactive =
            finalize_resume_from_args(["codex", "resume", "-m", "gpt-5.1-test"].as_ref());

        assert_eq!(interactive.model.as_deref(), Some("gpt-5.1-test"));
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
    }

    #[test]
    fn resume_picker_logic_none_and_not_last() {
        let interactive = finalize_resume_from_args(["codex", "resume"].as_ref());
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
        assert!(!interactive.resume_show_all);
        assert!(interactive.resume_include_non_interactive);
    }

    #[test]
    fn resume_picker_logic_last() {
        let interactive = finalize_resume_from_args(["codex", "resume", "--last"].as_ref());
        assert!(!interactive.resume_picker);
        assert!(interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
        assert!(!interactive.resume_show_all);
    }

    #[test]
    fn resume_picker_logic_with_session_id() {
        let interactive = finalize_resume_from_args(["codex", "resume", "1234"].as_ref());
        assert!(!interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id.as_deref(), Some("1234"));
        assert!(!interactive.resume_show_all);
    }

    #[test]
    fn resume_all_flag_sets_show_all() {
        let interactive = finalize_resume_from_args(["codex", "resume", "--all"].as_ref());
        assert!(interactive.resume_picker);
        assert!(interactive.resume_show_all);
    }

    #[test]
    fn resume_include_non_interactive_flag_sets_source_filter_override() {
        let interactive =
            finalize_resume_from_args(["codex", "resume", "--include-non-interactive"].as_ref());

        assert!(interactive.resume_picker);
        assert!(interactive.resume_include_non_interactive);
    }

    #[test]
    fn resume_exclude_non_interactive_flag_sets_source_filter_override() {
        let interactive =
            finalize_resume_from_args(["codex", "resume", "--exclude-non-interactive"].as_ref());

        assert!(interactive.resume_picker);
        assert!(!interactive.resume_include_non_interactive);
    }

    #[test]
    fn resume_last_non_interactive_flag_wins_when_both_are_passed() {
        let interactive = finalize_resume_from_args(
            [
                "codex",
                "resume",
                "--exclude-non-interactive",
                "--include-non-interactive",
            ]
            .as_ref(),
        );
        assert!(interactive.resume_include_non_interactive);
    }

    #[test]
    fn resume_merges_option_flags_and_full_auto() {
        let interactive = finalize_resume_from_args(
            [
                "codex",
                "resume",
                "sid",
                "--oss",
                "--full-auto",
                "--search",
                "--bare-prompt",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "-m",
                "gpt-5.1-test",
                "-p",
                "my-profile",
                "-C",
                "/tmp",
                "-i",
                "/tmp/a.png,/tmp/b.png",
            ]
            .as_ref(),
        );

        assert_eq!(interactive.model.as_deref(), Some("gpt-5.1-test"));
        assert!(interactive.oss);
        assert_eq!(interactive.config_profile.as_deref(), Some("my-profile"));
        assert_matches!(
            interactive.sandbox_mode,
            Some(codex_utils_cli::SandboxModeCliArg::WorkspaceWrite)
        );
        assert_matches!(
            interactive.approval_policy,
            Some(codex_utils_cli::ApprovalModeCliArg::OnRequest)
        );
        assert!(interactive.full_auto);
        assert_eq!(
            interactive.cwd.as_deref(),
            Some(std::path::Path::new("/tmp"))
        );
        assert!(interactive.web_search);
        assert!(interactive.bare_prompt);
        let has_a = interactive
            .images
            .iter()
            .any(|p| p == std::path::Path::new("/tmp/a.png"));
        let has_b = interactive
            .images
            .iter()
            .any(|p| p == std::path::Path::new("/tmp/b.png"));
        assert!(has_a && has_b);
        assert!(!interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id.as_deref(), Some("sid"));
    }

    #[test]
    fn resume_merges_dangerously_bypass_flag() {
        let interactive = finalize_resume_from_args(
            [
                "codex",
                "resume",
                "--dangerously-bypass-approvals-and-sandbox",
            ]
            .as_ref(),
        );
        assert!(interactive.dangerously_bypass_approvals_and_sandbox);
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
    }

    #[test]
    fn fork_picker_logic_none_and_not_last() {
        let interactive = finalize_fork_from_args(["codex", "fork"].as_ref());
        assert!(interactive.fork_picker);
        assert!(!interactive.fork_last);
        assert_eq!(interactive.fork_session_id, None);
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_picker_logic_last() {
        let interactive = finalize_fork_from_args(["codex", "fork", "--last"].as_ref());
        assert!(!interactive.fork_picker);
        assert!(interactive.fork_last);
        assert_eq!(interactive.fork_session_id, None);
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_picker_logic_with_session_id() {
        let interactive = finalize_fork_from_args(["codex", "fork", "1234"].as_ref());
        assert!(!interactive.fork_picker);
        assert!(!interactive.fork_last);
        assert_eq!(interactive.fork_session_id.as_deref(), Some("1234"));
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_all_flag_sets_show_all() {
        let interactive = finalize_fork_from_args(["codex", "fork", "--all"].as_ref());
        assert!(interactive.fork_picker);
        assert!(interactive.fork_show_all);
    }

    #[test]
    fn app_server_analytics_default_disabled_without_flag() {
        let app_server = app_server_from_args(["codex", "app-server"].as_ref());
        assert!(!app_server.analytics_default_enabled);
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::Stdio
        );
    }

    #[test]
    fn app_server_analytics_default_enabled_with_flag() {
        let app_server =
            app_server_from_args(["codex", "app-server", "--analytics-default-enabled"].as_ref());
        assert!(app_server.analytics_default_enabled);
    }

    #[test]
    fn remote_flag_parses_for_interactive_root() {
        let cli = MultitoolCli::try_parse_from(["codex", "--remote", "ws://127.0.0.1:4500"])
            .expect("parse");
        assert_eq!(cli.remote.remote.as_deref(), Some("ws://127.0.0.1:4500"));
    }

    #[test]
    fn remote_auth_token_env_flag_parses_for_interactive_root() {
        let cli = MultitoolCli::try_parse_from([
            "codex",
            "--remote-auth-token-env",
            "CODEX_REMOTE_AUTH_TOKEN",
            "--remote",
            "ws://127.0.0.1:4500",
        ])
        .expect("parse");
        assert_eq!(
            cli.remote.remote_auth_token_env.as_deref(),
            Some("CODEX_REMOTE_AUTH_TOKEN")
        );
    }

    #[test]
    fn remote_flag_parses_for_resume_subcommand() {
        let cli =
            MultitoolCli::try_parse_from(["codex", "resume", "--remote", "ws://127.0.0.1:4500"])
                .expect("parse");
        let Subcommand::Resume(ResumeCommand { remote, .. }) =
            cli.subcommand.expect("resume present")
        else {
            panic!("expected resume subcommand");
        };
        assert_eq!(remote.remote.as_deref(), Some("ws://127.0.0.1:4500"));
    }

    #[test]
    fn reject_remote_mode_for_non_interactive_subcommands() {
        let err = reject_remote_mode_for_subcommand(
            Some("127.0.0.1:4500"),
            /*remote_auth_token_env*/ None,
            "exec",
        )
        .expect_err("non-interactive subcommands should reject --remote");
        assert!(
            err.to_string()
                .contains("only supported for interactive TUI commands")
        );
    }

    #[test]
    fn reject_remote_auth_token_env_for_non_interactive_subcommands() {
        let err = reject_remote_mode_for_subcommand(
            /*remote*/ None,
            Some("CODEX_REMOTE_AUTH_TOKEN"),
            "exec",
        )
        .expect_err("non-interactive subcommands should reject --remote-auth-token-env");
        assert!(
            err.to_string()
                .contains("only supported for interactive TUI commands")
        );
    }

    #[test]
    fn reject_remote_auth_token_env_for_app_server_generate_internal_json_schema() {
        let subcommand =
            AppServerSubcommand::GenerateInternalJsonSchema(GenerateInternalJsonSchemaCommand {
                out_dir: PathBuf::from("/tmp/out"),
            });
        let err = reject_remote_mode_for_app_server_subcommand(
            /*remote*/ None,
            Some("CODEX_REMOTE_AUTH_TOKEN"),
            Some(&subcommand),
        )
        .expect_err("non-interactive app-server subcommands should reject --remote-auth-token-env");
        assert!(err.to_string().contains("generate-internal-json-schema"));
    }

    #[test]
    fn read_remote_auth_token_from_env_var_reports_missing_values() {
        let err = read_remote_auth_token_from_env_var_with("CODEX_REMOTE_AUTH_TOKEN", |_| {
            Err(std::env::VarError::NotPresent)
        })
        .expect_err("missing env vars should be rejected");
        assert!(err.to_string().contains("is not set"));
    }

    #[test]
    fn read_remote_auth_token_from_env_var_trims_values() {
        let auth_token =
            read_remote_auth_token_from_env_var_with("CODEX_REMOTE_AUTH_TOKEN", |_| {
                Ok("  bearer-token  ".to_string())
            })
            .expect("env var should parse");
        assert_eq!(auth_token, "bearer-token");
    }

    #[test]
    fn read_remote_auth_token_from_env_var_rejects_empty_values() {
        let err = read_remote_auth_token_from_env_var_with("CODEX_REMOTE_AUTH_TOKEN", |_| {
            Ok(" \n\t ".to_string())
        })
        .expect_err("empty env vars should be rejected");
        assert!(err.to_string().contains("is empty"));
    }

    #[test]
    fn app_server_listen_websocket_url_parses() {
        let app_server = app_server_from_args(
            ["codex", "app-server", "--listen", "ws://127.0.0.1:4500"].as_ref(),
        );
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::WebSocket {
                bind_address: "127.0.0.1:4500".parse().expect("valid socket address"),
            }
        );
    }

    #[test]
    fn app_server_listen_stdio_url_parses() {
        let app_server =
            app_server_from_args(["codex", "app-server", "--listen", "stdio://"].as_ref());
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::Stdio
        );
    }

    #[test]
    fn app_server_listen_invalid_url_fails_to_parse() {
        let parse_result =
            MultitoolCli::try_parse_from(["codex", "app-server", "--listen", "http://foo"]);
        assert!(parse_result.is_err());
    }

    #[test]
    fn app_server_capability_token_flags_parse() {
        let app_server = app_server_from_args(
            [
                "codex",
                "app-server",
                "--ws-auth",
                "capability-token",
                "--ws-token-file",
                "/tmp/codex-token",
            ]
            .as_ref(),
        );
        assert_eq!(
            app_server.auth.ws_auth,
            Some(codex_app_server::WebsocketAuthCliMode::CapabilityToken)
        );
        assert_eq!(
            app_server.auth.ws_token_file,
            Some(PathBuf::from("/tmp/codex-token"))
        );
    }

    #[test]
    fn app_server_signed_bearer_flags_parse() {
        let app_server = app_server_from_args(
            [
                "codex",
                "app-server",
                "--ws-auth",
                "signed-bearer-token",
                "--ws-shared-secret-file",
                "/tmp/codex-secret",
                "--ws-issuer",
                "issuer",
                "--ws-audience",
                "audience",
                "--ws-max-clock-skew-seconds",
                "9",
            ]
            .as_ref(),
        );
        assert_eq!(
            app_server.auth.ws_auth,
            Some(codex_app_server::WebsocketAuthCliMode::SignedBearerToken)
        );
        assert_eq!(
            app_server.auth.ws_shared_secret_file,
            Some(PathBuf::from("/tmp/codex-secret"))
        );
        assert_eq!(app_server.auth.ws_issuer.as_deref(), Some("issuer"));
        assert_eq!(app_server.auth.ws_audience.as_deref(), Some("audience"));
        assert_eq!(app_server.auth.ws_max_clock_skew_seconds, Some(9));
    }

    #[test]
    fn app_server_rejects_removed_insecure_non_loopback_flag() {
        let parse_result = MultitoolCli::try_parse_from([
            "codex",
            "app-server",
            "--allow-unauthenticated-non-loopback-ws",
        ]);
        assert!(parse_result.is_err());
    }

    #[test]
    fn usage_clear_parses_flags() {
        let cli =
            MultitoolCli::try_parse_from(["codex", "usage", "clear", "--all-accounts", "--yes"])
                .expect("parse should succeed");
        let Some(Subcommand::Usage(UsageCommand { subcommand })) = cli.subcommand else {
            panic!("expected usage subcommand");
        };
        let UsageSubcommand::Clear(UsageClearCommand { all_accounts, yes }) = subcommand;
        assert!(all_accounts);
        assert!(yes);
    }

    #[test]
    fn tlogin_start_parses_flags() {
        let cli = MultitoolCli::try_parse_from([
            "codex",
            "--auth-file",
            "/tmp/auth.json",
            "tlogin",
            "start",
            "--user-id",
            "1234",
            "--json",
        ])
        .expect("parse should succeed");
        let Some(Subcommand::Tlogin(TloginCommand { action })) = cli.subcommand else {
            panic!("expected tlogin subcommand");
        };
        let TloginSubcommand::Start(TloginStartCommand { user_id, json }) = action else {
            panic!("expected tlogin start");
        };
        assert_eq!(user_id, "1234");
        assert!(json);
    }

    #[test]
    fn tlogin_complete_parses_flags() {
        let cli = MultitoolCli::try_parse_from([
            "codex",
            "--auth-file",
            "/tmp/auth.json",
            "tlogin",
            "complete",
            "--user-id",
            "1234",
        ])
        .expect("parse should succeed");
        let Some(Subcommand::Tlogin(TloginCommand { action })) = cli.subcommand else {
            panic!("expected tlogin subcommand");
        };
        let TloginSubcommand::Complete(TloginCompleteCommand { user_id }) = action else {
            panic!("expected tlogin complete");
        };
        assert_eq!(user_id, "1234");
    }

    #[test]
    fn features_enable_parses_feature_name() {
        let cli = MultitoolCli::try_parse_from(["codex", "features", "enable", "unified_exec"])
            .expect("parse should succeed");
        let Some(Subcommand::Features(FeaturesCli { sub })) = cli.subcommand else {
            panic!("expected features subcommand");
        };
        let FeaturesSubcommand::Enable(FeatureSetArgs { feature }) = sub else {
            panic!("expected features enable");
        };
        assert_eq!(feature, "unified_exec");
    }

    #[test]
    fn features_disable_parses_feature_name() {
        let cli = MultitoolCli::try_parse_from(["codex", "features", "disable", "shell_tool"])
            .expect("parse should succeed");
        let Some(Subcommand::Features(FeaturesCli { sub })) = cli.subcommand else {
            panic!("expected features subcommand");
        };
        let FeaturesSubcommand::Disable(FeatureSetArgs { feature }) = sub else {
            panic!("expected features disable");
        };
        assert_eq!(feature, "shell_tool");
    }

    #[test]
    fn feature_toggles_known_features_generate_overrides() {
        let toggles = FeatureToggles {
            enable: vec!["web_search_request".to_string()],
            disable: vec!["unified_exec".to_string()],
        };
        let overrides = toggles.to_overrides().expect("valid features");
        assert_eq!(
            overrides,
            vec![
                "features.web_search_request=true".to_string(),
                "features.unified_exec=false".to_string(),
            ]
        );
    }

    #[test]
    fn feature_toggles_accept_legacy_linux_sandbox_flag() {
        let toggles = FeatureToggles {
            enable: vec!["use_linux_sandbox_bwrap".to_string()],
            disable: Vec::new(),
        };
        let overrides = toggles.to_overrides().expect("valid features");
        assert_eq!(
            overrides,
            vec!["features.use_linux_sandbox_bwrap=true".to_string(),]
        );
    }

    #[test]
    fn feature_toggles_unknown_feature_errors() {
        let toggles = FeatureToggles {
            enable: vec!["does_not_exist".to_string()],
            disable: Vec::new(),
        };
        let err = toggles
            .to_overrides()
            .expect_err("feature should be rejected");
        assert_eq!(err.to_string(), "Unknown feature flag: does_not_exist");
    }
}
