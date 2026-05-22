#!/usr/bin/env bash
set -euo pipefail

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT_DIR="$(cd "$ANDROID_DIR/.." && pwd)"
LAB_DIR="${PSIPHON_LAB_DIR:-$ANDROID_DIR/.psiphon-core-lab}"
BIN_REPO="$LAB_DIR/psiphon-tunnel-core-binaries"
BIN_REPO_URL="${PSIPHON_BIN_REPO_URL:-https://github.com/Psiphon-Labs/psiphon-tunnel-core-binaries.git}"
SERVER_DIR="${PSIPHON_SERVER_DIR:-$LAB_DIR/server}"
CLIENT_DIR="${PSIPHON_CLIENT_DIR:-$LAB_DIR/client}"
ASSET_DIR="$ANDROID_DIR/app/src/main/assets/psiphon"
IOS_ASSET_DIR="$ROOT_DIR/packet-ios/PacketTunnel/Resources/psiphon"

PSIPHOND="$BIN_REPO/psiphond/psiphond"
CONSOLE_CLIENT="$BIN_REPO/linux/psiphon-tunnel-core-x86_64"
PUBLIC_IP="${PUBLIC_IP:-}"
PROTOCOL="${PROTOCOL:-OSSH}"
PORT="${PORT:-9999}"
LOCAL_HTTP_PROXY_PORT="${LOCAL_HTTP_PROXY_PORT:-18080}"
LOCAL_SOCKS_PROXY_PORT="${LOCAL_SOCKS_PROXY_PORT:-18081}"
UPSTREAM_PROXY_URL="${UPSTREAM_PROXY_URL:-}"

usage() {
  cat <<EOF
Usage:
  $0 fetch
  PUBLIC_IP=<server-ip> PORT=9999 $0 generate-server
  $0 run-server
  $0 run-client
  $0 install-client-asset
  $0 print

This is a Psiphon-core lab, isolated from normal Packet code.
It uses your own generated psiphond server entry, not Psiphon's public network.
EOF
}

ensure_bins() {
  if [[ ! -x "$PSIPHOND" || ! -x "$CONSOLE_CLIENT" ]]; then
    echo "Missing Psiphon binaries. Run: $0 fetch" >&2
    exit 1
  fi
}

ensure_linux_x86_64() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  if [[ "$os" != "Linux" || "$arch" != "x86_64" ]]; then
    cat >&2 <<EOF
This Psiphon lab binary set is Linux x86_64 only.
Run this command on the VPS/server, not on this host.
  detected: ${os}/${arch}
EOF
    exit 1
  fi
}

fetch_bins() {
  mkdir -p "$LAB_DIR"
  if [[ ! -d "$BIN_REPO/.git" ]]; then
    git clone --depth 1 "$BIN_REPO_URL" "$BIN_REPO"
  else
    git -C "$BIN_REPO" fetch --depth 1 origin
    git -C "$BIN_REPO" reset --hard origin/master
  fi
  chmod +x "$PSIPHOND" "$CONSOLE_CLIENT"
  echo "Fetched Psiphon binaries into $BIN_REPO"
}

detect_public_ip() {
  if [[ -n "$PUBLIC_IP" ]]; then
    printf '%s' "$PUBLIC_IP"
    return
  fi
  curl -fsS --max-time 8 https://api.ipify.org
}

generate_server() {
  ensure_bins
  ensure_linux_x86_64
  mkdir -p "$SERVER_DIR" "$CLIENT_DIR"
  local ip
  ip="$(detect_public_ip)"
  (
    cd "$SERVER_DIR"
    "$PSIPHOND" -ipaddress "$ip" -protocol "${PROTOCOL}:${PORT}" generate
  )

  local server_entry
  server_entry="$(cat "$SERVER_DIR/server-entry.dat")"
  local upstream_line=""
  if [[ -n "$UPSTREAM_PROXY_URL" ]]; then
    upstream_line="  \"UpstreamProxyURL\": \"$UPSTREAM_PROXY_URL\","
  fi
  cat > "$CLIENT_DIR/client.config" <<JSON
{
  "LocalHttpProxyPort": $LOCAL_HTTP_PROXY_PORT,
  "LocalSocksProxyPort": $LOCAL_SOCKS_PROXY_PORT,
${upstream_line}
  "PropagationChannelId": "24BCA4EE20BEB92C",
  "SponsorId": "721AE60D76700F5A",
  "TargetServerEntry": "$server_entry"
}
JSON

  cat <<EOF
Generated Psiphon server/client config.
  server_dir=$SERVER_DIR
  client_config=$CLIENT_DIR/client.config
  protocol=${PROTOCOL}:${PORT}
  public_ip=$ip
  upstream_proxy_url=${UPSTREAM_PROXY_URL:-"(set dynamically by Android Psiphon Chain)"}

Open firewall:
  sudo ufw allow ${PORT}/tcp

Run server:
  $0 run-server

Run client:
  $0 run-client
EOF
}

run_server() {
  ensure_bins
  ensure_linux_x86_64
  if [[ ! -f "$SERVER_DIR/psiphond.config" ]]; then
    echo "Missing $SERVER_DIR/psiphond.config. Run generate-server first." >&2
    exit 1
  fi
  cd "$SERVER_DIR"
  exec "$PSIPHOND" run
}

run_client() {
  ensure_bins
  ensure_linux_x86_64
  if [[ ! -f "$CLIENT_DIR/client.config" ]]; then
    echo "Missing $CLIENT_DIR/client.config. Run generate-server first." >&2
    exit 1
  fi
  exec "$CONSOLE_CLIENT" -formatNotices -config "$CLIENT_DIR/client.config"
}

install_client_asset() {
  if [[ ! -f "$CLIENT_DIR/client.config" ]]; then
    echo "Missing $CLIENT_DIR/client.config. Run generate-server first." >&2
    exit 1
  fi
  mkdir -p "$ASSET_DIR"
  cp "$CLIENT_DIR/client.config" "$ASSET_DIR/client.config"
  echo "Installed embedded Psiphon client config: $ASSET_DIR/client.config"
  mkdir -p "$IOS_ASSET_DIR"
  cp "$CLIENT_DIR/client.config" "$IOS_ASSET_DIR/client.config"
  echo "Installed embedded Psiphon iOS client config: $IOS_ASSET_DIR/client.config"
}

print_state() {
  cat <<EOF
Psiphon core lab:
  lab_dir=$LAB_DIR
  psiphond=$PSIPHOND
  console_client=$CONSOLE_CLIENT
  server_dir=$SERVER_DIR
  client_config=$CLIENT_DIR/client.config
  android_asset=$ASSET_DIR/client.config
  ios_asset=$IOS_ASSET_DIR/client.config
  protocol=${PROTOCOL}:${PORT}
  local_http_proxy_port=$LOCAL_HTTP_PROXY_PORT
  local_socks_proxy_port=$LOCAL_SOCKS_PROXY_PORT
EOF
}

cmd="${1:-}"
case "$cmd" in
  fetch) fetch_bins ;;
  generate-server) generate_server ;;
  run-server) run_server ;;
  run-client) run_client ;;
  install-client-asset) install_client_asset ;;
  print) print_state ;;
  ""|-h|--help|help) usage ;;
  *) usage >&2; exit 1 ;;
esac
