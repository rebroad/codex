use pretty_assertions::assert_eq;

use super::ExecutableIdentity;
use super::parse_codex_version;
use std::path::Path;

#[test]
fn parses_codex_cli_version_output() {
    assert_eq!(
        parse_codex_version("codex 1.2.3\n").expect("version"),
        "1.2.3"
    );
}

#[test]
fn rejects_malformed_codex_cli_version_output() {
    assert!(parse_codex_version("codex\n").is_err());
}

#[test]
fn executable_identity_uses_version_and_path() {
    let identity = ExecutableIdentity::new("1.2.3".to_string(), Path::new("/first/codex"));
    let same_version = ExecutableIdentity::new("1.2.3".to_string(), Path::new("/second/codex"));
    let different_version = ExecutableIdentity::new("1.2.4".to_string(), Path::new("/first/codex"));

    assert_ne!(identity, same_version);
    assert!(identity.same_version(&same_version));
    assert!(!identity.same_version(&different_version));
}
