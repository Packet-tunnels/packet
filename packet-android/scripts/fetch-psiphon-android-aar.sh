#!/usr/bin/env bash
set -euo pipefail

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAB_DIR="${PSIPHON_LAB_DIR:-$ANDROID_DIR/.psiphon-core-lab}"
REPO_DIR="$LAB_DIR/psiphon-tunnel-core-Android-library"
TARGET_DIR="$ANDROID_DIR/app/libs"
TARGET_AAR="$TARGET_DIR/psiphon-tunnel-core.aar"
REPO_URL="${PSIPHON_ANDROID_REPO_URL:-https://github.com/Psiphon-Labs/psiphon-tunnel-core-Android-library.git}"

mkdir -p "$LAB_DIR" "$TARGET_DIR"

if [[ ! -d "$REPO_DIR/.git" ]]; then
  git clone --depth 1 "$REPO_URL" "$REPO_DIR"
else
  git -C "$REPO_DIR" fetch --depth 1 origin
  git -C "$REPO_DIR" reset --hard origin/master
fi

latest_aar="$(
  find "$REPO_DIR/releases/ca/psiphon/psiphontunnel" -name '*.aar' -type f 2>/dev/null |
    sort -V |
    tail -n 1
)"

if [[ -z "$latest_aar" ]]; then
  echo "No Psiphon Android AAR found under $REPO_DIR/releases/ca/psiphon/psiphontunnel" >&2
  exit 1
fi

cp "$latest_aar" "$TARGET_AAR"

cat <<EOF
Psiphon Android AAR installed:
  $TARGET_AAR

Next:
  1. Put a generated client config at:
     $ANDROID_DIR/app/src/main/assets/psiphon/client.config
  2. Rebuild:
     bash packet-android/scripts/build-apk.sh

The app keeps this optional. If the AAR is missing, normal Packet builds still work.
EOF
