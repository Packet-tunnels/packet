#!/usr/bin/env bash
set -euo pipefail

HOST=""
EDGE=""
HTTP_PATH="/api/v1/health"
WS_PATH="/api/v1/lessons/live"
UA="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
WS_KEY="dGhlIHNhbXBsZSBub25jZQ=="

usage() {
  cat <<'EOF'
Usage: diagnose-fronted-ws.sh --host HOST [--edge IP]

Probes fronted HTTP/HTTPS health and WebSocket upgrade paths and prints a
diagnosis for CDN/front-proxy failures.

Examples:
  ./tools/diagnose-fronted-ws.sh --host piano-lessons.site
  ./tools/diagnose-fronted-ws.sh --host piano-lessons.site --edge 185.239.1.185
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      HOST="${2:-}"
      shift 2
      ;;
    --edge)
      EDGE="${2:-}"
      shift 2
      ;;
    --http-path)
      HTTP_PATH="${2:-}"
      shift 2
      ;;
    --ws-path)
      WS_PATH="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$HOST" ]]; then
  usage >&2
  exit 1
fi

http_resolve=()
https_resolve=()
if [[ -n "$EDGE" ]]; then
  http_resolve=(--resolve "${HOST}:80:${EDGE}")
  https_resolve=(--resolve "${HOST}:443:${EDGE}")
fi

cleanup_files=()
cleanup() {
  for file in "${cleanup_files[@]}"; do
    rm -f "$file"
  done
}
trap cleanup EXIT

probe() {
  local label="$1"
  local url="$2"
  shift 2

  local headers body
  headers="$(mktemp)"
  body="$(mktemp)"
  cleanup_files+=("$headers" "$body")

  local curl_exit=0
  if ! curl -skS --http1.1 --max-time 10 -D "$headers" -o "$body" "$@" "$url"; then
    curl_exit=$?
  fi

  local status server reason
  status="$(awk '$1 ~ /^HTTP\// { code = $2 } END { print code }' "$headers")"
  server="$(awk -F': ' 'tolower($1)=="server" { value=$2 } END { gsub(/\r/, "", value); print value }' "$headers")"
  reason="$(awk -F': ' 'tolower($1)=="wcdn-nfc-reason" { value=$2 } END { gsub(/\r/, "", value); print value }' "$headers")"

  if [[ -z "$status" ]]; then
    status="curl:${curl_exit}"
  fi

  printf '%-14s status=%-10s server=%-12s reason=%s\n' "$label" "$status" "${server:-unknown}" "${reason:-n/a}"

  case "$label" in
    http-health) HTTP_HEALTH_STATUS="$status" ;;
    https-health) HTTPS_HEALTH_STATUS="$status" ;;
    http-ws) HTTP_WS_STATUS="$status"; HTTP_WS_SERVER="${server:-}"; HTTP_WS_REASON="${reason:-}" ;;
    https-ws) HTTPS_WS_STATUS="$status"; HTTPS_WS_SERVER="${server:-}"; HTTPS_WS_REASON="${reason:-}" ;;
  esac
}

probe "http-health" "http://${HOST}${HTTP_PATH}" "${http_resolve[@]}"
probe "https-health" "https://${HOST}${HTTP_PATH}" "${https_resolve[@]}"
probe \
  "http-ws" \
  "http://${HOST}${WS_PATH}" \
  "${http_resolve[@]}" \
  -H "Origin: http://${HOST}" \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: ${WS_KEY}" \
  -H "User-Agent: ${UA}" \
  -H "Accept-Language: en-US,en;q=0.9,fa;q=0.8" \
  -H "Cache-Control: no-cache" \
  -H "Pragma: no-cache"
probe \
  "https-ws" \
  "https://${HOST}${WS_PATH}" \
  "${https_resolve[@]}" \
  -H "Origin: https://${HOST}" \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: ${WS_KEY}" \
  -H "User-Agent: ${UA}" \
  -H "Accept-Language: en-US,en;q=0.9,fa;q=0.8" \
  -H "Cache-Control: no-cache" \
  -H "Pragma: no-cache"

echo
echo "Diagnosis:"
if [[ "${HTTP_HEALTH_STATUS:-}" == "200" && "${HTTPS_HEALTH_STATUS:-}" == "400" ]]; then
  echo "- Plain HTTP reaches the origin, but HTTPS does not. The CDN HTTPS origin configuration is broken."
fi

if [[ "${HTTP_WS_STATUS:-}" == "400" && "${HTTPS_WS_STATUS:-}" == "400" ]]; then
  echo "- Both WebSocket upgrades fail at the fronted path. The CDN is not delivering a usable upgrade to origin."
fi

if [[ "${HTTP_WS_SERVER:-}" == WCDN* || "${HTTPS_WS_SERVER:-}" == WCDN* ]]; then
  echo "- The 400 comes from WCDN/front proxy handling, not from the local Axum route itself."
fi

if [[ "${HTTP_HEALTH_STATUS:-}" == "200" && "${HTTP_WS_STATUS:-}" == "400" ]]; then
  echo "- Regular HTTP forwarding works while WS does not. Check whether WebSocket forwarding is enabled on the CDN and whether any intermediate proxy preserves Upgrade/Connection headers."
fi
