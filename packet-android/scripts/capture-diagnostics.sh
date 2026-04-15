#!/usr/bin/env bash
set -euo pipefail

PACKAGE="${PHANTOM_ANDROID_PACKAGE:-com.resolo.packet}"
OUTPUT_BASE="${PHANTOM_DIAG_OUTPUT_DIR:-$HOME/Desktop}"
SERIAL="${ADB_SERIAL:-}"
CAPTURE_SECONDS=""

usage() {
  cat <<'EOF'
Usage: capture-diagnostics.sh [--serial SERIAL] [--package PACKAGE] [--out DIR] [--duration SECONDS]

Starts an adb logcat capture, collects Packet app files and Android network state,
waits for you to reproduce the issue, then writes a zip bundle with a summary.

Examples:
  packet-android/scripts/capture-diagnostics.sh
  packet-android/scripts/capture-diagnostics.sh --serial emulator-5554 --duration 30
  PHANTOM_ANDROID_PACKAGE=com.resolo.packet packet-android/scripts/capture-diagnostics.sh
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --serial)
      SERIAL="${2:-}"
      shift 2
      ;;
    --package)
      PACKAGE="${2:-}"
      shift 2
      ;;
    --out)
      OUTPUT_BASE="${2:-}"
      shift 2
      ;;
    --duration)
      CAPTURE_SECONDS="${2:-}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd adb
need_cmd zip
need_cmd awk
need_cmd grep
need_cmd sed

