#!/bin/bash
# deploy.sh — Deploy Phantom Tunnel server to VPS
# Usage: ./deploy.sh <VPS_IP> <SECRET_KEY>
set -e

VPS_IP="${1:?Usage: ./deploy.sh <VPS_IP> <SECRET_KEY>}"
SECRET="${2:?Usage: ./deploy.sh <VPS_IP> <SECRET_KEY>}"
SSH_KEY="$HOME/.ssh/google_compute_engine"
SSH_USER="mohammadshayani"
SSH_CMD="ssh -i $SSH_KEY -o StrictHostKeyChecking=no -o ConnectTimeout=15 $SSH_USER@$VPS_IP"

echo "=== Phantom Tunnel Deployment ==="
echo "Target: $SSH_USER@$VPS_IP"

# Step 1: Install Rust if not present
echo "[1/5] Checking Rust installation..."
$SSH_CMD "source \$HOME/.cargo/env 2>/dev/null; which cargo || (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal)"

# Step 2: Install build deps
echo "[2/5] Installing build dependencies..."
$SSH_CMD "sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev git"

# Step 3: Clone or pull repo
echo "[3/5] Cloning/updating repository..."
$SSH_CMD "if [ -d phantom-tunnel ]; then cd phantom-tunnel && git pull; else git clone https://github.com/$SSH_USER/phantom-tunnel.git; fi"

# Step 4: Build server (single thread to avoid OOM on 1GB RAM)
echo "[4/5] Building server (this may take a few minutes on first build)..."
$SSH_CMD "source \$HOME/.cargo/env && cd phantom-tunnel && CARGO_BUILD_JOBS=1 cargo build --release -p phantom-server 2>&1 | tail -5"

# Step 5: Install and create systemd service
echo "[5/5] Installing service..."
$SSH_CMD "sudo cp phantom-tunnel/target/release/phantom-server /usr/local/bin/phantom-server && sudo chmod +x /usr/local/bin/phantom-server"

$SSH_CMD "sudo tee /etc/systemd/system/phantom.service > /dev/null << 'UNIT'
[Unit]
Description=Phantom Tunnel Server
After=network.target

[Service]
Type=simple
User=root
Environment=PHANTOM_SECRET=$SECRET
ExecStart=/usr/local/bin/phantom-server --port 80 --secret \${PHANTOM_SECRET}
Restart=always
RestartSec=3
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT"

$SSH_CMD "sudo systemctl daemon-reload && sudo systemctl enable phantom && sudo systemctl restart phantom && sleep 2 && sudo systemctl status phantom --no-pager"

echo ""
echo "=== Deployment Complete ==="
echo "Server running on http://$VPS_IP:80"
echo ""
echo "Test: curl http://$VPS_IP/"
echo ""
echo "Client usage:"
echo "  ./phantom-client --server http://$VPS_IP --secret '$SECRET' --listen 127.0.0.1:1080"
echo ""
echo "Once DNS is fixed for piano-lessons.site (via ArvanCloud):"
echo "  ./phantom-client --server https://piano-lessons.site --secret '$SECRET' --listen 127.0.0.1:1080"
