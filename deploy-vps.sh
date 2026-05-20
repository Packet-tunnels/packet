#!/usr/bin/env bash
# deploy-vps.sh — One-shot deploy of phantom-server to a fresh VPS.
#
# Usage:
#   ./deploy-vps.sh [VPS_IP]    # default: 185.127.19.211 (the new VPSServer node)
#
# What it does:
#   1. Generates a fresh PHANTOM_SECRET (32 bytes hex) and OBFS_KEY (32 bytes
#      hex) on the local machine.
#   2. Rsyncs the local source tree to /root/packet on the VPS, excluding
#      build artefacts.
#   3. Installs Rust + build deps on the VPS if missing.
#   4. Builds phantom-server in release mode.
#   5. Installs a systemd unit, opens UFW ports if UFW is on, and starts the
#      service on port 80 (HTTP/WS) + 8443 (OBFS).
#   6. Rewrites packet-android/app/src/main/java/com/resolo/packet/TunnelModels.kt
#      so the in-app Chain points at the new IP/secret/obfs-key.
#   7. Saves the generated secrets locally so you don't lose them.
#
# Assumes you can ssh to root@<VPS_IP> (password or key). VPSServer's fresh
# Ubuntu/Debian images allow root SSH by default.

set -euo pipefail

VPS_IP="${1:-185.127.19.211}"
SSH_USER="${SSH_USER:-root}"
WS_PORT="${WS_PORT:-80}"
OBFS_PORT="${OBFS_PORT:-8443}"

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANDROID_MODELS="$PROJECT_ROOT/packet-android/app/src/main/java/com/resolo/packet/TunnelModels.kt"
SECRETS_OUT="$PROJECT_ROOT/.deploy-secrets-$VPS_IP.env"

echo "═══════════════════════════════════════════════════════════"
echo "  Packet VPS deploy"
echo "  Target : $SSH_USER@$VPS_IP"
echo "  Ports  : WS=$WS_PORT  OBFS=$OBFS_PORT"
echo "═══════════════════════════════════════════════════════════"

# ── 1. Generate secrets locally ───────────────────────────────────
gen_hex32() {
  # 32 bytes of cryptographic randomness, hex-encoded → 64 hex chars
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    head -c 32 /dev/urandom | xxd -p -c 64
  fi
}
PHANTOM_SECRET="$(gen_hex32)"
OBFS_KEY="$(gen_hex32)"
echo "[1/7] Generated PHANTOM_SECRET (64 hex) and OBFS_KEY (64 hex)."

cat > "$SECRETS_OUT" <<EOF
# Saved by deploy-vps.sh on $(date -u +"%Y-%m-%dT%H:%M:%SZ")
PHANTOM_SECRET=$PHANTOM_SECRET
OBFS_KEY=$OBFS_KEY
VPS_IP=$VPS_IP
WS_PORT=$WS_PORT
OBFS_PORT=$OBFS_PORT
EOF
chmod 600 "$SECRETS_OUT"
echo "      → saved to $SECRETS_OUT (mode 600)"

# ── 2. Sanity-check SSH ───────────────────────────────────────────
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ConnectTimeout=15)
SSH_CMD=(ssh "${SSH_OPTS[@]}" "$SSH_USER@$VPS_IP")
echo "[2/7] Testing SSH to $SSH_USER@$VPS_IP …"
if ! "${SSH_CMD[@]}" "echo ok" >/dev/null 2>&1; then
  echo "ERROR: cannot SSH to $SSH_USER@$VPS_IP." >&2
  echo "If you have not pushed your key yet, run:" >&2
  echo "  ssh-copy-id $SSH_USER@$VPS_IP" >&2
  echo "or set SSH_USER=… if root login is disabled." >&2
  exit 1
fi

# ── 3. Rsync source tree to /root/packet ──────────────────────────
echo "[3/7] Rsyncing source to $SSH_USER@$VPS_IP:/root/packet …"
rsync -az --delete \
  --exclude '/target' --exclude '/.git' --exclude 'node_modules' \
  --exclude '/packet-android/build' --exclude '/packet-ios/build' \
  --exclude '*.apk' --exclude '*.ipa' \
  -e "ssh ${SSH_OPTS[*]}" \
  "$PROJECT_ROOT/" "$SSH_USER@$VPS_IP:/root/packet/"

# ── 4. Install Rust + build deps remotely ─────────────────────────
echo "[4/7] Installing Rust + build deps on the VPS …"
"${SSH_CMD[@]}" 'bash -se' <<'REMOTE_SETUP'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev curl ca-certificates >/dev/null
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
echo "  rustc: $(/root/.cargo/bin/rustc --version 2>/dev/null || rustc --version)"
REMOTE_SETUP

