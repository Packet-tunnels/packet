#!/usr/bin/env bash
set -euo pipefail

IOS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT_DIR="$(cd "$IOS_DIR/.." && pwd)"
PROJECT_PATH="$IOS_DIR/Packet.xcodeproj"
DERIVED_DATA_PATH="${DERIVED_DATA_PATH:-/tmp/packet-ios-derived-data}"

"$IOS_DIR/scripts/build-rust-ios.sh"
"$IOS_DIR/scripts/generate-xcodeproj.sh"

if [[ ! -d "$PROJECT_PATH" ]]; then
  echo "Missing generated project at $PROJECT_PATH" >&2
  exit 1
fi

cd "$ROOT_DIR"

xcodebuild \
  -project "$PROJECT_PATH" \
  -scheme Packet \
  -configuration Debug \
  -sdk iphonesimulator \
  -destination "generic/platform=iOS Simulator" \
  -derivedDataPath "$DERIVED_DATA_PATH" \
  build

echo
echo "App build finished."
echo "Derived data: $DERIVED_DATA_PATH"
