pkg update
pkg install -y curl dnsutils iproute2 net-tools
cat > termux-vpn-probe.sh <<'PACKET_TERMUX_PROBE'
#!/usr/bin/env sh
# Run inside Termux while one VPN/proxy is connected.
# Usage:
#   sh termux-vpn-probe.sh psiphon
#   TROJAN_URI='trojan://...' ORIGIN_IP='1.2.3.4' sh termux-vpn-probe.sh packet
set -u

LABEL="${1:-run}"
OUT_DIR="${OUT_DIR:-$HOME/packet-vpn-probes}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ 2>/dev/null || date +%Y%m%dT%H%M%S)"
OUT_FILE="$OUT_DIR/${STAMP}-${LABEL}.log"
SUMMARY_FILE="$OUT_DIR/${STAMP}-${LABEL}-summary.txt"
TMP_DIR="$OUT_DIR/.tmp-$$"

TROJAN_URI="${TROJAN_URI:-}"
ORIGIN_IP="${ORIGIN_IP:-}"
LOCAL_PROXY_PORTS="${LOCAL_PROXY_PORTS:-10808 1080 8080 8118 7070 8888}"

mkdir -p "$OUT_DIR" "$TMP_DIR"
: >"$OUT_FILE"
: >"$SUMMARY_FILE"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log() {
  printf '%s\n' "$*" | tee -a "$OUT_FILE"
}

summary() {
  printf '%s\n' "$*" >>"$SUMMARY_FILE"
}

section() {
  log ""
  log "===== $* ====="
}

have() {
  command -v "$1" >/dev/null 2>&1
}

run_sh() {
  _title="$1"
  _cmd="$2"
  _tmp="$TMP_DIR/cmd.out"
  section "$_title"
  log "$ $_cmd"
  sh -c "$_cmd" >"$_tmp" 2>&1
  _rc=$?
  cat "$_tmp" | tee -a "$OUT_FILE"
  log "[exit=$_rc]"
}

preview_file() {
  _file="$1"
  _max="${2:-40}"
  if [ -s "$_file" ]; then
    sed -n "1,${_max}p" "$_file" | sed 's/\r$//'
  fi
}

probe_url() {
  _name="$1"
  _url="$2"
  shift 2
  _body="$TMP_DIR/${_name}.body"
  _err="$TMP_DIR/${_name}.err"
  _metrics="$TMP_DIR/${_name}.metrics"

  section "HTTP probe: $_name"
  log "url=$_url"
  if ! have curl; then
    log "curl is missing. In Termux run: pkg install -y curl"
    summary "http_probe $_name status=missing_curl"
    return
  fi

  curl -L -sS --connect-timeout 10 --max-time 30 \
    -o "$_body" \
    -w 'http_code=%{http_code} remote_ip=%{remote_ip} local_ip=%{local_ip} time_connect=%{time_connect} time_appconnect=%{time_appconnect} time_starttransfer=%{time_starttransfer} time_total=%{time_total} size=%{size_download}\n' \
    "$@" "$_url" >"$_metrics" 2>"$_err"
  _rc=$?

  _metric_line="$(cat "$_metrics" 2>/dev/null)"
  _err_line="$(preview_file "$_err" 8 | tr '\n' ' ' | sed 's/  */ /g')"

  log "curl_exit=$_rc"
  log "$_metric_line"
  if [ -n "$_err_line" ]; then
    log "curl_error=$_err_line"
  fi
  log "-- response preview --"
  preview_file "$_body" 30 | tee -a "$OUT_FILE"

  _http_code="$(printf '%s\n' "$_metric_line" | sed -n 's/.*http_code=\([0-9][0-9][0-9]\).*/\1/p')"
  _remote_ip="$(printf '%s\n' "$_metric_line" | sed -n 's/.*remote_ip=\([^ ]*\).*/\1/p')"
  summary "http_probe $_name curl_exit=$_rc http_code=${_http_code:-unknown} remote_ip=${_remote_ip:-unknown} error=${_err_line:-none}"
}

