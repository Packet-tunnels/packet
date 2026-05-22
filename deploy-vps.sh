#!/usr/bin/env bash
# deploy-vps.sh — Build phantom-server and keep it alive via systemd.
#
# Run this ON the VPS, from inside the packet repo:
#     cd /root/packet  (or wherever you cloned/copied this repo)
#     bash deploy-vps.sh
#
# What it does:
#   1. Generates a fresh PHANTOM_SECRET and OBFS_KEY (32 bytes hex each).
#   2. Installs Rust + build deps if missing.
#   3. Builds packet-server in release mode.
#   4. Installs a systemd unit (`phantom-server.service`) that restarts on
#      failure → that's the "alive mode" you asked for; no nginx needed.
#   5. Starts the service on port 80 (HTTP/WS/Meek, dual-stack v4/v6),
#      8443 (OBFS TCP), and 443 (QUIC UDP).
#   6. Starts a private Psiphon OSSH server on port 9999 for Psiphon Escape.
#   7. Prints the secrets and the snippet you need to paste into
#      packet-android/.../TunnelModels.kt on your laptop.
#
# Safe to re-run: secrets are regenerated only if /root/packet-secrets.env
# is missing.

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "This script must be run as root (sudo bash deploy-vps.sh)." >&2
  exit 1
fi

REPO_DIR="${REPO_DIR:-$(pwd)}"
WS_PORT="${WS_PORT:-80}"
OBFS_PORT="${OBFS_PORT:-8443}"
QUIC_PORT="${QUIC_PORT:-443}"
PSIPHON_PORT="${PSIPHON_PORT:-9999}"
SECRETS_FILE="/root/packet-secrets.env"

if [[ ! -f "$REPO_DIR/Cargo.toml" ]] || [[ ! -d "$REPO_DIR/packet-server" ]]; then
  echo "ERROR: $REPO_DIR doesn't look like the packet repo (Cargo.toml + packet-server expected)." >&2
  echo "       cd into the repo first, or set REPO_DIR=/path/to/packet." >&2
  exit 1
fi

echo "═══════════════════════════════════════════════════════════"
echo "  phantom-server VPS setup"
echo "  Repo  : $REPO_DIR"
echo "  Ports : WS/Meek=$WS_PORT  OBFS-TCP=$OBFS_PORT  QUIC-UDP=$QUIC_PORT  Psiphon-OSSH=$PSIPHON_PORT"
echo "═══════════════════════════════════════════════════════════"

# ── 1. Secrets (preserve if already deployed) ─────────────────────
gen_hex32() { openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64; }
if [[ -f "$SECRETS_FILE" ]]; then
  echo "[1/5] Reusing existing secrets at $SECRETS_FILE"
  # shellcheck disable=SC1090
  source "$SECRETS_FILE"
else
  echo "[1/5] Generating fresh PHANTOM_SECRET and OBFS_KEY …"
  PHANTOM_SECRET="$(gen_hex32)"
  OBFS_KEY="$(gen_hex32)"
  umask 077
  cat > "$SECRETS_FILE" <<EOF
# Generated $(date -u +"%Y-%m-%dT%H:%M:%SZ")
PHANTOM_SECRET=$PHANTOM_SECRET
OBFS_KEY=$OBFS_KEY
WS_PORT=$WS_PORT
OBFS_PORT=$OBFS_PORT
QUIC_PORT=$QUIC_PORT
PSIPHON_PORT=$PSIPHON_PORT
EOF
  echo "       → saved to $SECRETS_FILE (mode 600)"
fi

# Backfill new vars when reusing an older secrets file.
if ! grep -q '^QUIC_PORT=' "$SECRETS_FILE"; then
  echo "QUIC_PORT=$QUIC_PORT" >> "$SECRETS_FILE"
fi
if ! grep -q '^PSIPHON_PORT=' "$SECRETS_FILE"; then
  echo "PSIPHON_PORT=$PSIPHON_PORT" >> "$SECRETS_FILE"
fi

