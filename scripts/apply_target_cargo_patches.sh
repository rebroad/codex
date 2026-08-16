#!/usr/bin/env bash
set -euo pipefail

manifest="${1:?usage: apply_target_cargo_patches.sh MANIFEST TARGET}"
target="${2:?usage: apply_target_cargo_patches.sh MANIFEST TARGET}"

sed -i '/^# ARMv7 support is not in the crates.io release yet\.$/,+1d' "${manifest}"
sed -i "/^# Keep pagable's shared workspace dependencies on crates.io so they use the$/,/^strong_hash = { version = \"0.1.0\" }$/d" "${manifest}"
sed -i '/^# ARMv7 Rusty V8 support comes from the matching rusty_v8 release\.$/,+1d' "${manifest}"
sed -i '/^v8 = { git = .*tag = \"v[^\"]*\" }$/d' "${manifest}"

case "${target}" in
  armv7-unknown-linux-gnueabihf|armv7-unknown-linux-musleabihf)
    v8_version="$(sed -n 's/^v8 = "=\([^\"]*\)"$/\1/p' "${manifest}")"
    [[ -n "${v8_version}" ]] || {
      echo "Unable to determine the pinned v8 version from ${manifest}." >&2
      exit 1
    }
    rusty_v8_repo="${RUSTY_V8_REPO_URL:-https://github.com/rebroad/rusty_v8.git}"
    rusty_v8_source_tag="${RUSTY_V8_SOURCE_TAG:-v${v8_version}-armv7-release}"
    escaped_rusty_v8_repo="${rusty_v8_repo//\//\\/}"
    sed -i "/^\[patch\.crates-io\]$/a # ARMv7 Rusty V8 support comes from the matching compatibility release.\nv8 = { git = \"${escaped_rusty_v8_repo}\", tag = \"${rusty_v8_source_tag}\" }\n# ARMv7 support is not in the crates.io release yet.\npagable = { git = \"https://github.com/facebook/starlark-rust\", rev = \"4190cefd570e05858cbb51815a4de11a7b49f951\" }" "${manifest}"
    cat >>"${manifest}" <<'EOF'

# Keep pagable's shared workspace dependencies on crates.io so they use the
# same Dupe/Allocative traits as the released Starlark crates.
[patch."https://github.com/facebook/starlark-rust"]
allocative = { version = "0.3.6" }
dupe = { version = "0.9.1" }
strong_hash = { version = "0.1.0" }
EOF
    ;;
esac