probe_proxy_port() {
  _port="$1"
  _mode="$2"
  _body="$TMP_DIR/proxy-${_mode}-${_port}.body"
  _err="$TMP_DIR/proxy-${_mode}-${_port}.err"
  _metrics="$TMP_DIR/proxy-${_mode}-${_port}.metrics"

  if [ "$_mode" = "http" ]; then
    _proxy_arg="--proxy"
    _proxy_value="http://127.0.0.1:${_port}"
  else
    _proxy_arg="--socks5-hostname"
    _proxy_value="127.0.0.1:${_port}"
  fi

  curl -sS --connect-timeout 3 --max-time 12 \
    "$_proxy_arg" "$_proxy_value" \
    -o "$_body" \
    -w 'http_code=%{http_code} remote_ip=%{remote_ip} time_total=%{time_total}\n' \
    'http://cloudflare.com/cdn-cgi/trace' >"$_metrics" 2>"$_err"
  _rc=$?
  _metric_line="$(cat "$_metrics" 2>/dev/null)"
  _preview="$(preview_file "$_body" 12 | tr '\n' ';' | sed 's/  */ /g')"
  _err_line="$(preview_file "$_err" 4 | tr '\n' ' ' | sed 's/  */ /g')"
  log "${_mode}_proxy_port=$_port curl_exit=$_rc $_metric_line"
  if [ -n "$_preview" ]; then
    log "preview=$_preview"
  fi
  if [ -n "$_err_line" ]; then
    log "error=$_err_line"
  fi
  summary "local_proxy mode=$_mode port=$_port curl_exit=$_rc metrics=$_metric_line error=${_err_line:-none}"
}

extract_query_value() {
  _key="$1"
  printf '%s' "$TROJAN_URI" |
    sed 's/#.*//' |
    sed 's/.*?//' |
    tr '&' '\n' |
    sed -n "s/^${_key}=//p" |
    sed -n '1p'
}

decode_path() {
  printf '%s' "$1" |
    sed 's/%2[Ff]/\//g; s/%3[Aa]/:/g; s/%3[Ff]/?/g; s/%26/\&/g'
}

section "Run metadata"
log "label=$LABEL"
log "utc_stamp=$STAMP"
log "log_file=$OUT_FILE"
log "summary_file=$SUMMARY_FILE"
summary "label=$LABEL"
summary "utc_stamp=$STAMP"

section "Dependency check"
for _bin in curl ip getprop settings ss netstat nslookup dig logcat dumpsys; do
  if have "$_bin"; then
    log "$_bin=present ($(command -v "$_bin"))"
  else
    log "$_bin=missing"
  fi
done
log "Termux packages useful for richer output: pkg install -y curl dnsutils iproute2 net-tools"

run_sh "Android build/device properties" "getprop ro.product.manufacturer; getprop ro.product.model; getprop ro.build.version.release; getprop ro.build.version.sdk; getprop ro.product.cpu.abi"
run_sh "Android proxy settings" "settings get global http_proxy 2>/dev/null; settings get global global_http_proxy_host 2>/dev/null; settings get global global_http_proxy_port 2>/dev/null; settings get global private_dns_mode 2>/dev/null; settings get global private_dns_specifier 2>/dev/null"
run_sh "DNS-related properties" "getprop | grep -Ei 'dns|proxy|vpn|net\\.iface|net\\.gateway' | head -n 120"

if have ip; then
  run_sh "Interfaces" "ip -o addr show"
  run_sh "IPv4 routes" "ip route show table all"
  run_sh "IPv6 routes" "ip -6 route show table all 2>/dev/null | head -n 120"
  run_sh "Policy routing rules" "ip rule show"
else
  log "ip command missing; install with: pkg install -y iproute2"
fi

if have ss; then
  run_sh "Listening TCP sockets" "ss -lnt 2>/dev/null"
else
  run_sh "Listening TCP sockets via netstat" "netstat -lnt 2>/dev/null || true"
fi

if have dumpsys; then
  run_sh "Android connectivity VPN/DNS hints" "dumpsys connectivity 2>/dev/null | grep -Ei 'VPN|NetworkAgentInfo|LinkProperties|DnsAddresses|Routes|InterfaceName|Validated|Captive' | head -n 180"
fi

section "Name resolution"
if have getent; then
  run_sh "getent hosts" "getent hosts cloudflare.com google.com cdn.leorre.com 2>/dev/null || true"
fi
if have nslookup; then
  run_sh "nslookup" "nslookup cloudflare.com 2>/dev/null; nslookup cdn.leorre.com 2>/dev/null"
