#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: codex_cargo_env.sh --source-repo DIR --build-repo DIR
  --mode debug|release --target-mode native|musl|armv7|android
  [--purpose NAME]
  [--emit|--print-target]

Each --emit invocation records the Cargo artifact operation in
BUILD_REPO/build/cargo-artifact-operations.jsonl.  The record's target_dir,
timestamp, and fingerprint can be used to identify artifacts produced by
that operation without relying on target-purpose symlinks.
EOF
}

SOURCE_REPO=''
BUILD_REPO=''
MODE=debug
TARGET_MODE=native
PURPOSE="${CODEX_CARGO_PURPOSE:-default}"
OUTPUT=emit

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-repo) SOURCE_REPO="${2:?missing source repository}"; shift 2 ;;
    --build-repo) BUILD_REPO="${2:?missing build repository}"; shift 2 ;;
    --mode) MODE="${2:?missing build mode}"; shift 2 ;;
    --target-mode) TARGET_MODE="${2:?missing target mode}"; shift 2 ;;
    --purpose) PURPOSE="${2:?missing target purpose}"; shift 2 ;;
    --emit) OUTPUT=emit; shift ;;
    --print-target) OUTPUT=target; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -d "${SOURCE_REPO}/codex-rs" ]] || { echo "source repository not found: ${SOURCE_REPO}" >&2; exit 1; }
[[ -d "${BUILD_REPO}/codex-rs" ]] || { echo "build repository not found: ${BUILD_REPO}" >&2; exit 1; }
[[ "${MODE}" == debug || "${MODE}" == release ]] || { echo "invalid mode: ${MODE}" >&2; exit 2; }
[[ "${PURPOSE}" =~ ^[[:alnum:]_.-]+$ ]] || { echo "invalid target purpose: ${PURPOSE}" >&2; exit 2; }

source_lock_fingerprint() {
  python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
    --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
    --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
    --fingerprint
}

ensure_build_lockfile() {
  local source_fingerprint marker stored_fingerprint build_lock
  marker="${BUILD_REPO}/build/.cargo-source-lock-fingerprint"
  build_lock="${BUILD_REPO}/codex-rs/Cargo.lock"
  source_fingerprint="$(source_lock_fingerprint)"
  stored_fingerprint=''
  [[ -f "${marker}" ]] && read -r stored_fingerprint <"${marker}"
  if [[ -f "${build_lock}" ]] \
    && [[ "${source_fingerprint}" == "${stored_fingerprint}" ]] \
    && python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
      --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
      --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
      --build-lock "${build_lock}" \
    && cargo metadata --locked --offline --no-deps \
      --manifest-path "${BUILD_REPO}/codex-rs/Cargo.toml" >/dev/null 2>&1; then
    return
  fi

  mkdir -p "$(dirname "${marker}")"
  cp "${SOURCE_REPO}/codex-rs/Cargo.lock" "${build_lock}"
  python3 "${SOURCE_REPO}/scripts/normalize_cargo_lock.py" \
    --manifest "${BUILD_REPO}/codex-rs/Cargo.toml" \
    --source-lock "${SOURCE_REPO}/codex-rs/Cargo.lock" \
    --build-lock "${build_lock}"
  echo "Synchronizing workspace package versions in the build-tree Cargo.lock." >&2
  if ! cargo metadata --locked --offline --no-deps \
    --manifest-path "${BUILD_REPO}/codex-rs/Cargo.toml" >/dev/null 2>&1; then
    echo "The source Cargo.lock is not valid for this workspace." >&2
    return 1
  fi
  printf '%s\n' "${source_fingerprint}" >"${marker}"
}

ensure_build_lockfile

RUSTC_BIN="${RUSTC:-rustc}"
HOST_TARGET="$(${RUSTC_BIN} -vV | sed -n 's/^host: //p')"
SCCACHE_BIN="$(command -v sccache || true)"
SCCACHE_WRAPPER="${BUILD_REPO}/scripts/codex_sccache_wrapper.sh"
case "${TARGET_MODE}" in
  native) TARGET="${HOST_TARGET}"; TARGET_ROOT="${BUILD_REPO}/codex-rs/target" ;;
  musl) TARGET=x86_64-unknown-linux-musl; BASE_TARGET_DIR="${BUILD_REPO}/build/musl-${MODE}" ;;
  armv7) TARGET="${ARMV7_TARGET:-armv7-unknown-linux-musleabihf}"; BASE_TARGET_DIR="${BUILD_REPO}/build/armv7-${MODE}" ;;
  android)
    TARGET=aarch64-linux-android
    if [[ "${HOST_TARGET}" == "${TARGET}" ]]; then
      TARGET_ROOT="${BUILD_REPO}/codex-rs/target"
    else
      BASE_TARGET_DIR="${BUILD_REPO}/build/android-${MODE}"
    fi
    ;;
  *) echo "invalid target mode: ${TARGET_MODE}" >&2; exit 2 ;;
