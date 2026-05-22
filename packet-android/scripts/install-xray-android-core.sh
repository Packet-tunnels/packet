#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="$ROOT_DIR/app/src/main/jniLibs"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

usage() {
  cat <<'USAGE'
Install Xray Android binaries for Packet VLESS Reality support.

Set one or both inputs to a local Xray Android zip/binary path or an https URL:

  XRAY_ARM64=/path/to/Xray-android-arm64-v8a.zip \
  XRAY_X86_64=/path/to/Xray-android-amd64.zip \
  packet-android/scripts/install-xray-android-core.sh

The script stores each executable as app/src/main/jniLibs/<abi>/libpacket_xray.so
so Android extracts it into nativeLibraryDir where ProcessBuilder can execute it.
USAGE
}

fetch_source() {
  local source="$1"
  local output="$2"
  if [[ "$source" =~ ^https?:// ]]; then
    curl -LfsS "$source" -o "$output"
  else
    cp "$source" "$output"
  fi
}

install_for_abi() {
  local abi="$1"
  local source="${2:-}"
  if [[ -z "$source" ]]; then
    return
  fi

  local abi_dir="$DEST_DIR/$abi"
  local target="$abi_dir/libpacket_xray.so"
  local staged="$TMP_DIR/$abi-input"
  mkdir -p "$abi_dir"
  fetch_source "$source" "$staged"

  if unzip -tq "$staged" >/dev/null 2>&1; then
    local member
    member="$(unzip -Z1 "$staged" | awk '/(^|\/)xray$/ { print; exit }')"
    if [[ -z "$member" ]]; then
      echo "No xray executable found in $source" >&2
      exit 1
    fi
    unzip -p "$staged" "$member" > "$target"
  else
    cp "$staged" "$target"
  fi

  chmod 0755 "$target"
  echo "Installed $target"
}

if [[ -z "${XRAY_ARM64:-}" && -z "${XRAY_X86_64:-}" ]]; then
  usage
  exit 2
fi

install_for_abi "arm64-v8a" "${XRAY_ARM64:-}"
install_for_abi "x86_64" "${XRAY_X86_64:-}"
