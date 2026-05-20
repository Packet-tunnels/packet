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
#   5. Starts the service on port 80 (HTTP/WS, dual-stack v4/v6 from main.rs)
#      and 8443 (OBFS).
#   6. Prints the secrets and the snippet you need to paste into
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
SECRETS_FILE="/root/packet-secrets.env"

if [[ ! -f "$REPO_DIR/Cargo.toml" ]] || [[ ! -d "$REPO_DIR/packet-server" ]]; then
  echo "ERROR: $REPO_DIR doesn't look like the packet repo (Cargo.toml + packet-server expected)." >&2
  echo "       cd into the repo first, or set REPO_DIR=/path/to/packet." >&2
  exit 1
fi

echo "═══════════════════════════════════════════════════════════"
echo "  phantom-server VPS setup"
echo "  Repo  : $REPO_DIR"
echo "  Ports : WS=$WS_PORT  OBFS=$OBFS_PORT"
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
EOF
  echo "       → saved to $SECRETS_FILE (mode 600)"
fi

# ── 2. Build deps + Rust ──────────────────────────────────────────
echo "[2/5] Installing build deps + Rust if missing …"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev curl ca-certificates >/dev/null
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
Description=Packet phantom-server (dual-stack v4/v6, WS + OBFS)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
EnvironmentFile=$SECRETS_FILE
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/phantom-server --port \${WS_PORT} --secret \${PHANTOM_SECRET} --obfs-port \${OBFS_PORT} --obfs-key \${OBFS_KEY}
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

# Open UFW only if it's the active firewall. Default VPSServer images leave
# UFW off, so this is a no-op there.
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q 'Status: active'; then
  ufw allow "$WS_PORT/tcp"   || true
  ufw allow "$OBFS_PORT/tcp" || true
fi

# ── 5. Start + report ─────────────────────────────────────────────
echo "[5/5] Starting phantom-server …"
systemctl restart phantom-server
sleep 2
systemctl --no-pager --full status phantom-server | sed -n '1,18p' || true

PUBLIC_IP4="$(curl -4 -s --max-time 5 https://api.ipify.org || echo "?")"
PUBLIC_IP6="$(curl -6 -s --max-time 5 https://api64.ipify.org || echo "")"

echo
echo "═══════════════════════════════════════════════════════════"
echo "  ✓ phantom-server is alive (systemd Restart=always)"
echo "═══════════════════════════════════════════════════════════"
echo "  Public IPv4 : ${PUBLIC_IP4}"
[[ -n "$PUBLIC_IP6" ]] && echo "  Public IPv6 : ${PUBLIC_IP6}"
echo "  WS port     : $WS_PORT"
echo "  OBFS port   : $OBFS_PORT"
echo "  Secret      : $PHANTOM_SECRET"
echo "  OBFS key    : $OBFS_KEY"
echo "  Secrets file: $SECRETS_FILE"
echo
echo "  Smoke tests (run from anywhere):"
echo "    curl -sS http://${PUBLIC_IP4}:${WS_PORT}/api/v1/health"
echo "    curl -sS http://${PUBLIC_IP4}:${WS_PORT}/ | head -n 5"
echo
echo "  Tail logs:    journalctl -u phantom-server -f -n 50"
echo "  Restart:      systemctl restart phantom-server"
echo "  Stop:         systemctl stop phantom-server"
echo
echo "  On your laptop, paste these into"
echo "  packet-android/app/src/main/java/com/resolo/packet/TunnelModels.kt:"
echo
echo "      const val CHAIN_SERVER_URL = \"http://${PUBLIC_IP4}:${WS_PORT}\""
echo "      const val CHAIN_EDGE       = \"${PUBLIC_IP4}:${WS_PORT}\""
echo "      const val CHAIN_SECRET     = \"${PHANTOM_SECRET}\""
echo "      const val CHAIN_OBFS_KEY   = \"${OBFS_KEY}\""
echo
echo "  Then rebuild the Android lib + APK and you're live."
echo