# ── 2. Build deps + Rust ──────────────────────────────────────────
echo "[2/5] Installing build deps + Rust if missing …"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev curl ca-certificates git openssl >/dev/null
if ! command -v cargo >/dev/null 2>&1 && [[ ! -x /root/.cargo/bin/cargo ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
fi
# shellcheck disable=SC1091
source /root/.cargo/env 2>/dev/null || true
echo "       rustc: $(rustc --version)"

# ── 3. Build phantom-server (release) ─────────────────────────────
echo "[3/5] Building phantom-server (release) — first build can take ~3 min …"
cd "$REPO_DIR"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build --release -p phantom-server 2>&1 \
  | tail -n 8
install -m 0755 target/release/phantom-server /usr/local/bin/phantom-server
echo "       installed to /usr/local/bin/phantom-server"

# ── 4. systemd unit (alive mode) ──────────────────────────────────
echo "[4/5] Installing systemd unit (Restart=always) …"
cat > /etc/systemd/system/phantom-server.service <<UNIT
[Unit]
Description=Packet phantom-server (dual-stack v4/v6, WS + Meek + OBFS + QUIC)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
EnvironmentFile=$SECRETS_FILE
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/phantom-server --port \${WS_PORT} --secret \${PHANTOM_SECRET} --obfs-port \${OBFS_PORT} --obfs-key \${OBFS_KEY} --quic-port \${QUIC_PORT}
Restart=always
RestartSec=3
LimitNOFILE=65536
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable phantom-server >/dev/null 2>&1 || true

# ── 4b. Private Psiphon OSSH server ───────────────────────────────
echo "[4b/5] Installing private Psiphon OSSH service …"
PSIPHON_SCRIPT="$REPO_DIR/packet-android/scripts/psiphon-core-lab.sh"
PSIPHON_SERVER_CONFIG="$REPO_DIR/packet-android/.psiphon-core-lab/server/psiphond.config"
PSIPHON_CLIENT_CONFIG="$REPO_DIR/packet-android/.psiphon-core-lab/client/client.config"
if [[ ! -x "$REPO_DIR/packet-android/.psiphon-core-lab/psiphon-tunnel-core-binaries/psiphond/psiphond" ]]; then
  bash "$PSIPHON_SCRIPT" fetch
fi
PSIPHON_PUBLIC_IP="$(curl -4 -s --max-time 5 https://api.ipify.org || hostname -I | awk '{print $1}')"
if [[ ! -f "$PSIPHON_SERVER_CONFIG" || ! -f "$PSIPHON_CLIENT_CONFIG" ]]; then
  PUBLIC_IP="$PSIPHON_PUBLIC_IP" PORT="$PSIPHON_PORT" PROTOCOL=OSSH bash "$PSIPHON_SCRIPT" generate-server
fi

cat > /etc/systemd/system/packet-psiphond.service <<UNIT
[Unit]
Description=Packet private Psiphon OSSH server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
EnvironmentFile=$SECRETS_FILE
WorkingDirectory=$REPO_DIR
ExecStart=/bin/bash $PSIPHON_SCRIPT run-server
Restart=always
RestartSec=3
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable packet-psiphond >/dev/null 2>&1 || true

# Open UFW only if it's the active firewall. Default VPSServer images leave
# UFW off, so this is a no-op there.
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q 'Status: active'; then
  ufw allow "$WS_PORT/tcp"   || true
  ufw allow "$OBFS_PORT/tcp" || true
  ufw allow "$QUIC_PORT/udp" || true
  ufw allow "$PSIPHON_PORT/tcp" || true
fi

# ── 5. Start + report ─────────────────────────────────────────────
echo "[5/5] Starting phantom-server …"
systemctl restart phantom-server
systemctl restart packet-psiphond
sleep 2
systemctl --no-pager --full status phantom-server | sed -n '1,18p' || true
systemctl --no-pager --full status packet-psiphond | sed -n '1,18p' || true

PUBLIC_IP4="$(curl -4 -s --max-time 5 https://api.ipify.org || echo "?")"
PUBLIC_IP6="$(curl -6 -s --max-time 5 https://api64.ipify.org || echo "")"

echo
echo "═══════════════════════════════════════════════════════════"
echo "  ✓ phantom-server is alive (systemd Restart=always)"
echo "═══════════════════════════════════════════════════════════"
echo "  Public IPv4 : ${PUBLIC_IP4}"
[[ -n "$PUBLIC_IP6" ]] && echo "  Public IPv6 : ${PUBLIC_IP6}"
echo "  WS/Meek port: $WS_PORT/tcp"
echo "  OBFS port   : $OBFS_PORT/tcp"
echo "  QUIC port   : $QUIC_PORT/udp"
echo "  Psiphon OSSH: $PSIPHON_PORT/tcp"
echo "  Secret      : $PHANTOM_SECRET"
echo "  OBFS key    : $OBFS_KEY"
echo "  Secrets file: $SECRETS_FILE"
echo "  Psiphon cfg : $PSIPHON_CLIENT_CONFIG"
echo
echo "  Smoke tests (run from anywhere):"
echo "    curl -sS http://${PUBLIC_IP4}:${WS_PORT}/api/v1/health"
echo "    curl -sS http://${PUBLIC_IP4}:${WS_PORT}/ | head -n 5"
echo "    nc -vz ${PUBLIC_IP4} ${PSIPHON_PORT}"
echo "    nc -vu -w2 ${PUBLIC_IP4} ${QUIC_PORT}   # UDP reachability only; no HTTP output expected"
echo
echo "  Tail logs:    journalctl -u phantom-server -f -n 50"
echo "                journalctl -u packet-psiphond -f -n 50"
echo "  Restart:      systemctl restart phantom-server"
echo "                systemctl restart packet-psiphond"
echo "  Stop:         systemctl stop phantom-server"
echo "                systemctl stop packet-psiphond"
echo
echo "  If packet-psiphond generated a new Psiphon server entry, copy it back:"
echo "    scp root@${PUBLIC_IP4}:${PSIPHON_CLIENT_CONFIG} packet-android/.psiphon-core-lab/client/client.config"
echo "    bash packet-android/scripts/psiphon-core-lab.sh install-client-asset"
echo
echo "  On your laptop, paste these into"
echo "  packet-android/app/src/main/java/com/resolo/packet/TunnelModels.kt:"
echo
echo "      const val CHAIN_SERVER_URL = \"http://${PUBLIC_IP4}:${WS_PORT}\""
echo "      const val CHAIN_EDGE       = \"${PUBLIC_IP4}:${WS_PORT}\""
echo "      const val CHAIN_SECRET     = \"${PHANTOM_SECRET}\""
echo "      const val CHAIN_OBFS_KEY   = \"${OBFS_KEY}\""
echo
echo "  Packet Native QUIC test profile:"
echo "      Server URL : http://${PUBLIC_IP4}:${WS_PORT}"
echo "      Secret     : ${PHANTOM_SECRET}"
echo "      Transport  : QUIC"
echo "      CDN Edge   : ${PUBLIC_IP4}:${QUIC_PORT}"
echo
echo "  Then rebuild the Android lib + APK and you're live."
echo
