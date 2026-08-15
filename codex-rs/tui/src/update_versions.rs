pub(crate) fn is_newer(latest: &str, current: &str) -> Option<bool> {
    match (parse_version(latest), parse_version(current)) {
        (Some((l, l_pre)), Some((c, c_pre))) => Some(match l.cmp(&c) {
            std::cmp::Ordering::Equal => match (l_pre, c_pre) {
                (None, None) => false,
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (Some(l_pre), Some(c_pre)) => l_pre > c_pre,
            },
            ordering => ordering.is_gt(),
        }),
        _ => None,
    }
}

pub(crate) fn extract_version_from_latest_tag(latest_tag_name: &str) -> anyhow::Result<String> {
    latest_tag_name
        .strip_prefix("rust-v")
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse latest tag name '{latest_tag_name}'"))
}

pub(crate) fn is_source_build_version(version: &str) -> bool {
    parse_version(version)
        .is_some_and(|((major, minor, patch), _)| (major, minor, patch) == (0, 0, 0))
}

fn parse_version(v: &str) -> Option<((u64, u64, u64), Option<(u8, u64, u64)>)> {
    let mut iter = v.trim().splitn(3, '.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let patch = iter.next()?;
    let (patch, pre) = match patch.split_once('-') {
        Some((patch, pre)) => (patch, Some(pre)),
        None => (patch, None),
    };
    let patch = patch.parse::<u64>().ok()?;
    let pre = pre.map(|pre| {
        let mut pre_iter = pre.split('.');
        let rank = match pre_iter.next()? {
            "alpha" => 0,
            "beta" => 1,
            _ => return None,
        };
        let number = pre_iter.next()?.parse::<u64>().ok()?;
        let hotfix = pre_iter
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        Some((rank, number, hotfix))
    });
    Some(((maj, min, patch), pre.flatten()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_version_from_latest_tag() {
        assert_eq!(
            extract_version_from_latest_tag("rust-v1.5.0").expect("failed to parse version"),
            "1.5.0"
        );
    }

    #[test]
    fn latest_tag_without_prefix_is_invalid() {
        assert!(extract_version_from_latest_tag("v1.5.0").is_err());
    }

    #[test]
    fn prerelease_version_is_not_considered_newer() {
        assert_eq!(is_newer("0.11.0-beta.1", "0.11.0"), Some(false));
        assert_eq!(is_newer("1.0.0-rc.1", "1.0.0"), None);
    }

    #[test]
    fn alpha_versions_are_compared_within_their_channel() {
        assert_eq!(is_newer("0.11.0-alpha.2", "0.11.0-alpha.1"), Some(true));
        assert_eq!(is_newer("0.11.0-alpha.1", "0.11.0-alpha.2"), Some(false));
        assert_eq!(is_newer("0.11.0", "0.11.0-alpha.2"), Some(true));
    }

    #[test]
    fn plain_semver_comparisons_work() {
        assert_eq!(is_newer("0.11.1", "0.11.0"), Some(true));
        assert_eq!(is_newer("0.11.0", "0.11.1"), Some(false));
        assert_eq!(is_newer("1.0.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("0.9.9", "1.0.0"), Some(false));
    }

    #[test]
    fn source_build_version_is_not_checked() {
        assert!(is_source_build_version("0.0.0"));
        assert!(!is_source_build_version("0.1.0"));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_version(" 1.2.3 \n"), Some(((1, 2, 3), None)));
        assert_eq!(is_newer(" 1.2.3 ", "1.2.2"), Some(true));
    }
}
