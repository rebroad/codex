#!/usr/bin/env bash
set -euo pipefail

# nextest can reuse target/debug/codex from an older checkout when the selected
# package does not itself build the CLI binary. Refresh it only for packages
# whose tests resolve codex through cargo_bin("codex").
selected_packages=()
build_all_packages=false
package_spec=""
requested_args=("$@")
build_linux_sandbox=false
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

if [[ "$build_all_packages" == false && ${#selected_packages[@]} -eq 1 \
  && "${selected_packages[0]}" == codex-app-server ]]; then
  for requested_arg in "${requested_args[@]}"; do
    if [[ "$requested_arg" == managed_handoff_reattaches_a_running_local_command ]]; then
      # This test launches app-server and exec-server fixtures, not the Codex
      # CLI. It needs the sandbox binary to execute the test command.
      build_cli=false
      build_linux_sandbox=true
      break
    fi
  done
fi

if [[ "$build_cli" == true ]]; then
  cargo build --locked -p codex-cli --bin codex
else
  echo "Skipping Codex CLI refresh; selected tests do not spawn the CLI."
fi

if [[ "$build_linux_sandbox" == true ]]; then
  cargo build --locked -p codex-linux-sandbox --bin codex-linux-sandbox
fi
