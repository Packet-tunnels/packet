# Phantom Tunnel

A custom, protocol-level internet tunnel built in Rust. Designed from scratch — not based on VLESS, Shadowsocks, or any existing proxy protocol.

**v2: Now with WebSocket transport and CDN bypass for total internet blockouts.**

## How It Works

Unlike VPNs and standard proxies that use recognizable protocols (which get fingerprinted and blocked), Phantom Tunnel hides data inside **normal web traffic**:

### Direct Mode
```
Browser → SOCKS5 (local :1080) → Phantom Client → HTTP POST → Phantom Server → Internet
```

### CDN Bypass Mode (for censored networks like Iran)
```
Browser → SOCKS5 → Phantom Client → WebSocket → CDN Edge (domestic IP:80) → CDN Forward → Phantom Server → Internet
```

To DPI/censors, CDN bypass traffic looks like a normal web application connecting to a domestic website's real-time feature. No unusual protocols, no foreign IPs visible.

## Architecture

```
phantom-tunnel/
├── phantom-proto/     # Shared: encryption (XChaCha20-Poly1305), framing, auth, padding
├── phantom-server/    # VPS: HTTP server + WebSocket tunnel + real website
├── phantom-client/    # Local: SOCKS5 proxy + WebSocket/HTTP tunnel client
├── phantom-bridge/    # Domestic relay: transparent TCP forwarder for censored networks
├── phantom-relay/     # Exit relay: outbound Starlink / unfiltered internet hop
└── static/            # Camouflage website content (piano lessons)
```

## Build

```bash
# Server (on VPS)
cargo build --release -p phantom-server

# Client (on local machine)
cargo build --release -p phantom-client

# Bridge (on domestic VPS, if needed)
cargo build --release -p phantom-bridge
```

## Usage

### Server
```bash
./phantom-server --port 80 --secret "your-shared-secret"
```

### Client (Direct — outside censored networks)
```bash
./phantom-client \
  --server http://35.222.22.49 \
  --secret "your-shared-secret" \
  --listen 127.0.0.1:1080
```

### Client (CDN Bypass — for Iran/censored networks)
```bash
# Connect through ArvanCloud CDN edge (domestic IP, whitelisted by DPI)
./phantom-client \
  --server http://piano-lessons.site \
  --secret "your-shared-secret" \
  --listen 127.0.0.1:1080 \
  --transport ws \
  --cdn-edge "185.143.234.235:80" \
  --host "piano-lessons.site"
```

### Client (TLS Fronting with custom SNI)
```bash
# Use this when the reachable ingress, the Host header, and the visible SNI
# need to be different.
./phantom-client \
  --server https://piano-lessons.site \
  --secret "your-shared-secret" \
  --listen 127.0.0.1:1080 \
  --transport ws \
  --cdn-edge "104.16.132.229:443" \
  --host "piano-lessons.site" \
  --sni "allowed-site.ir"
```

### Bridge (on domestic VPS)
```bash
# Run on a VPS inside the censored country
./phantom-bridge --listen 0.0.0.0:80 --upstream 35.222.22.49:80

# Then point the client to the bridge instead
./phantom-client \
  --server http://domestic-vps-ip \
  --secret "your-shared-secret" \
  --transport ws
```

### Android (via Termux)
```bash
# In Termux
wget https://your-release-url/phantom-client-android
chmod +x phantom-client-android
./phantom-client-android \
  -S http://piano-lessons.site \
  -s "your-secret" \
  -l 127.0.0.1:1080 \
  --transport ws \
  --cdn-edge "185.143.234.235:80" \
  --host "piano-lessons.site"

# Then: Android Settings → WiFi → Proxy → Manual → localhost:1080
```

## Transport Modes

| Mode | Flag | Use Case |
|------|------|----------|
| **WebSocket** | `--transport ws` | CDN bypass, persistent connection, recommended for Iran |
| **HTTP Polling** | `--transport http` | Direct connections, fallback mode |
| **Auto** | `--transport auto` | Tries WebSocket first, falls back to HTTP (default) |

## CDN Bypass Setup (ArvanCloud)

1. **Buy domain**: e.g., `piano-lessons.site`
2. **Add to ArvanCloud**: Point domain to your server's IP
3. **Enable WebSocket**: In ArvanCloud dashboard, enable WebSocket forwarding
4. **Note edge IPs**: ArvanCloud assigns domestic edge IPs to your domain
5. **Configure client**: Use `--cdn-edge` with the ArvanCloud edge IP

## Iran Connectivity Playbook

1. Treat direct foreign-IP access as unavailable from Iran.
2. Prefer a reachable ingress first:
   - a domestic bridge VPS running `phantom-bridge`, or
   - a CDN/fronted hostname once the provider is active.
3. Keep `phantom-relay` as an exit node only. It does not replace the need for a reachable ingress.
4. If Arvan still shows the domain as `pending`, do not block rollout on it. Use the domestic bridge first and switch to CDN later.

Recommended path during heavy blockout:

```text
Android client in Iran -> domestic bridge or active CDN edge -> phantom-server -> phantom-relay (optional exit)
```

## Deploy to VPS
```bash
chmod +x deploy.sh
./deploy.sh 35.222.22.49 "your-shared-secret"
```

## Security
- **Encryption:** XChaCha20-Poly1305 (all tunnel data encrypted end-to-end)
- **Authentication:** HMAC-SHA256 with pre-shared key
- **Probe resistance:** Unauthenticated visitors see a real website
- **No fingerprint:** Custom protocol — not in any DPI signature database
- **Traffic padding:** Messages padded to fixed block sizes to prevent analysis
- **CDN cover:** Traffic routes through legitimate domestic CDN infrastructure