esac
TARGET_ROOT="${TARGET_ROOT:-${BASE_TARGET_DIR}}"
RECIPE_TARGET_MODE="${TARGET_MODE}"
if [[ "${TARGET_MODE}" == android && "${HOST_TARGET}" == "${TARGET}" ]]; then
  RECIPE_TARGET_MODE=native
fi

LOCK_FILE="${BUILD_REPO}/codex-rs/Cargo.lock"
RUSTFLAGS_VALUE="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-}"
MOLD_BIN="$(command -v mold || true)"
DENY_WARNINGS_VALUE="${CODEX_DENY_WARNINGS:-1}"

if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  ANDROID_CLANG="$(command -v aarch64-linux-android-clang || true)"
  [[ -x "${ANDROID_CLANG}" ]] || { echo "Android linker not found" >&2; exit 1; }
  ANDROID_BUILTINS="$(${ANDROID_CLANG} -print-file-name=libclang_rt.builtins-aarch64-android.a)"
  [[ -s "${ANDROID_BUILTINS}" ]] || { echo "Android compiler builtins archive not found" >&2; exit 1; }
  ANDROID_TLS_ALIGNMENT_SOURCE="${SOURCE_REPO}/scripts/android_tls_alignment.S"
  [[ -f "${ANDROID_TLS_ALIGNMENT_SOURCE}" ]] || {
    echo "Android TLS alignment source not found" >&2
    exit 1
  }
  ANDROID_TLS_ALIGNMENT_FINGERPRINT="$({
    sha256sum "${ANDROID_TLS_ALIGNMENT_SOURCE}"
    "${ANDROID_CLANG}" --version
  } | sha256sum | awk '{print $1}')"
  ANDROID_TLS_ALIGNMENT_DIR="${BUILD_REPO}/build/android-tls-alignment"
  ANDROID_TLS_ALIGNMENT_OBJECT="${ANDROID_TLS_ALIGNMENT_DIR}/${ANDROID_TLS_ALIGNMENT_FINGERPRINT}.o"
  if [[ ! -s "${ANDROID_TLS_ALIGNMENT_OBJECT}" ]]; then
    mkdir -p "${ANDROID_TLS_ALIGNMENT_DIR}"
    "${ANDROID_CLANG}" -c "${ANDROID_TLS_ALIGNMENT_SOURCE}" \
      -o "${ANDROID_TLS_ALIGNMENT_OBJECT}.tmp"
    mv "${ANDROID_TLS_ALIGNMENT_OBJECT}.tmp" "${ANDROID_TLS_ALIGNMENT_OBJECT}"
  fi
  RUSTFLAGS_VALUE="-Clink-arg=${ANDROID_BUILTINS}"
  RUSTFLAGS_VALUE+=" -Clink-arg=${ANDROID_TLS_ALIGNMENT_OBJECT}"
  RUSTFLAGS_VALUE+=" -Clink-arg=-Wl,-u,codex_android_tls_alignment"
fi

if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  MOLD_AVAILABLE=false
  if [[ -n "${MOLD_BIN}" ]]; then
    mold_probe_dir="${TMPDIR:-${RUNNER_TEMP:-/var/tmp}}/codex-mold-probe"
    mkdir -p "${mold_probe_dir}"
    mold_probe_src="${mold_probe_dir}/probe.c"
    mold_probe_bin="${mold_probe_dir}/probe"
    printf '%s\n' 'int main(void) { return 0; }' >"${mold_probe_src}"
    if "${ANDROID_CLANG}" -fuse-ld=mold "${mold_probe_src}" -o "${mold_probe_bin}" >/dev/null 2>&1; then
      RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=mold"
      MOLD_AVAILABLE=true
    fi
    rm -f "${mold_probe_src}" "${mold_probe_bin}"
  fi
  if [[ "${MOLD_AVAILABLE}" != true ]]; then
    lld_bin="$(command -v ld.lld || true)"
    [[ -n "${lld_bin}" ]] || { echo "Neither Mold nor LLD is available for Android linking" >&2; exit 1; }
    mold_probe_dir="${TMPDIR:-${RUNNER_TEMP:-/var/tmp}}/codex-lld-probe"
    mkdir -p "${mold_probe_dir}"
    mold_probe_src="${mold_probe_dir}/probe.c"
    mold_probe_bin="${mold_probe_dir}/probe"
    printf '%s\n' 'int main(void) { return 0; }' >"${mold_probe_src}"
    if ! "${ANDROID_CLANG}" -fuse-ld=lld "${mold_probe_src}" -o "${mold_probe_bin}" >/dev/null 2>&1; then
      rm -f "${mold_probe_src}" "${mold_probe_bin}"
      echo "Neither Mold nor LLD can link the native Android target" >&2
      exit 1
    fi
    rm -f "${mold_probe_src}" "${mold_probe_bin}"
    RUSTFLAGS_VALUE+=" -Clink-arg=-fuse-ld=lld"
  fi
