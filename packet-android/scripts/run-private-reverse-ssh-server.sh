#!/usr/bin/env bash
set -euo pipefail

# Run this on the private laptop that has:
#   - Starlink as the default route for real internet egress.
#   - Iran Wi-Fi / hotspot / cable as the route to the Iran VPS.
#
# The laptop runs phantom-server locally. SSH exposes that local server on the
# Iran VPS public IP with a remote port forward, so the VPS does not need Rust,
# git, Packet code, or the Packet shared secret.

VPS_HOST="${VPS_HOST:-}"
VPS_USER="${VPS_USER:-root}"
VPS_SSH_PORT="${VPS_SSH_PORT:-22}"
REMOTE_PORT="${REMOTE_PORT:-80}"
REMOTE_BIND="${REMOTE_BIND:-0.0.0.0}"
LOCAL_HOST="${LOCAL_HOST:-127.0.0.1}"
LOCAL_PORT="${LOCAL_PORT:-8080}"
SECRET="${PHANTOM_SECRET:-${SECRET:-}}"

STARLINK_IF="${STARLINK_IF:-}"
STARLINK_GW="${STARLINK_GW:-}"
IRAN_WIFI_IF="${IRAN_WIFI_IF:-}"
IRAN_WIFI_GW="${IRAN_WIFI_GW:-}"

if [[ -z "$VPS_HOST" ]]; then
  echo "Usage:"
  echo "  VPS_HOST=<IRAN_VPS_PUBLIC_IP> $0"
  echo
  echo "Optional:"
  echo "  VPS_USER=root VPS_SSH_PORT=22 REMOTE_PORT=80 LOCAL_PORT=8080 PHANTOM_SECRET=... $0"
  echo
  echo "Optional Linux route pinning:"
  echo "  VPS_HOST=1.2.3.4 IRAN_WIFI_IF=wlan0 IRAN_WIFI_GW=192.168.1.1 STARLINK_IF=wlan1 STARLINK_GW=192.168.100.1 $0"
  exit 1
fi

if [[ -z "$SECRET" ]]; then
  if command -v openssl >/dev/null 2>&1; then
    SECRET="$(openssl rand -hex 32)"
  else
    SECRET="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 48)"
  fi
fi

if [[ -n "$IRAN_WIFI_IF" && -n "$IRAN_WIFI_GW" ]]; then
  echo "Pinning VPS route through Iran Wi-Fi: ${VPS_HOST}/32 via ${IRAN_WIFI_GW} dev ${IRAN_WIFI_IF}"
  sudo ip route replace "${VPS_HOST}/32" via "${IRAN_WIFI_GW}" dev "${IRAN_WIFI_IF}"
fi

if [[ -n "$STARLINK_IF" && -n "$STARLINK_GW" ]]; then
  echo "Setting Starlink as preferred default route: default via ${STARLINK_GW} dev ${STARLINK_IF}"
  sudo ip route replace default via "${STARLINK_GW}" dev "${STARLINK_IF}" metric 50
fi

cargo build -p phantom-server --release

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

echo
echo "Starting local Packet server on ${LOCAL_HOST}:${LOCAL_PORT}"
PHANTOM_SECRET="$SECRET" target/release/phantom-server --port "$LOCAL_PORT" &
server_pid="$!"

sleep 1
if ! kill -0 "$server_pid" >/dev/null 2>&1; then
  echo "phantom-server exited before SSH tunnel started." >&2
  wait "$server_pid" || true
  exit 1
fi

echo
echo "Packet private reverse-SSH mode"
echo "VPS public listen : ${REMOTE_BIND}:${REMOTE_PORT}"
echo "Laptop local      : ${LOCAL_HOST}:${LOCAL_PORT}"
echo "Secret            : ${SECRET}"
echo
echo "Android app:"
echo "  Stack      : Private Relay"
echo "  Server URL : http://${VPS_HOST}:${REMOTE_PORT}"
echo "  Secret     : ${SECRET}"
echo
echo "VPS requirement:"
echo "  sshd must allow remote forwarding and public bind:"
echo "    AllowTcpForwarding yes"
echo "    GatewayPorts clientspecified"
echo
echo "Keep this terminal open. Ctrl+C stops the SSH tunnel and local Packet server."
echo

exec ssh \
  -p "$VPS_SSH_PORT" \
  -N \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=20 \
  -o ServerAliveCountMax=3 \
  -R "${REMOTE_BIND}:${REMOTE_PORT}:${LOCAL_HOST}:${LOCAL_PORT}" \
  "${VPS_USER}@${VPS_HOST}"
