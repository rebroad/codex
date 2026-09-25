/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn cli_version_for_display() -> &'static str {
    CODEX_CLI_VERSION
}

#[cfg(test)]
pub(crate) fn normalize_cli_version_for_snapshot(rendered: &str) -> String {
    use regex_lite::Regex;

    let rendered = rendered.replace(CODEX_CLI_VERSION, "<VERSION>");
    Regex::new(r"v[0-9][A-Za-z0-9.+_-]*…")
        .expect("truncated version regex is valid")
        .replace_all(&rendered, "v<VERSION>")
        .into_owned()
}