# ── 5. Build phantom-server in release mode ───────────────────────
echo "[5/7] Building phantom-server (release) — first build can take ~3 min …"
"${SSH_CMD[@]}" 'bash -se' <<'REMOTE_BUILD'
set -euo pipefail
source /root/.cargo/env
cd /root/packet
# Limit parallelism on small VPS (≤2 GB RAM) to avoid OOM during link.
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build --release -p phantom-server 2>&1 \
  | tail -n 8
test -x /root/packet/target/release/phantom-server
install -m 0755 /root/packet/target/release/phantom-server /usr/local/bin/phantom-server
REMOTE_BUILD

# ── 6. Install systemd unit, open firewall, start service ─────────
echo "[6/7] Installing systemd unit, opening firewall, starting service …"
"${SSH_CMD[@]}" "PHANTOM_SECRET='$PHANTOM_SECRET' OBFS_KEY='$OBFS_KEY' WS_PORT='$WS_PORT' OBFS_PORT='$OBFS_PORT' bash -se" <<'REMOTE_SVC'
set -euo pipefail
cat > /etc/systemd/system/phantom-server.service <<UNIT
[Unit]
Description=Packet phantom-server (dual-stack v4/v6, WS + OBFS)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
Environment=PHANTOM_SECRET=$PHANTOM_SECRET
Environment=OBFS_KEY=$OBFS_KEY
Environment=RUST_LOG=info
# Bind to all interfaces (IPv4 + IPv6 via the dual-stack code in main.rs).
ExecStart=/usr/local/bin/phantom-server --port $WS_PORT --secret \${PHANTOM_SECRET} --obfs-port $OBFS_PORT --obfs-key \${OBFS_KEY}
Restart=always
RestartSec=3
LimitNOFILE=65536
# Allow binding to privileged ports (80) without running setcap each time.
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable phantom-server >/dev/null 2>&1 || true

# Open firewall if UFW is active. iptables/nftables-managed images can stay
# default-open; we only touch UFW because it's the common explicit firewall.
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q 'Status: active'; then
  ufw allow $WS_PORT/tcp   || true
  ufw allow $OBFS_PORT/tcp || true
fi

systemctl restart phantom-server
sleep 2
systemctl --no-pager --full status phantom-server | head -n 18 || true
REMOTE_SVC

# ── 7. Rewrite the Android Chain constants to point at this VPS ───
echo "[7/7] Updating $ANDROID_MODELS to point at the new VPS …"
if [[ ! -f "$ANDROID_MODELS" ]]; then
  echo "  WARN: $ANDROID_MODELS not found — skipping Kotlin rewrite."
else
  python3 - "$ANDROID_MODELS" "$VPS_IP" "$WS_PORT" "$PHANTOM_SECRET" "$OBFS_KEY" <<'PY'
import re, sys, pathlib
path, ip, port, secret, obfs = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
src = pathlib.Path(path).read_text()
def replace_const(name: str, value: str, text: str) -> str:
    pattern = rf'(const val\s+{re.escape(name)}\s*=\s*")[^"]*(")'
    new_text, n = re.subn(pattern, lambda m: f'{m.group(1)}{value}{m.group(2)}', text)
    if n == 0:
        print(f"  WARN: no const named {name} found — left unchanged.")
    return new_text
src = replace_const("CHAIN_SERVER_URL", f"http://{ip}:{port}", src)
src = replace_const("CHAIN_EDGE",       f"{ip}:{port}",         src)
src = replace_const("CHAIN_SECRET",     secret,                 src)
src = replace_const("CHAIN_OBFS_KEY",   obfs,                   src)
pathlib.Path(path).write_text(src)
print("  TunnelModels.kt updated.")
PY
fi

echo
echo "═══════════════════════════════════════════════════════════"
echo "  ✓ Deploy complete"
echo "═══════════════════════════════════════════════════════════"
echo "  Server URL  : http://$VPS_IP:$WS_PORT"
echo "  OBFS port   : $OBFS_PORT"
echo "  Secret      : $PHANTOM_SECRET"
echo "  OBFS key    : $OBFS_KEY"
echo "  Secrets file: $SECRETS_OUT"
echo
echo "  Quick smoke test:"
echo "    curl -sS http://$VPS_IP:$WS_PORT/ | head -n 5"
echo "    curl -sS http://$VPS_IP:$WS_PORT/api/v1/health"
echo
echo "  Tail server logs:"
echo "    ssh $SSH_USER@$VPS_IP 'journalctl -u phantom-server -f -n 50'"
echo
echo "  Next: rebuild Android with ./packet-android/scripts/build-rust-android.sh"
echo "  then assemble the APK. The Chain config now points at this VPS."
echo
