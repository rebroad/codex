#!/usr/bin/env bash
set -euo pipefail

# nextest can reuse target/debug/codex from an older checkout when the selected
# package does not itself build the CLI binary. Refresh it only for packages
# whose tests resolve codex through cargo_bin("codex").
selected_packages=()
build_all_packages=false
package_spec=""
while (($#)); do
  case "$1" in
    --workspace|--all)
      build_all_packages=true
      ;;
    -p|--package)
      (($# >= 2)) || break
      selected_packages+=("${2%%@*}")
      shift
      ;;
    -p*)
      package_spec="${1#-p}"
      selected_packages+=("${package_spec%%@*}")
      ;;
    --package=*)
      package_spec="${1#--package=}"
      selected_packages+=("${package_spec%%@*}")
      ;;
  esac
  shift
done

codex_cli_test_packages=(
  codex-app-server
  codex-cli
  codex-core
  codex-mcp
  codex-rmcp-client
  codex-tui
)

build_cli=false
if [[ "$build_all_packages" == true || ${#selected_packages[@]} -eq 0 ]]; then
  build_cli=true
else
  for selected_package in "${selected_packages[@]}"; do
    for codex_cli_test_package in "${codex_cli_test_packages[@]}"; do
      if [[ "$selected_package" == "$codex_cli_test_package" ]]; then
        build_cli=true
        break 2
      fi
    done
  done
fi

if [[ "$build_cli" == false ]]; then
  echo "Skipping Codex CLI refresh; selected packages do not spawn the CLI."
  exit 0
fi

cargo build --locked -p codex-cli --bin codex
