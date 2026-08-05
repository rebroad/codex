#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_TREE=""
for candidate in "${ROOT}.build" "${ROOT}.make"; do
  if [[ -d "${candidate}" ]]; then BUILD_TREE="${candidate}"; break; fi
done
[[ -n "${BUILD_TREE}" ]] || { echo "No sibling build tree found" >&2; exit 1; }

VERSION="${1:-}"
PROFILE="${2:-release}"
if [[ "${VERSION}" == --help || "${VERSION}" == -h ]]; then
  cat <<'EOF'
Usage: scripts/package-npm.sh [VERSION] [debug|release]
Packages Linux musl x64/ARMv7 and, when staged, Android ARM64 binaries.
Output defaults to the sibling build tree's npm-artifact directory.
EOF
  exit 0
fi
[[ -n "${VERSION}" ]] || VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${ROOT}/codex-rs/Cargo.toml" | head -n 1)"
[[ "${PROFILE}" == debug || "${PROFILE}" == release ]] || { echo "profile must be debug or release" >&2; exit 2; }

OUTPUT_DIR="${OUTPUT_DIR:-${BUILD_TREE}/npm-artifact}"
STAGE_ROOT="$(mktemp -d "${BUILD_TREE}/npm-stage.XXXXXX")"
trap 'rm -rf "${STAGE_ROOT}"' EXIT
COMMIT_SHORT="$(git -C "${ROOT}" rev-parse --short=12 HEAD)"
BUILD_TIMESTAMP="$(date +%Y%m%d%H%M)"
STRIP_TOOL="${STRIP:-}"
if [[ -z "${STRIP_TOOL}" ]]; then
  for candidate in llvm-strip strip; do
    if command -v "${candidate}" >/dev/null 2>&1; then STRIP_TOOL="${candidate}"; break; fi
  done
fi
[[ -n "${STRIP_TOOL}" ]] || { echo "llvm-strip or strip is required" >&2; exit 1; }
command -v npm >/dev/null 2>&1 || { echo "npm is required" >&2; exit 1; }

profile_path() { [[ "${1}" == release ]] && echo release || echo debug; }
require_binary() {
  [[ -x "${1}" ]] || {
    echo "Missing executable: ${1}" >&2
    echo "Build it with scripts/rebuild_codex.sh --release --build-npm-vendor" >&2
    exit 1
  }
}
patch_timestamp() {
  local binary="${1}"
  python3 - "${binary}" "${VERSION}" "${COMMIT_SHORT}-${BUILD_TIMESTAMP}" <<'PY'
import mmap, sys
from pathlib import Path

path = Path(sys.argv[1])
needle = (sys.argv[2] + "-000000000000-000000000000").encode()
replacement = (sys.argv[2] + "-" + sys.argv[3]).encode()
if len(needle) != len(replacement):
    raise SystemExit("timestamp placeholder width mismatch")
with path.open("r+b") as handle:
    with mmap.mmap(handle.fileno(), 0) as mapped:
        count = 0
        start = 0
        while True:
            index = mapped.find(needle, start)
            if index < 0:
                break
            mapped[index:index + len(needle)] = replacement
            count += 1
            start = index + len(needle)
        mapped.flush()
if count == 0:
    raise SystemExit(f"no timestamp placeholder found in {path}")
PY
}
write_native_package() {
  local dir="${1}" name="${2}" os="${3}" cpu="${4}" binary="${5}" strip_tool="${6}"
  require_binary "${binary}"
  mkdir -p "${dir}/bin"
  cat >"${dir}/package.json" <<EOF
{
  "name": "${name}",
  "version": "${VERSION}",
  "description": "Codex CLI native binary for ${os}/${cpu}",
  "license": "Apache-2.0",
  "os": ["${os}"],
  "cpu": ["${cpu}"],
  "bin": {"codex": "bin/codex"},
  "files": ["bin"]
}
EOF
  install -m 0755 "${binary}" "${dir}/bin/codex"
  patch_timestamp "${dir}/bin/codex"
  "${strip_tool}" --strip-all "${dir}/bin/codex"
  npm pack --ignore-scripts --pack-destination "${OUTPUT_DIR}" "${dir}" >/dev/null
}
mkdir -p "${OUTPUT_DIR}"

MUSL_BIN="${BUILD_TREE}/cargo-target-musl-${PROFILE}/x86_64-unknown-linux-musl/$(profile_path "${PROFILE}")/codex"
ARMV7_BIN="${BUILD_TREE}/cargo-target-armv7-${PROFILE}/${ARMV7_TARGET:-armv7-unknown-linux-gnueabihf}/$(profile_path "${PROFILE}")/codex"
ANDROID_STAGE="${BUILD_TREE}/android-artifact"
ARMV7_STRIP="${ARMV7_STRIP:-}"
if [[ -z "${ARMV7_STRIP}" ]]; then
  ARMV7_STRIP="$(command -v arm-linux-gnueabihf-strip || true)"
