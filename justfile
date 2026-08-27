set working-directory := "codex-rs"
set positional-arguments
export CODEX_REPO_ROOT := justfile_directory()
export JUST_SHELL := justfile_directory() / "scripts/just-shell.py"
set shell := ["python3", "-c", 'import os, runpy; runpy.run_path(os.environ["JUST_SHELL"], run_name="__main__")']
set windows-shell := ["python", "-c", 'import os, runpy; runpy.run_path(os.environ["JUST_SHELL"], run_name="__main__")']

rust_min_stack := "8388608" # 8 MiB
python := if os_family() == "windows" { "python" } else { "python3" }
build_tree := (justfile_directory() + ".build") / "codex-rs"
cargo_source_directory := justfile_directory() / "codex-rs"
cargo_working_directory := if path_exists(build_tree / "Cargo.toml") == "true" { build_tree } else { justfile_directory() / "codex-rs" }
cargo_target_dir := env_var_or_default("CARGO_TARGET_DIR", cargo_working_directory / "target")
rusty_v8_setup := "repo_root=\"$(cd \"$(pwd -P)/..\" && pwd -P)\"; rusty_v8_target=\"$(rustc -vV | sed -n 's/^host: //p')\"; rusty_v8_version=\"$(python3 \"${repo_root}/scripts/rusty_v8_version.py\" \"${repo_root}/codex-rs/Cargo.lock\")\"; rusty_v8_dir=\"${repo_root}/build/rusty-v8-artifacts/${rusty_v8_version}/${rusty_v8_target}\"; eval \"$(bash \"${repo_root}/scripts/resolve_rusty_v8_artifacts.sh\" --target=\"${rusty_v8_target}\" --output-dir=\"${rusty_v8_dir}\" --cargo-build-dir=\"${repo_root}/codex-rs/target\" 2>/dev/null)\"; export RUSTY_V8_ARCHIVE RUSTY_V8_SRC_BINDING_PATH;"
android_cargo_setup := "rustc_host=\"$(rustc -vV | sed -n 's/^host: //p')\"; if test \"${rustc_host}\" = aarch64-linux-android && android_clang=\"$(command -v aarch64-linux-android-clang || true)\" && test -x \"${android_clang}\"; then android_builtins=\"$(${android_clang} -print-file-name=libclang_rt.builtins-aarch64-android.a)\"; test -s \"${android_builtins}\" || { echo \"Android compiler builtins archive not found\" >&2; exit 1; }; export CARGO_BUILD_JOBS=\"${CARGO_BUILD_JOBS:-1}\" CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=\"${android_clang}\" CC_aarch64_linux_android=\"${android_clang}\" CXX_aarch64_linux_android=\"${android_clang}++\" AR_aarch64_linux_android=llvm-ar RANLIB_aarch64_linux_android=llvm-ranlib CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS=\"-Clink-arg=-lc++_shared -Clink-arg=-Wl,-rpath,\\$ORIGIN -Clink-arg=${android_builtins}\"; fi;"
openssl_setup := "repo_root=\"$(cd \"$(pwd -P)/..\" && pwd -P)\"; openssl_script=\"${repo_root}/scripts/openssl_artifacts.sh\"; openssl_target=\"$(rustc -vV | sed -n 's/^host: //p')\"; openssl_version=\"$(bash \"${openssl_script}\" version \"${repo_root}/codex-rs/Cargo.lock\")\"; openssl_env=\"$(bash \"${openssl_script}\" env \"${repo_root}\" \"${openssl_target}\" \"${openssl_version}\" 2>/dev/null || true)\"; if test -n \"${openssl_env}\"; then eval \"${openssl_env}\"; export OPENSSL_DIR OPENSSL_NO_VENDOR OPENSSL_STATIC; echo \"Using cached OpenSSL ${openssl_version} for ${openssl_target}.\" >&2; fi;"
openssl_cache := "repo_root=\"$(cd \"$(pwd -P)/..\" && pwd -P)\"; openssl_script=\"${repo_root}/scripts/openssl_artifacts.sh\"; openssl_target=\"$(rustc -vV | sed -n 's/^host: //p')\"; openssl_version=\"$(bash \"${openssl_script}\" version \"${repo_root}/codex-rs/Cargo.lock\")\"; bash \"${openssl_script}\" cache \"${repo_root}/codex-rs/target\" \"${repo_root}\" \"${openssl_target}\" \"${openssl_version}\" \"${openssl_target}\" || true;"
codex_build_setup := "CODEX_BUILD_TIMESTAMP=\"0000000000-000000000000\"; export CODEX_BUILD_TIMESTAMP;"

# Display help
help:
    just -l

# `codex`
alias c := codex
codex *args:
    cd "{{ cargo_working_directory }}" && cargo run --bin codex -- {args}

# `codex exec`
exec *args:
    cd "{{ cargo_working_directory }}" && cargo run --bin codex -- exec {args}

