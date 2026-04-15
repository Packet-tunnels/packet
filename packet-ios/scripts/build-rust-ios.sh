#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

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
  cargo build -p phantom-client --release --target "$target"
done

echo
echo "Rust iOS libraries built:"
for target in "${TARGETS[@]}"; do
  echo "  target/$target/release/libphantom_client.a"
done
