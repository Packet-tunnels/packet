#!/usr/bin/env bash
set -euo pipefail

# Linux laptop check script.
# Reports Wi-Fi priorities, active routes, local Packet server health, and VPS
# public reachability for the private reverse-SSH setup.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SECRET_FILE="${SECRET_FILE:-${REPO_ROOT}/.packet-private.env}"

VPS_HOST="${VPS_HOST:-PUT_IRAN_VPS_IP_HERE}"
REMOTE_PORT="${REMOTE_PORT:-80}"
LOCAL_PORT="${LOCAL_PORT:-80}"
STARLINK_WIFI_NAME="${STARLINK_WIFI_NAME:-yagoob}"

if [[ -f "$SECRET_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$SECRET_FILE"
fi

echo "Packet private route check"
echo "Repo        : ${REPO_ROOT}"
echo "Secret file : ${SECRET_FILE}"
echo

echo "[1] NetworkManager Wi-Fi connections"
if command -v nmcli >/dev/null 2>&1; then
  nmcli -f NAME,TYPE,AUTOCONNECT,AUTOCONNECT-PRIORITY con show | sed -n '1,12p'
  echo
  echo "[2] Active devices"
  nmcli -f DEVICE,TYPE,STATE,CONNECTION dev status
else
  echo "nmcli not found. Install NetworkManager tools or check routes manually."
fi

echo
echo "[3] Routes"
ip -4 route show default || true
if [[ "$VPS_HOST" != "PUT_IRAN_VPS_IP_HERE" && -n "$VPS_HOST" ]]; then
  ip -4 route get "$VPS_HOST" || true
fi

echo
echo "[4] Local Packet server"
if command -v curl >/dev/null 2>&1; then
  curl -fsS --max-time 3 "http://127.0.0.1:${LOCAL_PORT}/api/v1/health" || echo "local Packet health failed"
else
  echo "curl not found"
fi

echo
echo "[5] VPS public port"
if [[ "$VPS_HOST" == "PUT_IRAN_VPS_IP_HERE" || -z "$VPS_HOST" ]]; then
  echo "Set VPS_HOST first to test public route:"
  echo "  VPS_HOST=<IRAN_VPS_PUBLIC_IP> ${BASH_SOURCE[0]}"
elif command -v curl >/dev/null 2>&1; then
  curl -fsS --max-time 5 "http://${VPS_HOST}:${REMOTE_PORT}/api/v1/health" || echo "VPS public health failed"
fi

echo
echo "[6] Expected"
echo "Starlink/yagoob should be the low-metric default route."
echo "The route to VPS_HOST should use the non-yagoob Iran network when both networks are up."
