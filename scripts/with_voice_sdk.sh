#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: with_voice_sdk.sh --build-repo DIR -- COMMAND [ARGS...]" >&2
  exit 2
}

BUILD_REPO=""
CALLER_CWD="$PWD"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-repo) [[ $# -ge 2 ]] || usage; BUILD_REPO="$2"; shift 2 ;;
    --) shift; break ;;
    *) usage ;;
  esac
done
[[ -n "${BUILD_REPO}" && $# -gt 0 ]] || usage
COMMAND=("$@")

bazelisk_without_cargo_lock() {
  bazelisk "$@" 9>&-
}

# Focused tests for other packages should not trigger the substantial native
# voice build. The default `just test` invocation is the whole workspace, so it
# prepares the pinned voice SDK and includes codex-voice-host normally.
voice_requested=0
package_selected=0
for ((index = 0; index < ${#COMMAND[@]}; index++)); do
  arg="${COMMAND[index]}"
  case "${arg}" in
    -p|--package)
      ((index + 1 < ${#COMMAND[@]})) || usage
      package_selected=1
      [[ "${COMMAND[index + 1]}" == "codex-voice-host" ]] && voice_requested=1
      ((index += 1))
      ;;
    -p=*|--package=*)
      package_selected=1
      [[ "${arg#*=}" == "codex-voice-host" ]] && voice_requested=1
      ;;
    *) ;;
  esac
done

# No package selector means Cargo will test the full workspace. Build the
# repository-pinned GStreamer SDK/runtime used by the Bazel voice target rather
# than consulting the host's pkg-config database or distro GStreamer packages.
if ((package_selected == 0)); then
  voice_requested=1
fi
if [[ "$(rustc -vV | sed -n 's/^host: //p')" == *-android ]]; then
  voice_requested=0
fi

if ((voice_requested)); then
  case "$(uname -s)" in
    Linux) library_path_variable=LD_LIBRARY_PATH ;;
    Darwin) library_path_variable=DYLD_FALLBACK_LIBRARY_PATH ;;
    *)
      echo "pinned Cargo voice SDK setup is not implemented for $(uname -s)" >&2
      exit 1
      ;;
  esac

  command -v bazelisk >/dev/null 2>&1 || {
    echo "bazelisk is required to prepare the pinned voice SDK (.bazelversion)" >&2
    exit 1
  }
  cd "${BUILD_REPO}"
  bazelisk_without_cargo_lock build \
    //third_party/voice:native_sdk \
    //third_party/voice:native_link \
    //third_party/voice:pkg_config

  query_file() {
    local label="$1"
    local -a results=()
    mapfile -t results < <(bazelisk_without_cargo_lock cquery "${label}" --output=files)
    if [[ "${label}" == "//third_party/voice:pkg_config" ]]; then
      # Bazel may report both target- and exec-configured copies of this
      # executable. Keep only the materialized runnable copy for this host.
      local -a executable_results=()
      for result in "${results[@]}"; do
        [[ -f "${result}" && -x "${result}" ]] && executable_results+=("${result}")
      done
      if ((${#executable_results[@]} > 0)); then
        results=("${executable_results[@]}")
      fi
    fi
    if ((${#results[@]} != 1)); then
      printf 'expected one output for %s, got %s: %s\n' \
        "${label}" "${#results[@]}" "${results[*]}" >&2
      exit 1
    fi
    realpath -e "${results[0]}"
  }

  sdk_dir="$(query_file //third_party/voice:native_sdk)"
  native_link_locator="$(query_file //third_party/voice:native_link)"
  pkg_config_bin="$(query_file //third_party/voice:pkg_config)"
  native_lib_dir="$(dirname "${native_link_locator}")"
  voice_runtime_dir="$(dirname "${native_lib_dir}")"
  [[ -d "${sdk_dir}/lib/pkgconfig" ]] || {
    echo "pinned voice SDK has no lib/pkgconfig directory: ${sdk_dir}" >&2
    exit 1
  }
  [[ -x "${pkg_config_bin}" ]] || {
    echo "pinned pkg-config tool is not executable: ${pkg_config_bin}" >&2
    exit 1
  }
  [[ -e "${native_lib_dir}/libgstreamer-1.0.so.0" || -e "${native_lib_dir}/libgstreamer-1.0.0.dylib" ]] || {
    echo "pinned voice runtime libraries were not produced under ${native_lib_dir}" >&2
    exit 1
  }
  [[ -f "${voice_runtime_dir}/runtime.json" ]] || {
    echo "pinned voice runtime receipt was not produced under ${voice_runtime_dir}" >&2
    exit 1
  }
  system_pc_path=""
  if [[ "$(uname -s)" == Linux ]]; then
    command -v pkg-config >/dev/null 2>&1 || {
      echo "pkg-config is required to discover host native package metadata paths" >&2
      exit 1
    }
    system_pc_path="$(pkg-config --variable=pc_path pkg-config)"
  fi
  for plugin in app audioconvert audioresample coreelements opus rtp rtpmanager; do
    if [[ "$(uname -s)" == Linux ]]; then
      plugin_path="${voice_runtime_dir}/lib/gstreamer-1.0/libgst${plugin}.so"
    else
      plugin_path="${voice_runtime_dir}/plugins/libgst${plugin}.dylib"
    fi
    [[ -f "${plugin_path}" ]] || {
      echo "pinned voice plugin was not produced: ${plugin_path}" >&2
      exit 1
    }
  done

  cd "${CALLER_CWD}"
  export PKG_CONFIG="${pkg_config_bin}"
  # Prefer the pinned SDK’s GStreamer metadata, then allow unrelated host
  # native dependencies such as ALSA to use their normal pkg-config entries.
  export PKG_CONFIG_LIBDIR="${sdk_dir}/lib/pkgconfig${system_pc_path:+:${system_pc_path}}"
  export PKG_CONFIG_PATH=""
  export SYSTEM_DEPS_GLIB_2_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GOBJECT_2_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GIO_2_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GSTREAMER_1_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GSTREAMER_BASE_1_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GSTREAMER_APP_1_0_SEARCH_NATIVE="${native_lib_dir}"
  export SYSTEM_DEPS_GSTREAMER_AUDIO_1_0_SEARCH_NATIVE="${native_lib_dir}"
  export CODEX_TEST_VOICE_RUNTIME="${voice_runtime_dir}"
  export GST_PLUGIN_PATH=""
  export GST_PLUGIN_PATH_1_0=""
  export GST_PLUGIN_SYSTEM_PATH=""
  export GST_PLUGIN_SYSTEM_PATH_1_0=""
  export GST_REGISTRY="/dev/null"
  export GST_REGISTRY_UPDATE=no
  export GST_REGISTRY_FORK=no
  export "${library_path_variable}=${native_lib_dir}${!library_path_variable:+:${!library_path_variable}}"
fi

exec "${COMMAND[@]}"
