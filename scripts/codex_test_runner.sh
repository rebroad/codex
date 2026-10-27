#!/usr/bin/env bash
set -euo pipefail

rustc_bin="${RUSTC:-rustc}"
host_target="$("${rustc_bin}" -vV | sed -n 's/^host: //p')"
if [[ "${host_target}" == *-linux-android ]]; then
  # cargo-nextest waits for test children through Tokio's SIGCHLD waiter. On
  # Android that waiter can miss exited children, leaving them as zombies.
  # Keep tests serial on constrained Android devices, as codex-exec-server's
  # Bazel test targets already require with --test-threads=1.
  export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"
  exec cargo test --locked --no-fail-fast "$@"
fi

export NEXTEST_PROFILE=local
exec cargo nextest run --locked --no-fail-fast "$@"