fi

if [[ "${DENY_WARNINGS_VALUE}" == 1 ]]; then
  RUSTFLAGS_VALUE+=" -D warnings"
fi

record_artifact_operation() {
  local ledger lock_file timestamp source_revision source_state_fingerprint
  local rustc_fingerprint operation fingerprint
  ledger="${BUILD_REPO}/build/cargo-artifact-operations.jsonl"
  lock_file="${ledger}.lock"
  timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  source_revision="$(git -C "${SOURCE_REPO}" rev-parse HEAD 2>/dev/null || printf '%s' unknown)"
  source_state_fingerprint="$({
    git -C "${SOURCE_REPO}" diff --no-ext-diff --binary HEAD
    git -C "${SOURCE_REPO}" status --porcelain=v1 --untracked-files=all
  } | sha256sum | awk '{print $1}')"
  rustc_fingerprint="$("${RUSTC_BIN}" -vV | sha256sum | awk '{print $1}')"
  operation="${CODEX_CARGO_OPERATION:-${PURPOSE}}"
  fingerprint="$({
    printf '%s\0' \
      "${source_revision}" "${MODE}" "${TARGET_MODE}" "${RECIPE_TARGET_MODE}" \
      "${PURPOSE}" "${operation}" "${TARGET}" "${TARGET_DIR}" \
      "${RUSTFLAGS_VALUE}" "${rustc_fingerprint}" "${source_state_fingerprint}" \
      "${OPENSSL_VERSION}" "${RUSTY_V8_VERSION}"
  } | sha256sum | awk '{print $1}')"

  export CODEX_ARTIFACT_TIMESTAMP="${timestamp}"
  export CODEX_ARTIFACT_OPERATION="${operation}"
  export CODEX_ARTIFACT_PURPOSE="${PURPOSE}"
  export CODEX_ARTIFACT_MODE="${MODE}"
  export CODEX_ARTIFACT_TARGET_MODE="${TARGET_MODE}"
  export CODEX_ARTIFACT_TARGET="${TARGET}"
  export CODEX_ARTIFACT_TARGET_DIR="${TARGET_DIR}"
  export CODEX_ARTIFACT_SOURCE_REVISION="${source_revision}"
  export CODEX_ARTIFACT_SOURCE_STATE_FINGERPRINT="${source_state_fingerprint}"
  export CODEX_ARTIFACT_FINGERPRINT="${fingerprint}"

  mkdir -p "$(dirname "${ledger}")"
  python3 - "${ledger}" "${lock_file}" <<'PY'
import fcntl
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

