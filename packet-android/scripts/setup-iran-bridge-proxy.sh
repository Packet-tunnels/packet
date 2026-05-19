#!/usr/bin/env bash
set -euo pipefail

# Private first-hop bridge for Packet DirectSock.
#
# Run on a VPS reachable from Iranian mobile networks. The bridge is an
# authenticated HTTP CONNECT proxy and is restricted to known Trojan/Cloudflare
# targets so it is not an open proxy.

PORT="${PORT:-18080}"
USER_NAME="${USER_NAME:-packet}"
PASSWORD="${PASSWORD:-$(tr -dc 'A-Za-z0-9' </dev/urandom | head -c 24)}"
ALLOWED_TARGETS="${ALLOWED_TARGETS:-cdn.leorre.com:443,www.creationlong.org:443,172.64.152.23:443}"
SERVICE_USER="${SERVICE_USER:-packet-bridge}"
INSTALL_DIR="${INSTALL_DIR:-/opt/packet-bridge}"
BRIDGE_HOST="${BRIDGE_HOST:-$(hostname -I 2>/dev/null | awk '{print $1}')}"

if [[ $EUID -ne 0 ]]; then
  echo "Run as root: sudo -i"
  exit 1
fi

apt-get update
apt-get install -y python3 ca-certificates curl

id -u "$SERVICE_USER" >/dev/null 2>&1 || useradd --system --home "$INSTALL_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
mkdir -p "$INSTALL_DIR"

cat >"$INSTALL_DIR/packet_bridge_proxy.py" <<'PY'
#!/usr/bin/env python3
import base64
import os
import selectors
import socket
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("PACKET_BRIDGE_PORT", "18080"))
USER = os.environ.get("PACKET_BRIDGE_USER", "packet")
PASSWORD = os.environ["PACKET_BRIDGE_PASSWORD"]
ALLOWED = {
    item.strip()
    for item in os.environ.get("PACKET_BRIDGE_ALLOWED", "").split(",")
    if item.strip()
}


def auth_ok(header):
    if not header or not header.startswith("Basic "):
        return False
    try:
        raw = base64.b64decode(header[6:].strip()).decode("utf-8", "replace")
    except Exception:
        return False
    return raw == f"{USER}:{PASSWORD}"


def parse_target(path):
    if ":" not in path:
        return None
    host, port_s = path.rsplit(":", 1)
    try:
        port = int(port_s)
    except ValueError:
        return None
    if not host or port < 1 or port > 65535:
        return None
    return host, port


def relay(client, upstream):
    client.setblocking(False)
    upstream.setblocking(False)
    sel = selectors.DefaultSelector()
    sel.register(client, selectors.EVENT_READ, upstream)
    sel.register(upstream, selectors.EVENT_READ, client)
    try:
        while True:
            events = sel.select(300)
            if not events:
                break
            for key, _ in events:
                src = key.fileobj
                dst = key.data
                data = src.recv(65536)
                if not data:
                    return
                dst.sendall(data)
    finally:
        sel.close()


class Handler(BaseHTTPRequestHandler):
    timeout = 15

    def log_message(self, fmt, *args):
        print("%s - %s" % (self.client_address[0], fmt % args), flush=True)

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"packet-bridge ok\n")
            return
        self.send_error(405, "CONNECT only")

    def do_CONNECT(self):
        if not auth_ok(self.headers.get("Proxy-Authorization")):
            self.send_response(407, "Proxy Authentication Required")
            self.send_header("Proxy-Authenticate", 'Basic realm="PacketBridge"')
            self.end_headers()
            return

        target = parse_target(self.path)
        if target is None:
            self.send_error(400, "Bad CONNECT target")
            return
        host, port = target
        key = f"{host}:{port}"
        if key not in ALLOWED:
            self.send_error(403, "Target not allowed")
            return

        try:
            upstream = socket.create_connection((host, port), timeout=15)
        except OSError as exc:
            self.send_error(502, "Upstream connect failed: %s" % exc)
            return

        self.send_response(200, "Connection Established")
        self.end_headers()
        try:
            relay(self.connection, upstream)
        finally:
            try:
                upstream.close()
            except Exception:
                pass


def main():
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    print(
        "packet bridge listening on 0.0.0.0:%d allowed=%s"
        % (PORT, ",".join(sorted(ALLOWED))),
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
PY

chmod 0755 "$INSTALL_DIR/packet_bridge_proxy.py"
chown -R "$SERVICE_USER:$SERVICE_USER" "$INSTALL_DIR"

cat >/etc/systemd/system/packet-bridge.service <<EOF
[Unit]
Description=Packet private DirectSock first-hop bridge
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_USER
Environment=PACKET_BRIDGE_PORT=$PORT
Environment=PACKET_BRIDGE_USER=$USER_NAME
Environment=PACKET_BRIDGE_PASSWORD=$PASSWORD
Environment=PACKET_BRIDGE_ALLOWED=$ALLOWED_TARGETS
ExecStart=/usr/bin/python3 $INSTALL_DIR/packet_bridge_proxy.py
Restart=always
RestartSec=2
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable packet-bridge
systemctl restart packet-bridge

if command -v ufw >/dev/null 2>&1; then
  ufw allow "$PORT/tcp" || true
fi

UPSTREAM_ENC="http%3A%2F%2F${USER_NAME}%3A${PASSWORD}%40${BRIDGE_HOST}%3A${PORT}"

echo ""
echo "Packet bridge is installed."
echo "Status:"
systemctl status packet-bridge --no-pager || true
echo ""
echo "Bridge:"
echo "  host: $BRIDGE_HOST"
echo "  port: $PORT"
echo "  user: $USER_NAME"
echo "  pass: $PASSWORD"
echo "  allowed: $ALLOWED_TARGETS"
echo ""
echo "Append this to a trojan:// URI query:"
echo "  upstream=$UPSTREAM_ENC"
echo ""
echo "Example:"
echo "  trojan://PASSWORD@cdn.leorre.com:443?path=%2Fassignment&security=tls&host=cdn.leorre.com&type=ws&sni=cdn.leorre.com&upstream=$UPSTREAM_ENC#DirectSock"
echo ""
echo "From your Mac, quick check:"
echo "  curl -i --proxy http://$USER_NAME:$PASSWORD@$BRIDGE_HOST:$PORT https://cdn.leorre.com/assignment"
