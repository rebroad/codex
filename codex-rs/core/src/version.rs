/// Shared build version placeholder embedded into binaries at compile time.
///
/// The build script reads `codex-rs/VERSION`, appends a fixed-width suffix, and
/// exposes the result through `CODEX_BUILD_VERSION`. The release scripts patch
/// that suffix in-place after linking to stamp the final commit hash and
/// timestamp without forcing the manifest version to change.
pub const CODEX_BUILD_VERSION: &str = env!("CODEX_BUILD_VERSION");