ledger, lock_path = sys.argv[1:]
record = {
    "timestamp": os.environ["CODEX_ARTIFACT_TIMESTAMP"],
    "operation": os.environ["CODEX_ARTIFACT_OPERATION"],
    "purpose": os.environ["CODEX_ARTIFACT_PURPOSE"],
    "mode": os.environ["CODEX_ARTIFACT_MODE"],
    "target_mode": os.environ["CODEX_ARTIFACT_TARGET_MODE"],
    "target": os.environ["CODEX_ARTIFACT_TARGET"],
    "target_dir": os.environ["CODEX_ARTIFACT_TARGET_DIR"],
    "source_revision": os.environ["CODEX_ARTIFACT_SOURCE_REVISION"],
    "source_state_fingerprint": os.environ["CODEX_ARTIFACT_SOURCE_STATE_FINGERPRINT"],
    "fingerprint": os.environ["CODEX_ARTIFACT_FINGERPRINT"],
}
with open(lock_path, "a", encoding="utf-8") as lock:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print(f"Waiting for artifact operation ledger lock: {lock_path}", file=sys.stderr)
        registry = pathlib.Path(os.path.dirname(lock_path)) / "codex-build-processes"
        for record_path in sorted(registry.glob("*")):
            try:
                fields = dict(
                    line.split("=", 1)
                    for line in record_path.read_text(encoding="utf-8").splitlines()
                    if "=" in line
                )
                pid = int(fields["pid"])
                os.kill(pid, 0)
            except (FileNotFoundError, KeyError, OSError, ValueError):
                continue
            print(
                f"  pid={pid} started={fields.get('started', 'unknown')} "
                f"command={fields.get('command', 'unknown')}",
                file=sys.stderr,
            )
        holder_pids = []
        if shutil.which("fuser"):
            holder_pids = subprocess.run(
                ["fuser", lock_path], capture_output=True, text=True, check=False
            ).stdout.split()
        if holder_pids:
            subprocess.run(
                ["ps", "-ww", "-o", "pid,ppid,etime,args", "-p", ",".join(holder_pids)],
                stdout=sys.stderr,
                check=False,
            )
        else:
            print("No current ledger lock holder was reported.", file=sys.stderr)
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                time.sleep(1)
    with open(ledger, "a", encoding="utf-8") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")
        output.flush()
PY
}

OPENSSL_VERSION="$(bash "${SOURCE_REPO}/scripts/openssl_artifacts.sh" version "${LOCK_FILE}")"
OPENSSL_DIR_VALUE=''
if OPENSSL_ENV="$(bash "${SOURCE_REPO}/scripts/openssl_artifacts.sh" env "${BUILD_REPO}" "${TARGET}" "${OPENSSL_VERSION}" 2>/dev/null)"; then
  eval "${OPENSSL_ENV}"
  OPENSSL_DIR_VALUE="${OPENSSL_DIR}"
fi

RUSTY_V8_VERSION="$(python3 "${SOURCE_REPO}/scripts/rusty_v8_version.py" "${LOCK_FILE}")"
RUSTY_V8_DIR="${BUILD_REPO}/build/rusty-v8-artifacts/${RUSTY_V8_VERSION}/${TARGET}"
RUSTY_V8_ARCHIVE=''
RUSTY_V8_SRC_BINDING_PATH=''
if RUSTY_V8_ENV="$(bash "${SOURCE_REPO}/scripts/resolve_rusty_v8_artifacts.sh" \
  --target="${TARGET}" --output-dir="${RUSTY_V8_DIR}" \
  --cargo-build-dir="${TARGET_ROOT}" 2>/dev/null)"; then
  eval "${RUSTY_V8_ENV}"
elif [[ "${TARGET_MODE}" == native ]]; then
  echo "Rusty V8 artifacts are unavailable for the native target" >&2
  exit 1
fi

TARGET_DIR="${TARGET_ROOT}"
# The explicit override below intentionally permits concurrent writes to the
# shared target directory. The build script still records the process in its
# registry so later invocations can show all active and waiting builds.
# Keep the shared Cargo lock outside the target directory so `cargo clean`
# cannot unlink it, and inside the ignored build directory so source-to-build
# synchronization cannot unlink it while another process still holds it.
TARGET_LOCK_FILE="${BUILD_REPO}/build/codex-cargo.lock"
mkdir -p "$(dirname "${TARGET_LOCK_FILE}")"

mkdir -p "${TARGET_DIR}"

if [[ "${OUTPUT}" == target ]]; then
  printf '%s\n' "${TARGET_DIR}"
  exit 0
fi

record_artifact_operation

printf '%s\n' 'unset CC CXX AR RANLIB CFLAGS CXXFLAGS TARGET_CC TARGET_CXX TARGET_AR TARGET_RANLIB PKG_CONFIG_ALLOW_CROSS PKG_CONFIG_ALL_STATIC PKG_CONFIG_PATH PKG_CONFIG_LIBDIR PKG_CONFIG_SYSROOT_DIR CMAKE_C_COMPILER CMAKE_CXX_COMPILER CMAKE_ARGS'
printf 'export RUSTUP_DISABLE_SELF_UPDATE=1 CARGO_TARGET_DIR=%q CODEX_BUILD_TIMESTAMP=0000000000-000000000000\n' "${TARGET_DIR}"
if [[ "${CODEX_ALLOW_CONCURRENT_BUILD:-false}" == true ]]; then
  printf 'echo %q >&2\n' "Concurrent build override enabled; sharing Cargo target directory ${TARGET_DIR}."
