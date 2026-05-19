#!/usr/bin/env python3
# diagnostic.py — Termux fallback for the Packet connection diagnostic.
#
# Same experiment as the in-app DiagnosticActivity, but runnable from Termux
# with zero app rebuild. Python's socket/ssl is enough to classify *how* a
# connection fails, which tells us *which* DPI mechanism hit it.
#
# HOW TO USE
#   1. Install once:   pkg install python
#   2. Turn Psiphon ON.  Run:  python diagnostic.py   -> save output (report A)
#   3. Turn Psiphon OFF. Run again                     -> save output (report B)
#   4. Send both. The EGRESS line says which mode each report was in; the
#      diff between A and B is exactly what Psiphon's escape provides.
#
# When Psiphon's VPN is active it tunnels ALL device traffic including this
# script's sockets, so no special routing is needed.

import socket
import ssl
import sys
import time
import urllib.parse

TROJAN = (
    "trojan://humanity@172.64.152.23:443?path=%2Fassignment&security=tls"
    "&host=www.creationlong.org&type=ws&sni=www.creationlong.org#%40InfoTech_VK"
)
TIMEOUT = 8


def parse_trojan(uri):
    rest = uri.split("trojan://", 1)[1]
    # Strip the URI fragment (#tag) up front — it sits at the very end of
    # the query and otherwise contaminates the last param (e.g. sni).
    rest = rest.split("#", 1)[0]
    rest = rest.split("@", 1)[1] if "@" in rest else rest
    authority, _, query = rest.partition("?")
    host, _, port = authority.rpartition(":")
    port = int(port.split("/")[0]) if port else 443
    q = urllib.parse.parse_qs(query)
    ws_host = q.get("host", [host])[0]
    sni = q.get("sni", [ws_host])[0]
    path = urllib.parse.unquote(q.get("path", ["/"])[0])
    return {"ip": host, "port": port, "host": ws_host, "sni": sni, "path": path}


def classify_exc(e):
    s = str(e).lower()
    if isinstance(e, socket.timeout) or "timed out" in s:
        return "TIMEOUT (silent drop — DPI blackhole/SYN filter)"
    if "refused" in s:
        return "REFUSED (port closed — not censorship)"
    if "reset" in s or "broken pipe" in s or "aborted" in s:
        return "RST (active reset injected)"
    if "eof" in s or "closed" in s:
        return "RST-AFTER-CLIENTHELLO (SNI-based block)"
    return "ERROR: %s" % e


def probe_tcp(ip, port):
    t0 = time.time()
    try:
        s = socket.create_connection((ip, port), TIMEOUT)
        s.close()
        return "OK/connected", int((time.time() - t0) * 1000)
    except Exception as e:
        return classify_exc(e), int((time.time() - t0) * 1000)


def probe_tls(ip, port, sni):
    t0 = time.time()
    try:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        ctx.set_alpn_protocols(["h2", "http/1.1"])
        raw = socket.create_connection((ip, port), TIMEOUT)
        raw.settimeout(TIMEOUT)
        # server_hostname=None => no SNI sent (the "NO SNI" test).
        ss = ctx.wrap_socket(raw, server_hostname=sni if sni else None)
        cert = ""
        try:
            der = ss.getpeercert(binary_form=True) or b""
            cert = "".join(
                chr(c) if 32 <= c < 127 else "." for c in der
            )
            cert = "".join(
                p for p in cert.split(".") if len(p) >= 4 and
                ("." in p or "Inc" in p or "CA" in p)
            )[:120]
        except Exception:
            pass
        alpn = ss.selected_alpn_protocol() or "none"
        ss.close()
        return ("OK/tls-complete  alpn=%s  cert=%s" % (alpn, cert),
                int((time.time() - t0) * 1000))
    except Exception as e:
        return classify_exc(e), int((time.time() - t0) * 1000)


