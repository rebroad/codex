#!/bin/sh

set -eu

VERSION="${1:-latest}"
INSTALL_DIR="${CODEX_INSTALL_DIR:-$HOME/.local/bin}"
REPO="${CODEX_INSTALL_REPO:-rebroad/codex}"
path_action="already"
path_profile=""

step() {
  printf '==> %s\n' "$1"
}

normalize_version() {
  case "$1" in
    "" | latest)
      printf 'latest\n'
      ;;
    codex-v*)
      printf '%s\n' "${1#codex-v}"
      ;;
    v*)
      printf '%s\n' "${1#v}"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

download_file() {
  url="$1"
  output="$2"

  if command -v curl >/dev/null 2>&1; then
    if ! curl -fsSL "$url" -o "$output"; then
      echo "Failed to download: $url" >&2
      exit 1
    fi
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    if ! wget -q -O "$output" "$url"; then
      echo "Failed to download: $url" >&2
      exit 1
    fi
    return
  fi

  echo "curl or wget is required to install Codex." >&2
  exit 1
}

download_text() {
  url="$1"

  if command -v curl >/dev/null 2>&1; then
    if ! curl -fsSL "$url"; then
      echo "Failed to download: $url" >&2
      exit 1
    fi
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    if ! wget -q -O - "$url"; then
      echo "Failed to download: $url" >&2
      exit 1
    fi
    return
  fi

  echo "curl or wget is required to install Codex." >&2
  exit 1
}

add_to_path() {
  path_action="already"
  path_profile=""

  case ":$PATH:" in
    *":$INSTALL_DIR:"*)
      return
      ;;
  esac

  profile="$HOME/.profile"
  case "${SHELL:-}" in
    */zsh)
      profile="$HOME/.zshrc"
      ;;
    */bash)
      profile="$HOME/.bashrc"
      ;;
  esac

  path_profile="$profile"
  path_line="export PATH=\"$INSTALL_DIR:\$PATH\""
  if [ -f "$profile" ] && grep -F "$path_line" "$profile" >/dev/null 2>&1; then
    path_action="configured"
    return
  fi

  {
    printf '\n# Added by Codex installer\n'
    printf '%s\n' "$path_line"
  } >>"$profile"
  path_action="added"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to install Codex." >&2
    exit 1
  fi
}

require_command mktemp
require_command tar

resolve_version() {
  normalized_version="$(normalize_version "$VERSION")"

  if [ "$normalized_version" != "latest" ]; then
    printf '%s\n' "$normalized_version"
    return
  fi

  release_json="$(download_text "https://api.github.com/repos/${REPO}/releases/latest")"
  resolved="$(printf '%s\n' "$release_json" | sed -n 's/.*"tag_name":[[:space:]]*"codex-v\([^"]*\)".*/\1/p' | head -n 1)"

  if [ -z "$resolved" ]; then
    echo "Failed to resolve the latest Codex release version." >&2
    exit 1
  fi

  printf '%s\n' "$resolved"
}

case "$(uname -s)" in
  Linux)
    os="linux"
    ;;
  *)
    echo "This installer currently supports Linux only." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64)
    arch="x86_64"
    ;;
  arm64 | aarch64)
    arch="aarch64"
    ;;
  armv7l | armv7)
    arch="armv7"
    ;;
  *)
    echo "Unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [ "$arch" = "aarch64" ]; then
  target="aarch64-unknown-linux-gnu"
  platform_label="Linux (ARM64)"
elif [ "$arch" = "armv7" ]; then
  target="armv7-unknown-linux-gnueabihf"
  platform_label="Linux (ARMv7)"
else
  target="x86_64-unknown-linux-gnu"
  platform_label="Linux (x64)"
fi

if [ -x "$INSTALL_DIR/codex" ]; then
  install_mode="Updating"
else
  install_mode="Installing"
fi

step "$install_mode Codex CLI"
step "Detected platform: $platform_label"

resolved_version="$(resolve_version)"
asset="codex-${target}.tar.gz"
download_url="https://github.com/${REPO}/releases/download/codex-v${resolved_version}/${asset}"
sha_url="https://github.com/${REPO}/releases/download/codex-v${resolved_version}/${asset}.sha256"

step "Resolved version: $resolved_version"
step "Download URL: $download_url"

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

archive_path="$tmp_dir/$asset"
sha_path="$tmp_dir/$asset.sha256"

step "Downloading Codex CLI"
download_file "$download_url" "$archive_path"

if command -v sha256sum >/dev/null 2>&1; then
  if download_file "$sha_url" "$sha_path"; then
    step "Verifying checksum"
    (cd "$tmp_dir" && sha256sum -c "$(basename "$sha_path")")
  fi
fi

step "Installing to $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
tar -xzf "$archive_path" -C "$tmp_dir"
cp "$tmp_dir/codex" "$INSTALL_DIR/codex"
chmod 0755 "$INSTALL_DIR/codex"

add_to_path

case "$path_action" in
  added)
    step "PATH updated for future shells in $path_profile"
    step "Run now: export PATH=\"$INSTALL_DIR:\$PATH\" && codex"
    step "Or open a new terminal and run: codex"
    ;;
  configured)
    step "PATH is already configured for future shells in $path_profile"
    step "Run now: export PATH=\"$INSTALL_DIR:\$PATH\" && codex"
    step "Or open a new terminal and run: codex"
    ;;
  *)
    step "$INSTALL_DIR is already on PATH"
    step "Run: codex"
    ;;
esac

printf 'Codex CLI %s installed successfully.\n' "$resolved_version"