elif command -v flock >/dev/null 2>&1; then
  printf 'TARGET_LOCK_FILE=%q\n' "${TARGET_LOCK_FILE}"
  printf 'exec 9>>%q\n' "${TARGET_LOCK_FILE}"
  printf '%s\n' 'if ! flock -n 9; then'
  printf '  echo %q >&2\n' "Waiting for Cargo target lock: ${TARGET_LOCK_FILE}"
  printf '  echo %q >&2\n' 'Cargo build processes currently registered:'
  printf '  registry=%q\n' "${BUILD_REPO}/build/codex-build-processes"
  printf '  if [[ -d "${registry}" ]]; then\n'
  printf '    for record in "${registry}"/*; do\n'
  printf '      [[ -f "${record}" ]] || continue\n'
  printf '      pid="$(sed -n '\''s/^pid=//p'\'' "${record}")"\n'
  printf '      if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then\n'
  printf '        command="$(sed -n '\''s/^command=//p'\'' "${record}")"\n'
  printf '        started="$(sed -n '\''s/^started=//p'\'' "${record}")"\n'
  printf '        printf '\''  pid=%%s started=%%s command=%%s\\n'\'' "${pid}" "${started}" "${command}" >&2\n'
  printf '      else\n'
  printf '        rm -f "${record}"\n'
  printf '      fi\n'
  printf '    done\n'
  printf '  fi\n'
  printf '  holder_pids="$(fuser "${TARGET_LOCK_FILE}" 2>/dev/null || true)"\n'
  printf '  if [[ -n "${holder_pids}" ]]; then\n'
  printf '    echo %q >&2\n' 'Process(es) holding the lock:'
  printf '    ps -ww -o pid,ppid,etime,args -p "${holder_pids// /,}" >&2 || true\n'
  printf '  else\n'
  printf '    echo %q >&2\n' 'No current lock holder was reported; the lock may be transitioning or held by a process outside the current PID namespace.'
  printf '  fi\n'
  printf '  echo %q >&2\n' 'Waiting for the lock to be released...'
  printf 'fi\nflock -x 9\n'
else
  echo "warning: flock unavailable; Cargo target writes will not be serialized" >&2
fi
if [[ -n "${SCCACHE_BIN}" ]]; then
  printf 'export RUSTC_WRAPPER=%q CODEX_SCCACHE_BIN=%q SCCACHE_DIR=%q SCCACHE_CACHE_SIZE=%q\n' \
    "${SCCACHE_WRAPPER}" "${SCCACHE_BIN}" "${SCCACHE_DIR:-${HOME}/.cache/sccache}" \
    "${SCCACHE_CACHE_SIZE:-20G}"
fi
[[ -n "${CARGO_BUILD_JOBS:-}" ]] && printf 'export CARGO_BUILD_JOBS=%q\n' "${CARGO_BUILD_JOBS}"
if [[ -n "${OPENSSL_DIR_VALUE}" ]]; then
  printf 'export OPENSSL_DIR=%q OPENSSL_NO_VENDOR=1 OPENSSL_STATIC=1\n' "${OPENSSL_DIR_VALUE}"
fi
if [[ -n "${RUSTY_V8_ARCHIVE}" ]]; then
  printf 'export RUSTY_V8_ARCHIVE=%q RUSTY_V8_SRC_BINDING_PATH=%q\n' "${RUSTY_V8_ARCHIVE}" "${RUSTY_V8_SRC_BINDING_PATH}"
fi
if [[ "${TARGET}" == aarch64-linux-android && "${HOST_TARGET}" == aarch64-linux-android ]]; then
  printf 'export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-${CODEX_CARGO_BUILD_JOBS:-1}} CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=%q CC_aarch64_linux_android=%q CXX_aarch64_linux_android=%q AR_aarch64_linux_android=llvm-ar RANLIB_aarch64_linux_android=llvm-ranlib CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS=%q PROTOC=%q\n' \
    "${ANDROID_CLANG}" "${ANDROID_CLANG}" "${ANDROID_CLANG}++" "${RUSTFLAGS_VALUE}" "$(command -v protoc)"
elif [[ "${DENY_WARNINGS_VALUE}" == 1 ]]; then
  printf 'export RUSTFLAGS=\"${RUSTFLAGS:-} -D warnings\"\n'
fi
