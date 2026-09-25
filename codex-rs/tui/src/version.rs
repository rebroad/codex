/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn cli_version_for_display() -> &'static str {
    #[cfg(test)]
    {
        "0.0.0"
    }
    #[cfg(not(test))]
    {
        CODEX_CLI_VERSION
    }
}
