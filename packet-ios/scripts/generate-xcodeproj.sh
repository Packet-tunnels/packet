#!/usr/bin/env bash
set -euo pipefail

IOS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$IOS_DIR"

if ! command -v xcodegen >/dev/null 2>&1; then
  echo "xcodegen is not installed." >&2
  echo "Install it with:" >&2
  echo "  brew install xcodegen" >&2
  exit 1
fi

xcodegen generate --spec project.yml

echo
echo "Generated: $IOS_DIR/Packet.xcodeproj"
