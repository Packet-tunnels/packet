# Phantom Tunnel

A custom, protocol-level internet tunnel built in Rust. Designed from scratch — not based on VLESS, Shadowsocks, or any existing proxy protocol.

## How It Works

Unlike VPNs and standard proxies that use recognizable protocols (which get fingerprinted and blocked), Phantom Tunnel hides data inside **normal HTTP API calls**:

```
Browser → SOCKS5 (local :1080) → Phantom Client → HTTP POST requests → CDN → Phantom Server → Internet
```

To network observers, traffic looks like a web application making API calls to a piano lessons website. No WebSocket upgrades, no long-lived connections, no unusual protocols.

## Architecture

```
phantom-tunnel/
├── phantom-proto/     # Shared: encryption (XChaCha20-Poly1305), framing, auth
├── phantom-server/    # VPS: HTTP server with covert tunnel + real website
├── phantom-client/    # Local: SOCKS5 proxy + HTTP tunnel client
└── static/            # Camouflage website content
```

## Build

```bash
# Server (on VPS)
cargo build --release -p phantom-server

# Client (on local machine)
cargo build --release -p phantom-client
```

## Usage

### Server
```bash
./phantom-server --port 80 --secret "your-shared-secret"
```

### Client
```bash
./phantom-client \
  --server https://piano-lessons.site \
  --secret "your-shared-secret" \
  --listen 127.0.0.1:1080
```

Then configure your browser/apps to use SOCKS5 proxy at `127.0.0.1:1080`.

### Android (via Termux)
```bash
# In Termux
wget https://your-release-url/phantom-client-android
chmod +x phantom-client-android
./phantom-client-android -S https://piano-lessons.site -s "your-secret" -l 127.0.0.1:1080

# Then: Android Settings → WiFi → Proxy → Manual → localhost:1080
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
