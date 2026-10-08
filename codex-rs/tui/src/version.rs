/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The current Codex CLI build stamp shown in the TUI title.
pub const CODEX_CLI_DISPLAY_VERSION: &str = env!("CODEX_TUI_BUILD_VERSION");

pub(crate) fn cli_version_for_display() -> &'static str {
    CODEX_CLI_DISPLAY_VERSION
}

#[cfg(test)]
pub(crate) fn normalize_cli_version_for_snapshot(rendered: &str) -> String {
    use regex_lite::Regex;

    let rendered =
        Regex::new(r"(OpenAI Codex \(v)[0-9][A-Za-z0-9.+_…-]*(?:\n[A-Za-z0-9.+_…-]+)*(\)?)")
            .expect("truncated title version regex is valid")
            .replace_all(&rendered, |captures: &regex_lite::Captures<'_>| {
                format!("{}<VERSION>{}", &captures[1], &captures[2])
            })
            .into_owned();
    rendered
        .replace(CODEX_CLI_VERSION, "<VERSION>")
        .replace(CODEX_CLI_DISPLAY_VERSION, "<VERSION>")
}
