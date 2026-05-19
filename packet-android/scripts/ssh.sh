#!/usr/bin/env bash
set -euo pipefail

# Linux laptop script.
# Opens a reverse SSH tunnel from the Iran VPS public IP to the local Packet
# server started by runn.sh. Edit the VPS_* placeholders below or export them.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SECRET_FILE="${SECRET_FILE:-${REPO_ROOT}/.packet-private.env}"

# EDIT THESE:
VPS_HOST="${VPS_HOST:-PUT_IRAN_VPS_IP_HERE}"
VPS_USER="${VPS_USER:-root}"
VPS_SSH_PORT="${VPS_SSH_PORT:-22}"
VPS_PASSWORD="${VPS_PASSWORD:-}" # optional; if empty, ssh key / manual password prompt is used

REMOTE_BIND="${REMOTE_BIND:-0.0.0.0}"
REMOTE_PORT="${REMOTE_PORT:-80}"
LOCAL_HOST="${LOCAL_HOST:-127.0.0.1}"
LOCAL_PORT="${LOCAL_PORT:-80}"

STARLINK_WIFI_NAME="${STARLINK_WIFI_NAME:-yagoob}"
STARLINK_PRIORITY="${STARLINK_PRIORITY:-10}"

if [[ -f "$SECRET_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$SECRET_FILE"
fi

if [[ "$VPS_HOST" == "PUT_IRAN_VPS_IP_HERE" || -z "$VPS_HOST" ]]; then
  echo "Edit VPS_HOST inside ${BASH_SOURCE[0]} or run:"
  echo "  VPS_HOST=<IRAN_VPS_PUBLIC_IP> ${BASH_SOURCE[0]}"
  exit 1
fi

if [[ -z "${PHANTOM_SECRET:-}" ]]; then
  echo "No PHANTOM_SECRET found. Start runn.sh first, or set PHANTOM_SECRET." >&2
  exit 1
fi

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing command: $1" >&2
    exit 1
  fi
}

active_wifi_name() {
  nmcli -t -f active,ssid dev wifi 2>/dev/null | awk -F: '$1=="yes"{print $2; exit}'
}

wifi_device_for_ssid() {
  local ssid="$1"
  nmcli -t -f device,type,state,connection dev status 2>/dev/null \
    | awk -F: -v ssid="$ssid" '$2=="wifi" && $3=="connected" && $4==ssid {print $1; exit}'
}

connection_exists() {
  local name="$1"
  nmcli -t -f NAME con show 2>/dev/null | awk -F: -v name="$name" '$1==name {found=1} END {exit found ? 0 : 1}'
}

gateway_for_device() {
  local dev="$1"
  ip -4 route show default dev "$dev" 2>/dev/null | awk '/default/ {print $3; exit}'
}

set_wifi_priority() {
  local ssid="$1"
  local priority="$2"
  local con
  con="$(nmcli -t -f NAME,TYPE con show 2>/dev/null | awk -F: -v ssid="$ssid" '$2=="802-11-wireless" && $1==ssid {print $1; exit}')"
  if [[ -n "$con" ]]; then
    nmcli con modify "$con" connection.autoconnect yes connection.autoconnect-priority "$priority" >/dev/null
  fi
}

require_cmd ssh
require_cmd ip
require_cmd nmcli

set_wifi_priority "$STARLINK_WIFI_NAME" "$STARLINK_PRIORITY" || true

if [[ -z "$(wifi_device_for_ssid "$STARLINK_WIFI_NAME" || true)" ]]; then
  if connection_exists "$STARLINK_WIFI_NAME"; then
    echo "Connecting Starlink Wi-Fi: ${STARLINK_WIFI_NAME}"
    nmcli con up "$STARLINK_WIFI_NAME" >/dev/null || true
    sleep 2
  else
    echo "Starlink Wi-Fi profile '${STARLINK_WIFI_NAME}' was not found in NetworkManager." >&2
  fi
fi

STARLINK_IF="$(wifi_device_for_ssid "$STARLINK_WIFI_NAME" || true)"
STARLINK_GW=""
if [[ -n "$STARLINK_IF" ]]; then
  STARLINK_GW="$(gateway_for_device "$STARLINK_IF" || true)"
fi

IRAN_WIFI_IF=""
IRAN_WIFI_GW=""
while IFS=: read -r dev type state con; do
  [[ "$state" == "connected" ]] || continue
  [[ "$con" != "$STARLINK_WIFI_NAME" ]] || continue
  IRAN_WIFI_IF="$dev"
  IRAN_WIFI_GW="$(gateway_for_device "$dev" || true)"
  break
done < <(nmcli -t -f device,type,state,connection dev status 2>/dev/null)

if [[ -n "$IRAN_WIFI_IF" && -n "$IRAN_WIFI_GW" ]]; then
  echo "Pinning VPS route through Iran network: ${VPS_HOST}/32 via ${IRAN_WIFI_GW} dev ${IRAN_WIFI_IF}"
  sudo ip route replace "${VPS_HOST}/32" via "$IRAN_WIFI_GW" dev "$IRAN_WIFI_IF"
fi

if [[ -n "$STARLINK_IF" && -n "$STARLINK_GW" ]]; then
  echo "Setting Starlink/yagoob default route priority: default via ${STARLINK_GW} dev ${STARLINK_IF} metric ${STARLINK_PRIORITY}"
  sudo ip route replace default via "$STARLINK_GW" dev "$STARLINK_IF" metric "$STARLINK_PRIORITY"
fi

echo
echo "Reverse SSH starting"
echo "VPS public listen : ${REMOTE_BIND}:${REMOTE_PORT}"
echo "Laptop local      : ${LOCAL_HOST}:${LOCAL_PORT}"
echo "Starlink Wi-Fi    : ${STARLINK_WIFI_NAME} dev=${STARLINK_IF:-?} gw=${STARLINK_GW:-?}"
echo "Iran network      : dev=${IRAN_WIFI_IF:-?} gw=${IRAN_WIFI_GW:-?}"
echo
echo "Android app:"
echo "  Stack      : Private Relay"
echo "  Server URL : http://${VPS_HOST}:${REMOTE_PORT}"
echo "  Secret     : ${PHANTOM_SECRET}"
echo
echo "Keep this terminal open. Ctrl+C stops the reverse tunnel."
echo

SSH_ARGS=(
  -p "$VPS_SSH_PORT"
  -N
  -o ExitOnForwardFailure=yes
  -o ServerAliveInterval=20
  -o ServerAliveCountMax=3
  -R "${REMOTE_BIND}:${REMOTE_PORT}:${LOCAL_HOST}:${LOCAL_PORT}"
  "${VPS_USER}@${VPS_HOST}"
)

if [[ -n "$VPS_PASSWORD" ]]; then
  require_cmd sshpass
  exec sshpass -p "$VPS_PASSWORD" ssh "${SSH_ARGS[@]}"
else
  exec ssh "${SSH_ARGS[@]}"
fi