fi
[[ -n "${ARMV7_STRIP}" ]] || {
  echo "arm-linux-gnueabihf-strip is required for the ARMv7 npm package" >&2
  exit 1
}

write_native_package "${STAGE_ROOT}/linux-x64" "@reb.ai/codex-linux-x64" linux x64 "${MUSL_BIN}" "${STRIP_TOOL}"
write_native_package "${STAGE_ROOT}/linux-armv7" "@reb.ai/codex-linux-armv7" linux arm "${ARMV7_BIN}" "${ARMV7_STRIP}"

if [[ -x "${ANDROID_STAGE}/codex.bin" && -f "${ANDROID_STAGE}/libc++_shared.so" ]]; then
  ANDROID_OPTIONAL_DEPENDENCY=$',\n    "@reb.ai/codex-android-arm64": "'"${VERSION}"'"'
  mkdir -p "${STAGE_ROOT}/android-arm64/bin"
  cat >"${STAGE_ROOT}/android-arm64/package.json" <<EOF
{
  "name": "@reb.ai/codex-android-arm64",
  "version": "${VERSION}",
  "description": "Codex CLI native binary for Android/arm64",
  "license": "Apache-2.0",
  "os": ["linux"],
  "cpu": ["arm64"],
  "bin": {"codex": "bin/codex"},
  "files": ["bin"]
}
EOF
  install -m 0755 "${ANDROID_STAGE}/codex.bin" "${STAGE_ROOT}/android-arm64/bin/codex"
  install -m 0644 "${ANDROID_STAGE}/libc++_shared.so" "${STAGE_ROOT}/android-arm64/bin/libc++_shared.so"
  npm pack --ignore-scripts --pack-destination "${OUTPUT_DIR}" "${STAGE_ROOT}/android-arm64" >/dev/null
fi
ANDROID_OPTIONAL_DEPENDENCY="${ANDROID_OPTIONAL_DEPENDENCY:-}"

mkdir -p "${STAGE_ROOT}/root/bin"
cat >"${STAGE_ROOT}/root/package.json" <<EOF
{
  "name": "@reb.ai/codex",
  "version": "${VERSION}",
  "description": "Codex CLI with a platform-specific native package",
  "license": "Apache-2.0",
  "type": "module",
  "bin": {"codex": "bin/codex.js"},
  "files": ["bin/codex.js"],
  "engines": {"node": ">=16"},
  "optionalDependencies": {
    "@reb.ai/codex-linux-x64": "${VERSION}",
    "@reb.ai/codex-linux-armv7": "${VERSION}"${ANDROID_OPTIONAL_DEPENDENCY}
  }
}
EOF
cat >"${STAGE_ROOT}/root/bin/codex.js" <<'EOF'
#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
const require = createRequire(import.meta.url);
const termux = process.platform === "android" ||
  (process.platform === "linux" && process.env.PREFIX?.includes("/com.termux/"));
let packageName;
if (termux && process.arch === "arm64") packageName = "@reb.ai/codex-android-arm64";
else if (process.platform === "linux" && process.arch === "x64") packageName = "@reb.ai/codex-linux-x64";
else if (process.platform === "linux" && process.arch === "arm") packageName = "@reb.ai/codex-linux-armv7";
else throw new Error(`Unsupported platform: ${process.platform}/${process.arch}`);
let root;
try { root = dirname(require.resolve(`${packageName}/package.json`)); }
catch { throw new Error(`Missing optional dependency ${packageName}; reinstall @reb.ai/codex`); }
const binary = join(root, "bin", "codex");
const env = {...process.env, CODEX_MANAGED_BY_NPM: "1", CODEX_MANAGED_PACKAGE_ROOT: root, CODEX_SELF_EXE: binary};
if (termux) {
  const prefix = process.env.PREFIX || "/data/data/com.termux/files/usr";
  const blocked = new Set([`${prefix}/lib`, `${prefix}/libexec`]);
  env.LD_LIBRARY_PATH = [dirname(binary), ...(process.env.LD_LIBRARY_PATH || "").split(":").filter(p => p && !blocked.has(p))].join(":");
}
const child = spawn(binary, process.argv.slice(2), {stdio: "inherit", env});
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(signal, () => child.kill(signal));
child.on("error", error => { console.error(`Failed to launch Codex: ${error.message}`); process.exit(1); });
child.on("exit", (code, signal) => signal ? process.kill(process.pid, signal) : process.exit(code ?? 1));
EOF
chmod 0755 "${STAGE_ROOT}/root/bin/codex.js"
npm pack --ignore-scripts --pack-destination "${OUTPUT_DIR}" "${STAGE_ROOT}/root" >/dev/null
echo "Created @reb.ai/codex packages in ${OUTPUT_DIR}"
ls -lh "${OUTPUT_DIR}"/*.tgz
