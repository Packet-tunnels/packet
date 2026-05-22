#!/usr/bin/env bash
set -euo pipefail

IOS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT_DIR="$(cd "$IOS_DIR/.." && pwd)"
ZIP_PATH="${PSIPHON_IOS_BUILD_ZIP:-$ROOT_DIR/packet-android/.psiphon-core-lab/psiphon-tunnel-core-binaries/ios/build.zip}"
FRAMEWORKS_DIR="$IOS_DIR/Frameworks"

if [[ ! -f "$ZIP_PATH" ]]; then
  cat >&2 <<EOF
Missing Psiphon iOS build archive:
  $ZIP_PATH

Fetch Psiphon binaries first:
  bash packet-android/scripts/psiphon-core-lab.sh fetch
EOF
  exit 1
fi

mkdir -p "$FRAMEWORKS_DIR"
rm -rf "$FRAMEWORKS_DIR/PsiphonTunnel.xcframework"
unzip -q "$ZIP_PATH" "PsiphonTunnel.xcframework/*" -d "$FRAMEWORKS_DIR"

echo "Installed: $FRAMEWORKS_DIR/PsiphonTunnel.xcframework"
