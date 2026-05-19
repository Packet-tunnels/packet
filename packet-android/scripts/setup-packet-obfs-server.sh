#!/usr/bin/env bash
set -euo pipefail

# Run this on the VPS that will host Packet Native + obfs.
# It keeps the normal Packet HTTP API on HTTP_PORT and adds a raw-TCP obfs
# listener on OBFS_PORT. The obfs port must be reached directly by IP:port;
# do not put it behind Cloudflare or any TLS-terminating CDN.

SERVER_DIR="${SERVER_DIR:-$(pwd)}"
HTTP_PORT="${HTTP_PORT:-80}"
OBFS_PORT="${OBFS_PORT:-36571}"
PHANTOM_SECRET="${PHANTOM_SECRET:-$(openssl rand -hex 32)}"
PHANTOM_OBFS_KEY="${PHANTOM_OBFS_KEY:-$(openssl rand -hex 16)}"
SERVICE_NAME="${SERVICE_NAME:-packet-obfs}"
SERVER_BIN="/usr/local/bin/phantom-server"

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run as root from the packet repo on the VPS:"
  echo "  sudo SERVER_DIR=$(pwd) bash $0"
  exit 1
fi

if [[ ! -d "$SERVER_DIR" ]]; then
  echo "SERVER_DIR does not exist: $SERVER_DIR" >&2
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  apt-get update
  apt-get install -y curl ca-certificates openssl build-essential pkg-config libssl-dev
fi

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

cd "$SERVER_DIR"
cargo build -p phantom-server --release
install -m 0755 "$SERVER_DIR/target/release/phantom-server" "$SERVER_BIN"

cat >"/etc/systemd/system/${SERVICE_NAME}.service" <<SERVICE
[Unit]
Description=Packet Native server with raw obfs listener
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=PHANTOM_SECRET=${PHANTOM_SECRET}
Environment=PHANTOM_OBFS_KEY=${PHANTOM_OBFS_KEY}
ExecStart=${SERVER_BIN} --port ${HTTP_PORT} --obfs-port ${OBFS_PORT}
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
SERVICE

systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME"

if command -v ufw >/dev/null 2>&1; then
  ufw allow "${HTTP_PORT}/tcp" || true
  ufw allow "${OBFS_PORT}/tcp" || true
fi

PUBLIC_IP="$(curl -fsS https://api.ipify.org || hostname -I | awk '{print $1}')"

cat <<OUT

Packet obfs server is installed.

Service:
  systemctl status ${SERVICE_NAME} --no-pager
  journalctl -u ${SERVICE_NAME} -f

Mobile Packet Native config:
  Server URL : http://${PUBLIC_IP}:${HTTP_PORT}
  Secret     : ${PHANTOM_SECRET}
  Transport  : Obfs
  CDN Edge   : ${PUBLIC_IP}:${OBFS_PORT}
  Obfs Key   : ${PHANTOM_OBFS_KEY}

CLI test:
  ./target/release/phantom-client --server http://${PUBLIC_IP}:${HTTP_PORT} --secret '${PHANTOM_SECRET}' --transport obfs --cdn-edge ${PUBLIC_IP}:${OBFS_PORT} --obfs-key '${PHANTOM_OBFS_KEY}'

Important:
  The CDN Edge value is direct IP:port for obfs. Do not use Cloudflare here.
  If Iran reach_probe shows TIMEOUT for ${PUBLIC_IP}:${OBFS_PORT}, this VPS/IP
  is blackholed and obfs needs a different reachable VPS/provider.
OUT
