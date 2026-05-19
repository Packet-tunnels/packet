#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PROBE_SCRIPT="$SCRIPT_DIR/termux-vpn-probe.sh"
LABEL="${1:-packet}"
TROJAN_URI="${TROJAN_URI:-trojan://tLhdt8oOuoddtEVCtQt6WOVY@cdn.leorre.com:443?path=%2Fassignment&security=tls&host=cdn.leorre.com&type=ws&sni=cdn.leorre.com#DirectSock}"
ORIGIN_IP="${ORIGIN_IP:-103.241.67.247}"

if [ ! -f "$PROBE_SCRIPT" ]; then
  echo "Missing probe script: $PROBE_SCRIPT" >&2
  exit 1
fi

emit_install_block() {
  cat <<'HEADER'
pkg update
pkg install -y curl dnsutils iproute2 net-tools
cat > termux-vpn-probe.sh <<'PACKET_TERMUX_PROBE'
HEADER
  cat "$PROBE_SCRIPT"
  cat <<'FOOTER'
PACKET_TERMUX_PROBE
chmod +x termux-vpn-probe.sh
FOOTER
}

emit_tail() {
  cat <<'FOOTER'
echo ""
echo "Saved result files:"
ls -lt "$HOME/packet-vpn-probes" | head -n 10
FOOTER
}

case "$LABEL" in
  psiphon)
    emit_install_block
    cat <<'RUN'
sh termux-vpn-probe.sh psiphon
RUN
    emit_tail
    ;;
  packet)
    emit_install_block
    printf "TROJAN_URI='%s' ORIGIN_IP='%s' sh termux-vpn-probe.sh packet\n" "$TROJAN_URI" "$ORIGIN_IP"
    emit_tail
    ;;
  *)
    echo "Usage: $0 [psiphon|packet]" >&2
    exit 1
    ;;
esac
