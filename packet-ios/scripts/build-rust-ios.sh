#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.0}"

DEVELOPER_DIR_PATH="${DEVELOPER_DIR:-$(xcode-select -p)}"
IPHONEOS_SDK_PATH="$(printf '%s\n' "$DEVELOPER_DIR_PATH"/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS*.sdk | head -n 1)"
IPHONESIMULATOR_SDK_PATH="$(printf '%s\n' "$DEVELOPER_DIR_PATH"/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator*.sdk | head -n 1)"

if [[ ! -d "$IPHONEOS_SDK_PATH" || ! -d "$IPHONESIMULATOR_SDK_PATH" ]]; then
  echo "Unable to locate iOS SDK paths under $DEVELOPER_DIR_PATH" >&2
  exit 1
fi

TARGETS=(
  "aarch64-apple-ios"
  "aarch64-apple-ios-sim"
  "x86_64-apple-ios"
)

missing_targets=()
for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    missing_targets+=("$target")
  fi
done

if ((${#missing_targets[@]} > 0)); then
  echo "Missing Rust iOS targets: ${missing_targets[*]}" >&2
  echo "Install them with:" >&2
  echo "  rustup target add ${missing_targets[*]}" >&2
  exit 1
fi

for target in "${TARGETS[@]}"; do
  echo "==> Building phantom-client for $target"
  case "$target" in
    aarch64-apple-ios)
      export SDKROOT="$IPHONEOS_SDK_PATH"
      ;;
    *)
      export SDKROOT="$IPHONESIMULATOR_SDK_PATH"
      ;;
  esac
  cargo build -p phantom-client --release --target "$target"
done

unset SDKROOT

echo
echo "Rust iOS libraries built:"
for target in "${TARGETS[@]}"; do
  echo "  target/$target/release/libphantom_client.a"
done
