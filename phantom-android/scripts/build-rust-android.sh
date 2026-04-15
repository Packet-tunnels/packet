#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JNI_LIBS_DIR="$ANDROID_DIR/app/src/main/jniLibs"
NDK_VERSION="${ANDROID_NDK_VERSION:-27.1.12297006}"
NDK_ROOT="${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}/ndk/$NDK_VERSION"
TOOLCHAIN_ROOT="$NDK_ROOT/toolchains/llvm/prebuilt/darwin-x86_64/bin"
API_LEVEL="${ANDROID_API_LEVEL:-24}"

TARGETS=(
  "arm64-v8a|aarch64-linux-android|aarch64_linux_android"
  "x86_64|x86_64-linux-android|x86_64_linux_android"
)

missing_targets=()
for entry in "${TARGETS[@]}"; do
  rest="${entry#*|}"
  target="${rest%%|*}"
  if ! rustup target list --installed | grep -qx "$target"; then
    missing_targets+=("$target")
  fi
done

if [[ ! -d "$NDK_ROOT" ]]; then
  echo "Android NDK not found at $NDK_ROOT" >&2
  exit 1
fi

if ((${#missing_targets[@]} > 0)); then
  echo "Missing Rust Android targets: ${missing_targets[*]}" >&2
  echo "Install them with:" >&2
  echo "  rustup target add ${missing_targets[*]}" >&2
  exit 1
fi

mkdir -p "$JNI_LIBS_DIR"

for entry in "${TARGETS[@]}"; do
  abi="${entry%%|*}"
  rest="${entry#*|}"
  target="${rest%%|*}"
  cc_env_target="${entry##*|}"
  cargo_env_target="$(echo "$target" | tr '[:lower:]-' '[:upper:]_')"

  case "$target" in
    aarch64-linux-android)
      linker_prefix="aarch64-linux-android"
      clang="$TOOLCHAIN_ROOT/${linker_prefix}${API_LEVEL}-clang"
      clangxx="$TOOLCHAIN_ROOT/${linker_prefix}${API_LEVEL}-clang++"
      ;;
    x86_64-linux-android)
      linker_prefix="x86_64-linux-android"
      clang="$TOOLCHAIN_ROOT/${linker_prefix}${API_LEVEL}-clang"
      clangxx="$TOOLCHAIN_ROOT/${linker_prefix}${API_LEVEL}-clang++"
      ;;
    *)
      echo "Unsupported target: $target" >&2
      exit 1
      ;;
  esac

  echo "==> Building phantom-client for $target"

  env \
    "CARGO_TARGET_${cargo_env_target}_LINKER=$clang" \
    "CC_${cc_env_target}=$clang" \
    "CXX_${cc_env_target}=$clangxx" \
    "AR_${cc_env_target}=$TOOLCHAIN_ROOT/llvm-ar" \
    cargo build -p phantom-client --release --target "$target"

  mkdir -p "$JNI_LIBS_DIR/$abi"
  cp "$ROOT_DIR/target/$target/release/libphantom_client.so" "$JNI_LIBS_DIR/$abi/"
done

echo
echo "Rust Android libraries copied into:"
echo "  $JNI_LIBS_DIR"
