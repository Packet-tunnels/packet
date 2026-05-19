#!/usr/bin/env bash
set -euo pipefail

# Linux laptop script.
# Starts the private Packet server on this laptop and writes the generated key
# to .packet-private.env so ssh.sh can print the same Android config.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

LOCAL_PORT="${LOCAL_PORT:-80}"
SECRET_FILE="${SECRET_FILE:-${REPO_ROOT}/.packet-private.env}"
STARLINK_WIFI_NAME="${STARLINK_WIFI_NAME:-yagoob}"

cd "$REPO_ROOT"

if [[ -f "$SECRET_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$SECRET_FILE"
fi

if [[ -z "${PHANTOM_SECRET:-}" ]]; then
  if command -v openssl >/dev/null 2>&1; then
    PHANTOM_SECRET="$(openssl rand -hex 32)"
  else
    PHANTOM_SECRET="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 48)"
  fi
fi
export PHANTOM_SECRET

cat >"$SECRET_FILE" <<ENV
PHANTOM_SECRET='${PHANTOM_SECRET}'
LOCAL_PORT='${LOCAL_PORT}'
STARLINK_WIFI_NAME='${STARLINK_WIFI_NAME}'
ENV
chmod 600 "$SECRET_FILE"

echo "Building phantom-server..."
cargo build -p phantom-server --release

SERVER_BIN="${REPO_ROOT}/target/release/phantom-server"

if [[ "$LOCAL_PORT" -lt 1024 && "$EUID" -ne 0 ]]; then
  if command -v setcap >/dev/null 2>&1; then
    echo "Granting low-port bind permission to phantom-server..."
    sudo setcap 'cap_net_bind_service=+ep' "$SERVER_BIN"
  else
    echo "Port ${LOCAL_PORT} needs root or setcap. Re-run with sudo, or install libcap2-bin." >&2
    exit 1
  fi
fi

if command -v ss >/dev/null 2>&1; then
  if ss -ltn "( sport = :${LOCAL_PORT} )" | tail -n +2 | grep -q .; then
    echo "Port ${LOCAL_PORT} is already in use. Stop nginx/apache/old server first." >&2
    ss -ltnp "( sport = :${LOCAL_PORT} )" || true
    exit 1
  fi
fi

echo
echo "Packet server starting"
echo "Local listen : 127.0.0.1:${LOCAL_PORT}"
echo "Secret file  : ${SECRET_FILE}"
echo "Secret       : ${PHANTOM_SECRET}"
echo
echo "Keep this terminal open. Ctrl+C stops the Packet server."
echo

exec "$SERVER_BIN" --port "$LOCAL_PORT"