# Start `codex exec-server` and run codex-tui.
[no-cd]
[positional-arguments]
[unix]
tui-with-exec-server *args:
    {{ justfile_directory() }}/scripts/run_tui_with_exec_server.sh "$@"

# Run the CLI version of the file-search crate.
file-search *args:
    cd "{{ cargo_working_directory }}" && cargo run --bin codex-file-search -- {args}

# Run the standalone code-mode host from source.
code-mode-host *args:
    cd "{{ cargo_working_directory }}" && cargo run --bin codex-code-mode-host -- {args}

# Assemble a local Codex package.
[no-cd]
assemble-codex-package *args:
    {{ python }} {{ justfile_directory() }}/scripts/build_codex_package.py {args}

# Build the CLI and run the app-server test client
app-server-test-client *args:
    cd "{{ cargo_working_directory }}" && cargo build -p codex-cli
    cd "{{ cargo_working_directory }}" && cargo run -p codex-app-server-test-client -- --codex-bin "{{ cargo_target_dir }}/debug/codex" {args}

# Format the justfile, Rust, Bazel/Starlark, Python SDK code, and Python scripts.
fmt:
    @{{ python }} ../scripts/format.py

# Check formatting without modifying files.
fmt-check:
    @{{ python }} ../scripts/format.py --check

fix *args:
    cd "{{ cargo_working_directory }}" && cargo clippy --fix --tests --allow-dirty {args}

clippy *args:
    cd "{{ cargo_working_directory }}" && cargo clippy --tests {args}

[unix]
install:
    rustup show active-toolchain
    cd "{{ cargo_working_directory }}" && cargo fetch

