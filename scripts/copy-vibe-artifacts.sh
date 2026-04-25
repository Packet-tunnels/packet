#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VIBE_DIR="/Users/mohammadshayani/Vibe"

cp "$ROOT_DIR/target/aarch64-apple-ios/release/libphantom_client.a" \
  "$VIBE_DIR/ios/Vendor/Packet/ios-arm64/libphantom_client.a"
cp "$ROOT_DIR/target/aarch64-apple-ios-sim/release/libphantom_client.a" \
  "$VIBE_DIR/ios/Vendor/Packet/ios-sim-arm64/libphantom_client.a"
cp "$ROOT_DIR/target/x86_64-apple-ios/release/libphantom_client.a" \
  "$VIBE_DIR/ios/Vendor/Packet/ios-sim-x86_64/libphantom_client.a"

cp "$ROOT_DIR/packet-android/app/src/main/jniLibs/arm64-v8a/libphantom_client.so" \
  "$VIBE_DIR/android/app/src/main/jniLibs/arm64-v8a/libphantom_client.so"
cp "$ROOT_DIR/packet-android/app/src/main/jniLibs/x86_64/libphantom_client.so" \
  "$VIBE_DIR/android/app/src/main/jniLibs/x86_64/libphantom_client.so"
