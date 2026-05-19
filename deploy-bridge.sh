#!/bin/bash
# deploy-bridge.sh — Deploy Packet bridge to a domestic (Iran) VPS
# Usage: ./deploy-bridge.sh <BRIDGE_VPS_IP> <UPSTREAM_SERVER_IP> [SSH_USER] [SSH_KEY]
set -e

BRIDGE_IP="${1:?Usage: ./deploy-bridge.sh <BRIDGE_VPS_IP> <UPSTREAM_SERVER_IP> [SSH_USER] [SSH_KEY]}"
UPSTREAM="${2:?Usage: ./deploy-bridge.sh <BRIDGE_VPS_IP> <UPSTREAM_SERVER_IP> [SSH_USER] [SSH_KEY]}"
SSH_USER="${3:-root}"
SSH_KEY="${4:-$HOME/.ssh/google_compute_engine}"
SSH_CMD="ssh -i $SSH_KEY -o StrictHostKeyChecking=no -o ConnectTimeout=15 $SSH_USER@$BRIDGE_IP"

echo "=== Packet Bridge Deployment ==="
echo "Bridge VPS: $SSH_USER@$BRIDGE_IP"
echo "Upstream:   $UPSTREAM"

echo "[1/5] Checking Rust installation..."
$SSH_CMD "source \$HOME/.cargo/env 2>/dev/null; which cargo || (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal)"

echo "[2/5] Installing build dependencies..."
$SSH_CMD "sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev git"

echo "[3/5] Cloning/updating repository..."
$SSH_CMD "if [ -d packet ]; then cd packet && git pull; else git clone https://github.com/$SSH_USER/phantom-tunnel.git packet; fi"

echo "[4/5] Building bridge (CARGO_BUILD_JOBS=1 for low-RAM VPS)..."
$SSH_CMD "source \$HOME/.cargo/env && cd packet && CARGO_BUILD_JOBS=1 cargo build --release -p phantom-bridge 2>&1 | tail -5"

echo "[5/5] Installing bridge service..."
$SSH_CMD "sudo cp packet/target/release/phantom-bridge /usr/local/bin/phantom-bridge && sudo chmod +x /usr/local/bin/phantom-bridge"

$SSH_CMD "sudo tee /etc/systemd/system/phantom-bridge.service > /dev/null << 'UNIT'
[Unit]
Description=Packet Bridge (domestic relay)
After=network.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/phantom-bridge --listen 0.0.0.0:80 --upstream $UPSTREAM:80
Restart=always
RestartSec=3
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT"

$SSH_CMD "sudo systemctl daemon-reload && sudo systemctl enable phantom-bridge && sudo systemctl restart phantom-bridge && sleep 2 && sudo systemctl status phantom-bridge --no-pager"

echo ""
echo "=== Bridge Deployment Complete ==="
echo "Bridge listening on http://$BRIDGE_IP:80"
echo "Forwarding to $UPSTREAM:80"
echo ""
echo "Test from outside: curl -v http://$BRIDGE_IP/"
echo "(Should show the piano-lessons.site decoy page from the upstream)"
echo ""
echo "Client config for Iran user:"
echo "  Server URL: http://$BRIDGE_IP"
echo "  (No CDN edge, no host override, no SNI override needed)"
