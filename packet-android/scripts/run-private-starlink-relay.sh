#!/usr/bin/env bash
set -euo pipefail

# Run on the laptop that has BOTH:
#   1. an Iran Wi-Fi route to the Iran VPS, and
#   2. Starlink as the default route for public internet egress.
#
# The relay opens one outbound WebSocket to the Iran VPS. When clients connect
# to that VPS, their TCP streams are forwarded over this WebSocket to the laptop,
# and the laptop opens the real destination sockets through its OS routes.

SERVER="${PHANTOM_SERVER:-${SERVER:-}}"
SECRET="${PHANTOM_SECRET:-${SECRET:-}}"
LABEL="${LABEL:-starlink-laptop}"

STARLINK_IF="${STARLINK_IF:-}"
STARLINK_GW="${STARLINK_GW:-}"
IRAN_WIFI_IF="${IRAN_WIFI_IF:-}"
IRAN_WIFI_GW="${IRAN_WIFI_GW:-}"
VPS_IP="${VPS_IP:-}"

if [[ -z "$SERVER" || -z "$SECRET" ]]; then
  echo "Usage:"
  echo "  PHANTOM_SERVER=http://IRAN_VPS:80 PHANTOM_SECRET=... $0"
  echo
  echo "Optional Linux route pinning:"
  echo "  VPS_IP=1.2.3.4 IRAN_WIFI_IF=wlan0 IRAN_WIFI_GW=192.168.1.1 STARLINK_IF=wlan1 STARLINK_GW=192.168.100.1 $0"
  exit 1
fi

if [[ -n "$VPS_IP" && -n "$IRAN_WIFI_IF" && -n "$IRAN_WIFI_GW" ]]; then
  echo "Pinning VPS route through Iran Wi-Fi: ${VPS_IP}/32 via ${IRAN_WIFI_GW} dev ${IRAN_WIFI_IF}"
  sudo ip route replace "${VPS_IP}/32" via "${IRAN_WIFI_GW}" dev "${IRAN_WIFI_IF}"
fi

if [[ -n "$STARLINK_IF" && -n "$STARLINK_GW" ]]; then
  echo "Setting Starlink as preferred default route: default via ${STARLINK_GW} dev ${STARLINK_IF}"
  sudo ip route replace default via "${STARLINK_GW}" dev "${STARLINK_IF}" metric 50
fi

cargo build -p phantom-relay --release

echo
echo "Starting private Starlink relay:"
echo "  server=${SERVER}"
echo "  label=${LABEL}"
echo
exec target/release/phantom-relay \
  --server "${SERVER}" \
  --secret "${SECRET}" \
  --label "${LABEL}"
