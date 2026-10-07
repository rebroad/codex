#!/usr/bin/env bash
set -euo pipefail

# nextest can reuse target/debug/codex from an older checkout when the selected
# package does not itself build the CLI binary. Any package can acquire a test
# that resolves it through cargo_bin("codex"), so refresh it before every test
# run. Cargo keeps this cheap when the binary is already current.
cargo build --locked -p codex-cli --bin codex
