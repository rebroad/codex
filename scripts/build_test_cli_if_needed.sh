#!/usr/bin/env bash
set -euo pipefail

# nextest can reuse target/debug/codex from an older checkout when the selected
# package does not itself build the CLI binary. Most package selections may
# contain tests that resolve it through cargo_bin("codex"), so refresh it
# before those test runs. The daemon crate uses its own executable fixtures.
selected_packages=()
build_all_packages=false
while (($#)); do
  case "$1" in
    --workspace|--all)
      build_all_packages=true
      ;;
    -p|--package)
      (($# >= 2)) || break
      selected_packages+=("$2")
      shift
      ;;
    -p*)
      selected_packages+=("${1#-p}")
      ;;
    --package=*)
      selected_packages+=("${1#--package=}")
      ;;
  esac
  shift
done

if [[ "$build_all_packages" == false \
  && "${selected_packages[*]}" == codex-app-server-daemon ]]; then
  echo "Skipping Codex CLI refresh for daemon-only tests."
  exit 0
fi

cargo build --locked -p codex-cli --bin codex
