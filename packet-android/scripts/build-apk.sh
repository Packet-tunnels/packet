#!/usr/bin/env bash
set -euo pipefail

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRADLE_BIN="${GRADLE_BIN:-$HOME/.gradle/wrapper/dists/gradle-8.9-bin/90cnw93cvbtalezasaz0blq0a/gradle-8.9/bin/gradle}"
GRADLE_USER_HOME="${GRADLE_USER_HOME:-/tmp/phantom-gradle-home}"

bash "$ANDROID_DIR/scripts/build-rust-android.sh"

if [[ ! -x "$GRADLE_BIN" ]]; then
  echo "Gradle binary not found at $GRADLE_BIN" >&2
  exit 1
fi

cd "$ANDROID_DIR"
GRADLE_USER_HOME="$GRADLE_USER_HOME" "$GRADLE_BIN" --no-daemon assembleDebug

echo
echo "APK built at:"
echo "  $ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk"
