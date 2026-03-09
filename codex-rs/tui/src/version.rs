/// The current Codex CLI version as embedded at compile time.
///
/// This uses the same build-time version string (including timestamp) that
/// `codex --version` reports, so `codex status` shows a matching version.
pub const CODEX_CLI_VERSION: &str = codex_build_info::CODEX_BUILD_VERSION;
