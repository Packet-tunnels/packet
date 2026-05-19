#!/usr/bin/env python3
# reach_probe.py — "Where can I host the obfs server?" discovery probe
#
# The obfs transport defeats Iran's "RST any foreign TLS handshake" filter,
# but Iran ALSO blackholes most foreign IPs at the TCP layer. So we must
# find a foreign IP that Iran does NOT blackhole, and host the obfs server
# directly there (no TLS-terminating CDN in front).
#
# This script, run from the Iran phone (Termux), TCP-probes a spread of
# foreign endpoints across many providers / ASNs / regions / ports and
# classifies each:
#
#   OPEN              SYN/ACK came back  → usable hosting candidate
#   TIMEOUT           silent drop        → Iran blackholes this (unusable)
#   RST               active reset       → reachable but something resets TCP
#
# WORKFLOW
#   1. (Best) Rent 3-5 cheap VPSes in different providers/regions. On each,
#      open a few ports with netcat so SYNs get answered:
#        for p in 443 8443 2083 9443 36571; do (nc -lk -p $p &) ; done
#      Then add their IPs to MY_VPS below as ("ip", "label").
#   2. pkg install python ; run:  python reach_probe.py
#   3. Whichever MY_VPS line shows OPEN on a port → host phantom-server's
#      --obfs-port there on that port, point the client's --cdn-edge at
#      ip:port with --transport obfs.
#
# The BASELINE list uses KNOWN-LIVE public IPs (public DNS resolvers, big
# anycast services) per provider so a TIMEOUT means censorship, not a dead
# host. It tells you which PROVIDERS/ASNs are reachable at all, so you know
# where renting a VPS is even worth trying.

import socket
import sys
import time

TIMEOUT = 6

# Your rented VPSes — EDIT THIS. ("ip", "label", [ports])
MY_VPS = [
    # ("203.0.113.10", "hetzner-fsn1", [443, 8443, 2083, 9443, 36571]),
    # ("198.51.100.20", "ovh-gra", [443, 8443, 2083, 9443, 36571]),
]

# Known-live anycast / public endpoints, grouped by operator. A TIMEOUT
# here is censorship (these are always up), so this maps which networks
# Iran lets you reach at all.
BASELINE = [
    ("1.1.1.1", 443, "Cloudflare-anycast"),
    ("8.8.8.8", 443, "Google-DNS/anycast"),
    ("9.9.9.9", 443, "Quad9-anycast"),
    ("208.67.222.222", 443, "OpenDNS/Cisco"),
    ("94.140.14.14", 443, "AdGuard-DNS"),
    ("45.90.28.0", 443, "NextDNS-anycast"),
    ("185.228.168.9", 443, "CleanBrowsing"),
    ("76.76.2.0", 443, "ControlD-anycast"),
    ("216.239.32.10", 443, "Google-NS"),
    ("199.9.14.201", 443, "b-root-DNS"),
    ("193.0.14.129", 443, "k-root-RIPE"),
    ("185.43.135.1", 443, "Quad101-TW"),
    ("80.80.80.80", 443, "Freenom-DNS"),
    ("84.200.69.80", 443, "DNS.WATCH-DE"),
    # Datacenter probes — public services in big VPS ASNs (often live on 443)
    ("5.161.0.1", 443, "Hetzner-US-ASH"),
    ("65.108.0.1", 443, "Hetzner-FI-HEL"),
    ("51.38.0.1", 443, "OVH-EU"),
    ("139.99.0.1", 443, "OVH-APAC"),
    ("45.32.0.1", 443, "Vultr"),
    ("139.180.128.1", 443, "Vultr-2"),
    ("172.105.0.1", 443, "Linode/Akamai"),
    ("45.76.0.1", 443, "Choopa/Vultr"),
    ("141.95.0.1", 443, "OVH-Scale"),
    ("194.5.0.1", 443, "Aeza/various"),
    ("38.0.0.1", 443, "Cogent"),
    ("162.159.200.1", 443, "Cloudflare-WARP"),
]

# Ports to also try on the baseline anycast that reliably listen on 443.
EXTRA_PORTS_FOR = {"1.1.1.1", "8.8.8.8", "9.9.9.9"}
EXTRA_PORTS = [853, 443, 80]


def probe(ip, port):
    t0 = time.time()
    s = socket.socket()
    s.settimeout(TIMEOUT)
    try:
        s.connect((ip, port))
        s.close()
        return "OPEN", int((time.time() - t0) * 1000)
    except socket.timeout:
        return "TIMEOUT (blackholed by Iran)", int((time.time() - t0) * 1000)
    except ConnectionRefusedError:
        return "REFUSED (reachable, port closed)", int((time.time() - t0) * 1000)
    except OSError as e:
        msg = str(e).lower()
        if "reset" in msg or "abort" in msg:
            return "RST (reachable, reset)", int((time.time() - t0) * 1000)
        return "ERR: %s" % e, int((time.time() - t0) * 1000)
    finally:
        try:
            s.close()
        except Exception:
            pass


def line(label, ip, port, res, ms):
    print("  %-26s %-16s :%-5d %6dms  %s" % (label, ip, port, ms, res))


def main():
    print("=" * 70)
    print("REACH PROBE — which foreign IPs can host the obfs server?")
    print("Run from the Iran phone. OPEN = usable. TIMEOUT = Iran blackhole.")
    print("=" * 70)

    if MY_VPS:
        print("\n[A] YOUR RENTED VPSes (the decisive ones)")
        for ip, label, ports in MY_VPS:
            for p in ports:
                r, ms = probe(ip, p)
                line(label, ip, p, r, ms)
    else:
        print("\n[A] YOUR RENTED VPSes — none configured.")
        print("    Edit MY_VPS at the top once you rent some; rerun.")

    print("\n[B] BASELINE — which networks/ASNs Iran lets you reach at all")
    open_nets = []
    for ip, port, label in BASELINE:
        r, ms = probe(ip, port)
        if r == "OPEN":
            open_nets.append(label)
        line(label, ip, port, r, ms)
        if ip in EXTRA_PORTS_FOR:
            for ep in EXTRA_PORTS:
                if ep == port:
                    continue
                r2, ms2 = probe(ip, ep)
                line(label + "*", ip, ep, r2, ms2)

    print("\n" + "=" * 70)
    if open_nets:
        print("REACHABLE networks (rent a VPS in one of these ASNs/regions):")
        for n in open_nets:
            print("  • " + n)
    else:
        print("NOTHING reachable on :443 — Iran is in a hard-blackhole window.")
        print("Re-run later; reachability often changes hour to hour.")
    print("Next: rent a VPS in a reachable ASN, run phantom-server with")
    print("--obfs-port <p>, point client --transport obfs --cdn-edge ip:p")
    print("=" * 70)


if __name__ == "__main__":
    sys.exit(main())
