#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  tools/test-cdn-http.sh EDGE_IP HOST_HEADER SECRET [PORT] [SCHEME]

Example:
  tools/test-cdn-http.sh 185.239.1.185 piano-lessons.site 'shared-secret'

What it checks:
  1. GET / through the edge IP with Host override
  2. POST /api/v1/auth/login through the edge IP with Host override
  3. GET / through the hostname forced to the edge IP via --resolve
  4. POST /api/v1/auth/login through the hostname forced to the edge IP via --resolve
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 3 || $# -gt 5 ]]; then
  usage >&2
  exit 1
fi

EDGE_IP="$1"
HOST_HEADER="$2"
SECRET="$3"
PORT="${4:-80}"
SCHEME="${5:-http}"

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi

if ! command -v openssl >/dev/null 2>&1; then
  echo "openssl is required" >&2
  exit 1
fi

if ! command -v xxd >/dev/null 2>&1; then
  echo "xxd is required" >&2
  exit 1
fi

TS="$(date +%s)"
SIG="$(printf "%s" "$TS" | openssl dgst -sha256 -hmac "$SECRET" -binary | xxd -p -c 256)"
BODY="$(mktemp)"
trap 'rm -f "$BODY"' EXIT

ROOT_URL="${SCHEME}://${EDGE_IP}:${PORT}/"
AUTH_URL="${SCHEME}://${EDGE_IP}:${PORT}/api/v1/auth/login"
HOST_ROOT_URL="${SCHEME}://${HOST_HEADER}:${PORT}/"
HOST_AUTH_URL="${SCHEME}://${HOST_HEADER}:${PORT}/api/v1/auth/login"
RESOLVE_ARG="${HOST_HEADER}:${PORT}:${EDGE_IP}"

common_curl_args=(
  -4sS
  -D -
  --connect-timeout 15
  --max-time 20
)

if [[ "$SCHEME" == "https" ]]; then
  common_curl_args+=(-k)
fi

run_test() {
  local label="$1"
  local url="$2"
  shift 2

  rm -f "$BODY"
  echo "--- $label ---"
  curl "${common_curl_args[@]}" "$@" "$url" -o "$BODY"
  local code=$?
  echo "curl_exit=$code"
  if [[ -f "$BODY" ]]; then
    echo "body_preview=$(tr '\n' ' ' < "$BODY" | cut -c1-240)"
  fi
  echo
}

echo "timestamp=$TS"
echo "edge_ip=$EDGE_IP"
echo "host_header=$HOST_HEADER"
echo "port=$PORT"
echo "scheme=$SCHEME"
echo

run_test \
  "Edge root with Host header" \
  "$ROOT_URL" \
  -H "Host: $HOST_HEADER"

run_test \
  "Edge auth with Host header" \
  "$AUTH_URL" \
  -H "Host: $HOST_HEADER" \
  -H "Content-Type: application/json" \
  --data "{\"ts\":$TS,\"sig\":\"$SIG\"}"

run_test \
  "Hostname root via --resolve" \
  "$HOST_ROOT_URL" \
  --resolve "$RESOLVE_ARG"

run_test \
  "Hostname auth via --resolve" \
  "$HOST_AUTH_URL" \
  --resolve "$RESOLVE_ARG" \
  -H "Content-Type: application/json" \
  --data "{\"ts\":$TS,\"sig\":\"$SIG\"}"
