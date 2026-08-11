#!/usr/bin/env bash
set -euo pipefail

manifest="${1:?usage: apply_target_cargo_patches.sh MANIFEST TARGET}"
target="${2:?usage: apply_target_cargo_patches.sh MANIFEST TARGET}"

sed -i '/^# ARMv7 support is not in the crates.io release yet\.$/,+1d' "${manifest}"
sed -i "/^# Keep pagable's shared workspace dependencies on crates.io so they use the$/,/^strong_hash = { version = \"0.1.0\" }$/d" "${manifest}"

case "${target}" in
  armv7-unknown-linux-gnueabihf)
    sed -i '/^\[patch\.crates-io\]$/a # ARMv7 support is not in the crates.io release yet.\npagable = { git = "https://github.com/facebook/starlark-rust", rev = "4190cefd570e05858cbb51815a4de11a7b49f951" }' "${manifest}"
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
