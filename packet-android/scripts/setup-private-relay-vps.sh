#!/usr/bin/env bash
set -euo pipefail

# Run on the Iran VPS that private users connect to.
# This is only the front door. The real internet exit is the Starlink laptop
# running phantom-relay and connected outbound to this VPS.

PORT="${PORT:-80}"
SECRET="${PHANTOM_SECRET:-${SECRET:-$(tr -dc 'A-Za-z0-9' </dev/urandom | head -c 40)}}"
SERVICE_NAME="${SERVICE_NAME:-packet-private-vps}"
SERVER_BIN="/usr/local/bin/phantom-server"
REPO_DIR="${REPO_DIR:-$(pwd)}"

if [[ $EUID -ne 0 ]]; then
  echo "Run as root: sudo -i, then run this script from the packet repo."
  exit 1
fi

if [[ ! -f "$REPO_DIR/Cargo.toml" ]]; then
  echo "REPO_DIR must point to the packet repo. Current REPO_DIR=$REPO_DIR"
  exit 1
fi

apt-get update
apt-get install -y curl ca-certificates build-essential pkg-config ufw

if ! command -v cargo >/dev/null 2>&1; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

cd "$REPO_DIR"
cargo build -p phantom-server --release
install -m 0755 "$REPO_DIR/target/release/phantom-server" "$SERVER_BIN"

cat >/etc/systemd/system/${SERVICE_NAME}.service <<UNIT
[Unit]
Description=Packet Private Relay VPS front door
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=PHANTOM_SECRET=${SECRET}
ExecStart=${SERVER_BIN} --port ${PORT}
Restart=always
RestartSec=3
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now "${SERVICE_NAME}"

if command -v ufw >/dev/null 2>&1; then
  ufw allow "${PORT}/tcp" || true
fi

PUBLIC_IP="$(curl -fsS --max-time 8 https://api.ipify.org || hostname -I | awk '{print $1}')"

echo
echo "Private Relay VPS is running."
echo "Status:"
echo "  systemctl status ${SERVICE_NAME} --no-pager"
echo
echo "Android Private Relay profile:"
echo "  Server URL : http://${PUBLIC_IP}:${PORT}"
echo "  Secret     : ${SECRET}"
echo "  Stack      : Private Relay"
echo
echo "Starlink laptop relay command:"
echo "  PHANTOM_SERVER=http://${PUBLIC_IP}:${PORT} PHANTOM_SECRET='${SECRET}' phantom-relay --label starlink-laptop"
echo
echo "Health:"
echo "  curl -sS http://${PUBLIC_IP}:${PORT}/api/v1/health"
