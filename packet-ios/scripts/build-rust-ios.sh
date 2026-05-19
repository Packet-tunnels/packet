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

# BoringSSL (the Chrome-JA3 TLS engine, via the `boring` crate) is compiled
# from source by boring-sys. That needs cmake + go on PATH; the C/C++
# toolchain comes from Xcode via SDKROOT (set per-target below). Exporting
# IPHONEOS_DEPLOYMENT_TARGET + SDKROOT makes cmake build BoringSSL against
# the iOS SDK so the final link matches (otherwise: "Undefined symbols for
# architecture arm64"). Fail early if the build tools are missing.
missing_tools=()
command -v cmake >/dev/null 2>&1 || missing_tools+=("cmake")
command -v go    >/dev/null 2>&1 || missing_tools+=("go")
if ((${#missing_tools[@]} > 0)); then
  echo "Missing build tools required by BoringSSL: ${missing_tools[*]}" >&2
  echo "Install them with:" >&2
  echo "  brew install ${missing_tools[*]}" >&2
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
      SDK_NAME="iphoneos"
      export SDKROOT="$IPHONEOS_SDK_PATH"
      CLANG_TARGET="arm64-apple-ios"
      MIN_VERSION_FLAG="-miphoneos-version-min=${IPHONEOS_DEPLOYMENT_TARGET}"
      ;;
    aarch64-apple-ios-sim)
      SDK_NAME="iphonesimulator"
      export SDKROOT="$IPHONESIMULATOR_SDK_PATH"
      CLANG_TARGET="arm64-apple-ios-simulator"
      MIN_VERSION_FLAG="-mios-simulator-version-min=${IPHONEOS_DEPLOYMENT_TARGET}"
      ;;
    *)
      SDK_NAME="iphonesimulator"
      export SDKROOT="$IPHONESIMULATOR_SDK_PATH"
      CLANG_TARGET="x86_64-apple-ios-simulator"
      MIN_VERSION_FLAG="-mios-simulator-version-min=${IPHONEOS_DEPLOYMENT_TARGET}"
      ;;
  esac

  export CC="$(xcrun --sdk "$SDK_NAME" -f clang)"
  export CXX="$(xcrun --sdk "$SDK_NAME" -f clang++)"
  export AR="$(xcrun --sdk "$SDK_NAME" -f ar)"
  export CFLAGS="-target ${CLANG_TARGET} ${MIN_VERSION_FLAG} -isysroot ${SDKROOT}"
  export CXXFLAGS="$CFLAGS"
  export CPPFLAGS="$CFLAGS"
  export LFLAGS="$CFLAGS"

  TARGET_ENV_SUFFIX="${target//-/_}"
  export "CC_${TARGET_ENV_SUFFIX}=$CC"
  export "CXX_${TARGET_ENV_SUFFIX}=$CXX"
  export "AR_${TARGET_ENV_SUFFIX}=$AR"
  export "CFLAGS_${TARGET_ENV_SUFFIX}=$CFLAGS"
  export "CXXFLAGS_${TARGET_ENV_SUFFIX}=$CXXFLAGS"

  # CMake caches the compiler inside boring-sys. If PATH previously pointed
  # at another clang (for example Swiftly's), force a clean BoringSSL configure.
  rm -rf "$ROOT_DIR/target/$target/release/build"/boring-sys-*

  cargo build -p phantom-client --release --target "$target"
done

unset SDKROOT CC CXX AR CFLAGS CXXFLAGS CPPFLAGS LFLAGS

echo
echo "Rust iOS libraries built:"
for target in "${TARGETS[@]}"; do
  echo "  target/$target/release/libphantom_client.a"
done
