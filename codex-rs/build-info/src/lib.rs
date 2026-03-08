/// Canonical Codex build version embedded at compile time.
///
/// This value is shared across all crates that need to display the binary
/// version so all UI surfaces stay consistent.
pub const CODEX_BUILD_VERSION: &str = env!("CODEX_BUILD_VERSION");
