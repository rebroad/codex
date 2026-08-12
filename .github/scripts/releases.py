"""Validate and compare the version formats used by Codex releases."""

import re
import sys

_VERSION_RE = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha(?:\.[0-9]+){0,2}"
    r"|beta(?:\.[0-9]+)?)(?:\.[0-9a-f]{10}\.[0-9]{12})?)?$"
)
_CANDIDATE_SUFFIX_RE = re.compile(r"\.([0-9a-f]{10})\.([0-9]{12})$")


def is_valid_release_version(version: str) -> bool:
    return _VERSION_RE.fullmatch(version) is not None


def _version_key(version: str) -> tuple[tuple[int, ...], int, tuple[int, ...], int, str]:
    candidate = _CANDIDATE_SUFFIX_RE.search(version)
    candidate_timestamp = int(candidate.group(2)) if candidate else 0
    candidate_commit = candidate.group(1) if candidate else ""
    if candidate:
        version = version[: candidate.start()]
    base, _, prerelease = version.partition("-")
    channel, _, suffix = prerelease.partition(".")
    return (
        tuple(int(part) for part in base.split(".")),
        {"alpha": 0, "beta": 1, "": 2}[channel],
        tuple(int(part) for part in suffix.split(".")) if suffix else (),
        candidate_timestamp,
        candidate_commit,
    )


def should_update_version(version: str, current_version: str) -> bool:
    """Compare version strings; replace an unreadable current version."""
    if not is_valid_release_version(version):
        raise ValueError(f"invalid release version: {version}")
    if not is_valid_release_version(current_version):
        # Replace an unreadable pointer so publishing can repair it.
        return True

    return _version_key(version) > _version_key(current_version)


if __name__ == "__main__":
    raise SystemExit(0 if is_valid_release_version(sys.argv[1]) else 1)
