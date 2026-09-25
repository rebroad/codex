use super::*;
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn path(path: &str) -> io::Result<AdditionalWritableRoot> {
    AbsolutePathBuf::try_from(PathBuf::from(path)).map(AdditionalWritableRoot::Path)
}

#[test]
fn derives_roots_with_optional_dot_suffix_and_does_not_chain() -> io::Result<()> {
    let source = AbsolutePathBuf::try_from(PathBuf::from("/work/src/project.release.git"))?;
    let source_without_suffix = AbsolutePathBuf::try_from(PathBuf::from("/work/src/tidewater"))?;
    let unmatched = AbsolutePathBuf::try_from(PathBuf::from("/work/other"))?;
    let entries = [
        AdditionalWritableRoot::Rewrite(
            r"s/^\/work\/src\/([^\/\.]+)(\.[^\/]*)?$/\/work\/builds\/$1.build".to_owned(),
        ),
        AdditionalWritableRoot::Rewrite(
            r"s/\/builds\/([^\/]+)\.build$/\/must-not-chain\/$1".to_owned(),
        ),
    ];

    let actual =
        derive_additional_writable_roots(&entries, &[source, source_without_suffix, unmatched])?;

    assert_eq!(
        actual,
        vec![
            AbsolutePathBuf::try_from(PathBuf::from("/work/builds/project.build"))?,
            AbsolutePathBuf::try_from(PathBuf::from("/work/builds/tidewater.build"))?,
        ]
    );
    Ok(())
}

#[test]
fn keeps_absolute_paths_and_deserializes_vi_rewrites() -> Result<(), Box<dyn std::error::Error>> {
    let config: crate::config_toml::ConfigToml = toml::from_str(
        r#"
additional_writable_roots = [
    "/var/tmp",
    's/^\/work\/([^\/\.]+)(\.[^\/]*)?$/\/builds\/$1.build',
]
"#,
    )?;

    assert_eq!(
        config.additional_writable_roots,
        vec![
            path("/var/tmp")?,
            AdditionalWritableRoot::Rewrite(
                r"s/^\/work\/([^\/\.]+)(\.[^\/]*)?$/\/builds\/$1.build".to_owned()
            ),
        ]
    );
    Ok(())
}

#[test]
fn deduplicates_derived_roots() -> io::Result<()> {
    let root = AbsolutePathBuf::try_from(PathBuf::from("/work/project"))?;
    let entries = [AdditionalWritableRoot::Rewrite(
        r"s/^\/work\/(.*)$/\/build\/$1".to_owned(),
    )];

    let actual = derive_additional_writable_roots(&entries, &[root.clone(), root])?;

    assert_eq!(
        actual,
        vec![AbsolutePathBuf::try_from(PathBuf::from("/build/project"))?]
    );
    Ok(())
}

#[test]
fn invalid_regex_is_reported_without_matching_roots() {
    let entries = [AdditionalWritableRoot::Rewrite(r"s/[/build".to_owned())];

    let error = derive_additional_writable_roots(&entries, &[])
        .expect_err("invalid entry regex should be rejected even without roots");

    assert!(error.to_string().contains("entry 0 regex"));
}

#[test]
fn rejects_relative_replacement_results() -> io::Result<()> {
    let root = AbsolutePathBuf::try_from(PathBuf::from("/work/project"))?;
    let entries = [AdditionalWritableRoot::Rewrite(
        r"s/^\/work\/(.*)$/relative\/$1".to_owned(),
    )];

    let error = derive_additional_writable_roots(&entries, &[root])
        .expect_err("replacement output must remain absolute");

    assert!(error.to_string().contains("produced an invalid path"));
    Ok(())
}