elif have dig; then
  run_sh "dig" "dig +short cloudflare.com; dig +short cdn.leorre.com"
fi

probe_url "direct_cloudflare_http_trace" "http://cloudflare.com/cdn-cgi/trace"
probe_url "direct_cloudflare_https_trace" "https://cloudflare.com/cdn-cgi/trace"
probe_url "direct_ip_api" "http://ip-api.com/line/?fields=status,message,query,country,countryCode,as,isp"
probe_url "direct_google_generate_204" "https://www.google.com/generate_204"
probe_url "direct_example_https" "https://example.com"

section "Local proxy port scan"
log "Testing common localhost proxy ports as both HTTP and SOCKS5."
for _port in $LOCAL_PROXY_PORTS; do
  probe_proxy_port "$_port" "http"
  probe_proxy_port "$_port" "socks5"
done

if [ -n "$TROJAN_URI" ]; then
  section "Trojan URI endpoint checks"
  _trojan_host="$(printf '%s' "$TROJAN_URI" | sed -n 's#^trojan://[^@]*@\([^:/?]*\).*#\1#p')"
  _trojan_port="$(printf '%s' "$TROJAN_URI" | sed -n 's#^trojan://[^@]*@[^:/?]*:\([0-9][0-9]*\).*#\1#p')"
  _trojan_path="$(extract_query_value path)"
  _trojan_sni="$(extract_query_value sni)"
  _trojan_host_header="$(extract_query_value host)"
  _trojan_type="$(extract_query_value type)"
  _trojan_security="$(extract_query_value security)"
  _trojan_path_decoded="$(decode_path "${_trojan_path:-/}")"
  [ -n "$_trojan_port" ] || _trojan_port="443"

  log "trojan_host=$_trojan_host"
  log "trojan_port=$_trojan_port"
  log "trojan_path=$_trojan_path_decoded"
  log "trojan_sni=$_trojan_sni"
  log "trojan_host_header=$_trojan_host_header"
  log "trojan_type=$_trojan_type"
  log "trojan_security=$_trojan_security"
  summary "trojan host=$_trojan_host port=$_trojan_port path=$_trojan_path_decoded sni=$_trojan_sni host_header=$_trojan_host_header type=$_trojan_type security=$_trojan_security"

  if [ -n "$_trojan_host" ]; then
    probe_url "trojan_cloudflare_plain_ws_path" "https://${_trojan_host}:${_trojan_port}${_trojan_path_decoded}" --http1.1
    if [ -n "$ORIGIN_IP" ]; then
      probe_url "trojan_origin_direct_resolve" "https://${_trojan_host}:${_trojan_port}${_trojan_path_decoded}" --http1.1 -k --resolve "${_trojan_host}:${_trojan_port}:${ORIGIN_IP}"
    else
      summary "trojan_origin_direct_resolve skipped set_ORIGIN_IP_to_test_direct_origin"
    fi
  fi
else
  summary "trojan skipped set_TROJAN_URI_to_probe_endpoint"
fi

if have logcat; then
  run_sh "Recent VPN-related logcat lines if accessible" "logcat -d -t 300 2>/dev/null | grep -Ei 'psiphon|packet|directsock|vpn|tun2socks|socks|trojan|xray|dns' | tail -n 120 || true"
fi

section "Final summary"
cat "$SUMMARY_FILE" | tee -a "$OUT_FILE"
log ""
log "Saved full log: $OUT_FILE"
log "Saved summary:  $SUMMARY_FILE"
log ""
log "Run once with Psiphon connected, then run once with Packet/DirectSock connected."
log "Send both *-summary.txt files and the full .log files if the summaries differ."
PACKET_TERMUX_PROBE
chmod +x termux-vpn-probe.sh
TROJAN_URI='trojan://tLhdt8oOuoddtEVCtQt6WOVY@cdn.leorre.com:443?path=%2Fassignment&security=tls&host=cdn.leorre.com&type=ws&sni=cdn.leorre.com#DirectSock' ORIGIN_IP='103.241.67.247' sh termux-vpn-probe.sh packet
echo ""
echo "Saved result files:"
ls -lt "$HOME/packet-vpn-probes" | head -n 10