pick_serial() {
  if [[ -n "$SERIAL" ]]; then
    return
  fi

  mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
  if ((${#devices[@]} == 0)); then
    echo "No adb device is connected." >&2
    exit 1
  fi

  if ((${#devices[@]} > 1)); then
    echo "Multiple adb devices are connected. Re-run with --serial." >&2
    printf 'Devices:\n' >&2
    printf '  %s\n' "${devices[@]}" >&2
    exit 1
  fi

  SERIAL="${devices[0]}"
}

pick_serial

adb_cmd() {
  adb -s "$SERIAL" "$@"
}

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BUNDLE_NAME="packet-android-diagnostics-$STAMP"
BUNDLE_DIR="$OUTPUT_BASE/$BUNDLE_NAME"
ZIP_PATH="$OUTPUT_BASE/$BUNDLE_NAME.zip"
LOGCAT_FILE="$BUNDLE_DIR/logcat.txt"
SUMMARY_FILE="$BUNDLE_DIR/summary.md"

mkdir -p \
  "$BUNDLE_DIR/host" \
  "$BUNDLE_DIR/device/before" \
  "$BUNDLE_DIR/device/after" \
  "$BUNDLE_DIR/app/before" \
  "$BUNDLE_DIR/app/after"

capture_host_text() {
  local relative="$1"
  shift
  "$@" > "$BUNDLE_DIR/$relative" 2>&1 || true
}

capture_shell() {
  local relative="$1"
  shift
  if ! adb_cmd shell "$@" > "$BUNDLE_DIR/$relative" 2>&1; then
    printf 'Command failed: adb -s %s shell %s\n' "$SERIAL" "$*" > "$BUNDLE_DIR/$relative"
  fi
}

capture_run_as() {
  local relative="$1"
  shift
  if ! adb_cmd exec-out run-as "$PACKAGE" "$@" > "$BUNDLE_DIR/$relative" 2>&1; then
    printf 'run-as capture failed for package %s: %s\n' "$PACKAGE" "$*" > "$BUNDLE_DIR/$relative"
  fi
}

capture_device_snapshot() {
  local phase="$1"
  capture_shell "device/$phase/date.txt" date
  capture_shell "device/$phase/getprop.txt" getprop
  capture_shell "device/$phase/ip_addr.txt" ip addr
  capture_shell "device/$phase/ip_route.txt" ip route
  capture_shell "device/$phase/private_dns_mode.txt" settings get global private_dns_mode
  capture_shell "device/$phase/private_dns_specifier.txt" settings get global private_dns_specifier
  capture_shell "device/$phase/auto_time.txt" settings get global auto_time
  capture_shell "device/$phase/auto_time_zone.txt" settings get global auto_time_zone
  capture_shell "device/$phase/connectivity.txt" dumpsys connectivity
  capture_shell "device/$phase/vpn.txt" dumpsys vpn
  capture_shell "device/$phase/netstats.txt" dumpsys netstats
  capture_shell "device/$phase/wifi.txt" dumpsys wifi
  capture_shell "device/$phase/package.txt" dumpsys package "$PACKAGE"
}

capture_app_snapshot() {
  local phase="$1"
  capture_run_as "app/$phase/packet.log" cat files/packet.log
  capture_run_as "app/$phase/packet_preferences.xml" cat shared_prefs/packet_preferences.xml
}

line_count() {
  local file="$1"
  if [[ -f "$file" ]]; then
    wc -l < "$file" | tr -d ' '
  else
    echo 0
  fi
}

trimmed_file_value() {
  local file="$1"
  if [[ -f "$file" ]]; then
    tr -d '\r' < "$file" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//' | head -n 1
  fi
}

append_summary_line() {
  printf '%s\n' "$1" >> "$SUMMARY_FILE"
}

write_summary() {
  local app_log="$BUNDLE_DIR/app/after/packet.log"
  local private_dns_mode
  private_dns_mode="$(trimmed_file_value "$BUNDLE_DIR/device/after/private_dns_mode.txt")"
  local auto_time
  auto_time="$(trimmed_file_value "$BUNDLE_DIR/device/after/auto_time.txt")"
  local auto_time_zone
  auto_time_zone="$(trimmed_file_value "$BUNDLE_DIR/device/after/auto_time_zone.txt")"

  {
    printf '# Phantom Android Diagnostics\n\n'
    printf '- Generated: %s\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    printf '- Device Serial: `%s`\n' "$SERIAL"
    printf '- Package: `%s`\n' "$PACKAGE"
    printf '- Bundle Directory: `%s`\n' "$BUNDLE_DIR"
    printf '- Logcat Lines: `%s`\n' "$(line_count "$LOGCAT_FILE")"
    printf '- App Log Lines: `%s`\n' "$(line_count "$app_log")"
    printf '- Private DNS Mode: `%s`\n' "${private_dns_mode:-unknown}"
    printf '- Auto Time: `%s`\n' "${auto_time:-unknown}"
    printf '- Auto Time Zone: `%s`\n' "${auto_time_zone:-unknown}"
    printf '\n## Heuristic Findings\n'
  } > "$SUMMARY_FILE"

  if [[ ! -f "$app_log" ]]; then
    append_summary_line '- App log was not captured. Check `run-as` access and ensure you used a debuggable build.'
    return
  fi

  if grep -q "✓ TUNNEL ACTIVE" "$app_log"; then
    append_summary_line '- Relay reached `TUNNEL ACTIVE`. Failures after that point are downstream traffic issues, not handshake setup.'
  else
    append_summary_line '- Relay never reached `TUNNEL ACTIVE`. The tunnel setup stalled before authenticated bidirectional forwarding became live.'
  fi

  if grep -q "✓ WebSocket connected" "$app_log"; then
    append_summary_line "- WebSocket upgrade succeeded at least once."
  else
    append_summary_line '- No successful `WebSocket connected` marker was captured. Focus on DPI, CDN edge behavior, Host/Origin handling, or stale client build.'
  fi

  if grep -q "WebSocket .* timed out" "$app_log"; then
    append_summary_line "- WebSocket handshake timed out. This is strong evidence of DPI blackholing or CDN upgrade traffic being silently dropped."
  fi

  if grep -q "WebSocket handshake to .* failed" "$app_log"; then
    append_summary_line "- WebSocket handshake failed with an explicit client error. Check Host override, Origin, CDN websocket forwarding, and public ingress path."
  fi

  if grep -q "Auth timeout" "$app_log"; then
    append_summary_line "- Auth timed out after the WebSocket stage. That points to the CDN or origin not delivering auth responses back to the client."
  fi

  if grep -q "Connection closed during auth" "$app_log"; then
    append_summary_line "- Connection closed during auth. That usually means CDN policy, websocket proxying, or origin session handling dropped the link before relay start."
  fi

  if grep -q "timestamp drift" "$app_log"; then
    append_summary_line "- Timestamp drift was detected. Device clock skew can cause auth rejection and must be fixed before deeper network debugging."
  fi

  if grep -q "No address associated with hostname" "$app_log"; then
    append_summary_line "- Hostname resolution failed. This usually means an invalid CDN edge entry or a server URL/override mismatch."
  fi

  if grep -q "unsupported SOCKS5 command 0x03" "$app_log"; then
    append_summary_line '- Android issued SOCKS5 `UDP ASSOCIATE` requests. That is expected noise from UDP / Private DNS traffic and is not proof that TCP relay is broken by itself.'
  fi

  if grep -q "connect timeout:" "$app_log"; then
    append_summary_line '- Downstream CONNECT timeouts were captured. If `TUNNEL ACTIVE` is missing, those timeouts are likely secondary effects of the tunnel never becoming live.'
  fi

  if [[ -n "${private_dns_mode:-}" && "$private_dns_mode" != "off" && "$private_dns_mode" != "null" ]]; then
    append_summary_line "- Android Private DNS is enabled (\`$private_dns_mode\`). That can trigger \`198.18.0.2:853\` traffic which this tunnel does not support end-to-end yet."
  fi

  cat >> "$SUMMARY_FILE" <<EOF

## Key Files
- `logcat.txt`: full adb logcat capture while reproducing the issue
- `app/after/packet.log`: in-app Phantom log with Rust + Android evidence
- `app/after/packet_preferences.xml`: saved config, runtime JSON, and diagnostics JSON
- `device/after/connectivity.txt`: Android connectivity dump
- `device/after/vpn.txt`: Android VPN dump
- `device/after/netstats.txt`: network stats snapshot
EOF
}

capture_host_text "host/date_utc.txt" date -u +"%Y-%m-%dT%H:%M:%SZ"
capture_host_text "host/uname.txt" uname -a
capture_host_text "host/adb_version.txt" adb version

capture_shell "host/device_path.txt" pm path "$PACKAGE"
capture_device_snapshot "before"
capture_app_snapshot "before"

echo "Diagnostics bundle: $BUNDLE_DIR"
echo "Device serial: $SERIAL"
echo "Package: $PACKAGE"
echo
echo "Starting adb logcat capture..."
adb_cmd logcat -c >/dev/null 2>&1 || true
adb_cmd logcat -v threadtime > "$LOGCAT_FILE" 2>&1 &
LOGCAT_PID=$!

cleanup() {
  if ps -p "$LOGCAT_PID" >/dev/null 2>&1; then
    kill "$LOGCAT_PID" >/dev/null 2>&1 || true
    wait "$LOGCAT_PID" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

echo "Reproduce the problem now on the device."
if [[ -n "$CAPTURE_SECONDS" ]]; then
  echo "Collecting for ${CAPTURE_SECONDS}s..."
  sleep "$CAPTURE_SECONDS"
else
  read -r -p "Press Enter when you have reproduced the issue..."
fi

cleanup
capture_device_snapshot "after"
capture_app_snapshot "after"
write_summary

(
  cd "$OUTPUT_BASE"
  zip -qry "$ZIP_PATH" "$BUNDLE_NAME"
)

echo
echo "Diagnostics summary:"
cat "$SUMMARY_FILE"
echo
echo "Bundle directory:"
echo "  $BUNDLE_DIR"
echo "Zip archive:"
echo "  $ZIP_PATH"