[windows]
install:
    #!powershell.exe -File
    $pwsh = Get-Command pwsh.exe -ErrorAction SilentlyContinue
    if (-not $pwsh) {
        winget install --exact --id Microsoft.PowerShell --source winget --accept-package-agreements --accept-source-agreements
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    rustup show active-toolchain
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Set-Location "{{ cargo_working_directory }}"; cargo fetch
    exit $LASTEXITCODE

# Run nextest with --no-fail-fast so all tests are run.
#
# Run `cargo install --locked cargo-nextest` if you don't have it installed.
# Prefer this for routine local runs. Workspace crate features are banned, so
# there should be no need to add `--all-features`.
[unix]
test *args:
    @cd "{{ cargo_working_directory }}" && {{ android_cargo_setup }} {{ rusty_v8_setup }} {{ openssl_setup }} {{ codex_build_setup }} CARGO_TARGET_DIR={{ cargo_target_dir }} cargo build --locked -p codex-cli -p codex-code-mode-host
    @cd "{{ cargo_working_directory }}" && {{ android_cargo_setup }} {{ rusty_v8_setup }} {{ openssl_setup }} {{ codex_build_setup }} CARGO_TARGET_DIR={{ cargo_target_dir }} cargo build --locked -p codex-rmcp-client --bin test_stdio_server
    @cd "{{ cargo_working_directory }}" && {{ android_cargo_setup }} {{ rusty_v8_setup }} {{ openssl_setup }} {{ codex_build_setup }} CARGO_TARGET_DIR={{ cargo_target_dir }}; RUST_MIN_STACK={{ rust_min_stack }}; export CARGO_TARGET_DIR RUST_MIN_STACK RUSTY_V8_ARCHIVE RUSTY_V8_SRC_BINDING_PATH; if command -v cargo-nextest >/dev/null 2>&1 || test "$(rustc -vV | sed -n 's/^host: //p')" != aarch64-linux-android; then NEXTEST_PROFILE=local cargo nextest run --locked --no-fail-fast "$@"; else echo "cargo-nextest is unavailable on Android; using cargo test" >&2; cargo test --locked "$@"; fi
    @cd "{{ cargo_working_directory }}" && {{ openssl_cache }}

[windows]
test *args:
    @Set-Location "{{ cargo_working_directory }}"; $env:CARGO_TARGET_DIR = "{{ cargo_target_dir }}"; cargo build -p codex-cli -p codex-code-mode-host
    @Set-Location "{{ cargo_working_directory }}"; $env:CARGO_TARGET_DIR = "{{ cargo_target_dir }}"; cargo build -p codex-rmcp-client --bin test_stdio_server
    @Set-Location "{{ cargo_working_directory }}"; $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; $env:NEXTEST_PROFILE = "local"; cargo nextest run --no-fail-fast @($args | Select-Object -Skip 1)

# Run from the repository root so scripts that resolve paths from `cwd` see
# the same layout they use in GitHub Actions.
[no-cd]
test-github-scripts:
    {{ python }} -m unittest discover -s {{ justfile_directory() }}/.github/scripts -p 'test_*.py'

# Run explicit workspace benchmark targets.
bench *args:
    cd "{{ cargo_working_directory }}" && cargo bench --workspace --bench '*' {args}

# Run benchmark targets once to ensure they start successfully.
bench-smoke:
    just bench -- --test

# Run Bazel-backed end-to-end macrobenchmarks with optimized binaries.
bench-e2e:
    # Keep measured binaries comparable to production-style optimized builds.
    bazel test --compilation_mode=opt --cache_test_results=no --test_output=streamed //codex-rs:e2e-benchmarks

# Run Bazel-backed end-to-end macrobenchmarks once per case with release-like
# Rust cfg paths but fastbuild codegen.
bench-e2e-smoke:
    # Avoid optimizer cost because smoke runs only check that benchmarks work.
    # Compile target Rust code through the same release-only cfg paths as opt.
    # Compile exec-platform Rust tools through those release-only cfg paths too.
    bazel test --compilation_mode=fastbuild --@rules_rust//rust/settings:extra_rustc_flag=-Cdebug-assertions=no --@rules_rust//rust/settings:extra_exec_rustc_flag=-Cdebug-assertions=no --cache_test_results=no --test_output=streamed --test_arg=--test //codex-rs:e2e-benchmarks

# Build and run Codex from source using Bazel.
# On Unix, use `[no-cd]` and `--run_under="cd $PWD &&"` to ensure Bazel runs
# the command in the current working directory.
[no-cd]
[unix]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under="cd $PWD &&" -- "$@"

[windows]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under='cd /d "{{ invocation_directory_native() }}" &&' -- @($args | Select-Object -Skip 1)

# Build and run the standalone code-mode host from source using Bazel.
[no-cd]
[unix]
bazel-code-mode-host *args:
    bazel run //codex-rs/code-mode-host:codex-code-mode-host --run_under="cd $PWD &&" -- "$@"

[windows]
bazel-code-mode-host *args:
    bazel run //codex-rs/code-mode-host:codex-code-mode-host --run_under='cd /d "{{ invocation_directory_native() }}" &&' -- @($args | Select-Object -Skip 1)

[no-cd]
bazel-lock-update:
    bazel mod deps --lockfile_mode=update

[no-cd]
[unix]
bazel-lock-check:
    {{ justfile_directory() }}/scripts/check-module-bazel-lock.sh

[windows]
bazel-lock-check:
    bazel mod deps --lockfile_mode=error; if ($LASTEXITCODE -ne 0) { Write-Error "MODULE.bazel.lock is out of date. Run 'just bazel-lock-update' and commit the updated lockfile."; exit 1 }

bazel-test:
    bazel test --test_tag_filters=-argument-comment-lint //... --keep_going

[no-cd]
[unix]
bazel-clippy:
    bazel_targets="$({{ justfile_directory() }}/scripts/list-bazel-clippy-targets.sh)" && bazel build --config=clippy -- ${bazel_targets}

[no-cd]
[unix]
bazel-argument-comment-lint:
    bazel build --config=argument-comment-lint -- $({{ justfile_directory() }}/tools/argument-comment-lint/list-bazel-targets.sh)

build-for-release:
    bazel build //codex-rs/cli:release_binaries

# Run the MCP server
mcp-server-run *args:
    cd "{{ cargo_working_directory }}" && cargo run -p codex-mcp-server -- {args}

# Regenerate the json schema for config.toml from the current config types.
write-config-schema:
    cd "{{ cargo_working_directory }}" && cargo run -p codex-core --bin codex-write-config-schema -- --out "{{ cargo_source_directory }}/core/config.schema.json"

# Regenerate vendored app-server protocol schema artifacts.
write-app-server-schema *args:
    cd "{{ cargo_working_directory }}" && {{ python }} app-server-protocol/scripts/write_schema_fixtures.py --schema-root "{{ cargo_source_directory }}/app-server-protocol/schema" {args}

[no-cd]
write-hooks-schema:
    cd "{{ cargo_working_directory }}" && cargo run --manifest-path "{{ cargo_working_directory }}/Cargo.toml" -p codex-hooks --bin write_hooks_schema_fixtures -- "{{ cargo_source_directory }}/hooks/schema"

# Run the argument-comment Dylint checks across codex-rs.
[no-cd]
[unix]
argument-comment-lint *args:
    if [ "$#" -eq 0 ]; then \
      bazel build --config=argument-comment-lint -- $({{ justfile_directory() }}/tools/argument-comment-lint/list-bazel-targets.sh); \
    else \
      {{ justfile_directory() }}/tools/argument-comment-lint/run-prebuilt-linter.py "$@"; \
    fi

[no-cd]
argument-comment-lint-from-source *args:
    {{ python }} {{ justfile_directory() }}/tools/argument-comment-lint/run.py {args}

# Tail logs from the state SQLite database
[unix]
log *args:
    cd "{{ cargo_working_directory }}" && if [ "${1:-}" = "--" ]; then shift; fi; cargo run -p codex-cli --bin logs_client -- "$@"

[windows]
log *args:
    Set-Location "{{ cargo_working_directory }}"; $forwarded_args = @($args | Select-Object -Skip 1); if ($forwarded_args.Count -gt 0 -and $forwarded_args[0] -eq "--") { $forwarded_args = @($forwarded_args | Select-Object -Skip 1) }; cargo run -p codex-cli --bin logs_client -- @forwarded_args