def probe_ws(t):
    t0 = time.time()
    try:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        ctx.set_alpn_protocols(["http/1.1"])
        raw = socket.create_connection((t["ip"], t["port"]), TIMEOUT)
        raw.settimeout(TIMEOUT)
        ss = ctx.wrap_socket(raw, server_hostname=t["sni"])
        req = (
            "GET %s HTTP/1.1\r\n"
            "Host: %s\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "User-Agent: Mozilla/5.0 (Linux; Android 13) Chrome/121\r\n\r\n"
            % (t["path"], t["host"])
        )
        ss.sendall(req.encode())
        data = ss.recv(512)
        ss.close()
        if not data:
            return "RST-AFTER-CLIENTHELLO (closed with no reply)", \
                int((time.time() - t0) * 1000)
        first = data.decode("latin1").splitlines()[0]
        return "HTTP: " + first, int((time.time() - t0) * 1000)
    except Exception as e:
        return classify_exc(e), int((time.time() - t0) * 1000)


def egress():
    try:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        raw = socket.create_connection(("1.1.1.1", 443), TIMEOUT)
        ss = ctx.wrap_socket(raw, server_hostname="one.one.one.one")
        ss.sendall(
            b"GET /cdn-cgi/trace HTTP/1.1\r\nHost: one.one.one.one\r\n"
            b"Connection: close\r\n\r\n"
        )
        buf = b""
        while len(buf) < 4096:
            chunk = ss.recv(1024)
            if not chunk:
                break
            buf += chunk
        ss.close()
        txt = buf.decode("latin1")
        ip = loc = "?"
        for ln in txt.splitlines():
            if ln.startswith("ip="):
                ip = ln[3:]
            if ln.startswith("loc="):
                loc = ln[4:]
        return "ip=%s loc=%s" % (ip, loc)
    except Exception as e:
        return "egress probe failed: %s" % e


def main():
    uri = sys.argv[1] if len(sys.argv) > 1 else TROJAN
    print("=" * 60)
    print("PACKET CONNECTION DIAGNOSTIC (termux)")
    print("Run with Psiphon ON and OFF, then diff the two reports.")
    print("=" * 60)

    print("\n[1] EGRESS IDENTITY  ->", egress())
    print("    Iranian loc = DIRECT.  Foreign loc = TUNNELLED (Psiphon).")

    t = parse_trojan(uri)
    print("\n[2] TARGET  ip=%s port=%s sni=%s host=%s path=%s"
          % (t["ip"], t["port"], t["sni"], t["host"], t["path"]))

    print("\n[3] TARGET REACHABILITY (layer by layer)")
    o, ms = probe_tcp(t["ip"], t["port"])
    print("  %-32s %6dms  %s" % ("TCP connect", ms, o))
    o, ms = probe_tls(t["ip"], t["port"], t["sni"])
    print("  %-32s %6dms  %s" % ("TLS + config SNI", ms, o))
    o, ms = probe_tls(t["ip"], t["port"], None)
    print("  %-32s %6dms  %s" % ("TLS + NO SNI", ms, o))
    o, ms = probe_tls(t["ip"], t["port"], "www.cloudflare.com")
    print("  %-32s %6dms  %s" % ("TLS + benign SNI", ms, o))
    o, ms = probe_ws(t)
    print("  %-32s %6dms  %s" % ("WebSocket upgrade", ms, o))
    print("  Interp: TCP-OK + config-SNI=RST but NO-SNI=OK -> SNI block.")
    print("          TCP=TIMEOUT -> IP blackhole.  All-OK -> config fine,")
    print("          failure is auth/path/origin (not Iran DPI).")

    print("\n[4] CONTROL BASELINE")
    for label, host in [("google", "www.google.com"),
                        ("cloudflare", "www.cloudflare.com"),
                        ("aparat (IR)", "www.aparat.com")]:
        try:
            ip = socket.gethostbyname(host)
        except Exception as e:
            print("  %-32s  DNS-FAIL %s" % (label, e))
            continue
        o, ms = probe_tls(ip, 443, host)
        print("  %-32s %6dms  %s" % (label, ms, o))

    print("\n[5] CLOUDFLARE POOL PROBE")
    for label, ip in [("104.16 premium", "104.16.0.1"),
                      ("104.18 premium", "104.18.0.1"),
                      ("104.21 free", "104.21.0.1"),
                      ("172.64 spectrum", "172.64.0.1"),
                      ("172.67 free", "172.67.0.1")]:
        o, ms = probe_tcp(ip, 443)
        print("  %-32s %6dms  %s" % (label, ms, o))

    print("\n" + "=" * 60)
    print("END — copy everything above")


if __name__ == "__main__":
    main()
