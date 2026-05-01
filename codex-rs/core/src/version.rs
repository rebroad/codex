/// Shared build version placeholder embedded into binaries at compile time.
///
/// `scripts/rebuild_codex.sh` and `scripts/build_armv7.sh` patch this fixed-width
/// suffix in-place after linking to stamp the final timestamp and commit hash
/// without requiring a dedicated build-script crate rebuild.
pub const CODEX_BUILD_VERSION: &str =
    concat!(env!("CARGO_PKG_VERSION"), "-000000000000-000000000000");
