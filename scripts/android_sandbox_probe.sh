#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "aarch64" ||
  ! -x /system/bin/getprop ]]; then
  echo "This probe currently supports Android/Termux on AArch64 only." >&2
  exit 2
fi

if [[ -z "${PREFIX:-}" || ! -d "${PREFIX}/tmp" ]]; then
  echo "PREFIX/tmp must exist; run this from Termux." >&2
  exit 2
fi

compiler="${CC:-clang}"
if ! command -v "${compiler}" >/dev/null 2>&1; then
  echo "${compiler} is required; install it with: pkg install clang" >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
probe_dir="$(mktemp -d "${PREFIX}/tmp/codex-android-sandbox-probe.XXXXXX")"
trap 'rm -rf -- "${probe_dir}"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"${compiler}" -std=c11 -O2 -Wall -Wextra -Werror \
  "${script_dir}/android_sandbox_probe.c" -o "${probe_dir}/probe"
"${probe_dir}/probe"
