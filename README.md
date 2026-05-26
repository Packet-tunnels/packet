# Packet

> **A privacy-first internet tunnel designed to overcome network censorship through protocol obfuscation and CDN routing.**

A custom, protocol-level internet tunnel built in Rust for anonymous, uncensored access.

## Features

- **Zero-Knowledge Architecture:** End-to-end encryption with XChaCha20-Poly1305
- **Protocol Obfuscation:** Disguises traffic as normal HTTPS and WebSocket connections
- **CDN Bypass:** Routes through legitimate content delivery networks to bypass DPI inspection
- **Multiple Transports:** Supports WebSocket, HTTP polling, QUIC, and stealth HTTPS
- **Cross-Platform:** Native clients for Android (Kotlin), iOS (Swift), and Rust CLI
- **Probe Resistance:** Unauthenticated visitors see a legitimate decoy website

## Architecture

```
packet/
├── packet-proto/     # Shared encryption and framing (XChaCha20-Poly1305)
├── packet-server/    # Server-side tunnel endpoint
├── packet-client/    # SOCKS5 local proxy client
├── packet-bridge/    # Optional domestic relay for censored networks
├── packet-relay/     # Optional exit node for unfiltered internet access
└── static/           # Decoy website content (appears to legitimate traffic)
```

## Building

### Prerequisites
- Rust 1.70+
- Standard build tools (gcc, pkg-config, libssl-dev)

### Compile

```bash
# Server (runs on VPS)
cargo build --release -p phantom-server

# Client (runs locally)
cargo build --release -p phantom-client

# Optional: Bridge for domestic relay
cargo build --release -p phantom-bridge
```

## Deployment

### Server Setup

For a VPS deployment with automatic systemd service management:

```bash
chmod +x deploy-vps.sh
sudo ./deploy-vps.sh
```

The script will:
1. Generate cryptographic secrets
2. Install dependencies
3. Compile the server in release mode
4. Configure systemd for automatic restart
5. Output client configuration needed for mobile apps

### Client Configuration

Configure the client using the output from `deploy-vps.sh`. Clients need:
- Server URL (obtained from deployment)
- Shared secret (generated during deployment)
- Transport mode (WebSocket, HTTP, QUIC, or Stealth)
- CDN edge IP (if using CDN bypass)

## Transport Modes

| Mode | Use Case |
|------|----------|
| **WebSocket** | CDN bypass, persistent connections |
| **HTTP** | Direct connections, stateless fallback |
| **QUIC** | UDP escape for protocol-based blocking |
| **Stealth** | Browser-profile HTTPS for advanced DPI |

## Security

- **Encryption:** XChaCha20-Poly1305 (AEAD cipher)
- **Authentication:** Pre-shared key with HMAC-SHA256
- **Probe Resistance:** Unauthenticated traffic receives valid website responses
- **No Fingerprint:** Custom protocol not in standard DPI databases
- **Traffic Padding:** Fixed-size blocks prevent size-based analysis
- **CDN Integration:** Legitimate infrastructure provides cover traffic

## Documentation

- **Deployment:** See `deploy-vps.sh` for VPS setup
- **Android Build:** See `packet-android/` for native compilation
- **iOS Build:** See `packet-ios/` for native compilation
- **Protocol:** See `packet-proto/` for encryption and framing details

## Legal & Support

### Page URLs (for iOS/Android apps)

Once GitHub Pages is enabled:

- **Privacy Policy:** https://packet-tunnels.github.io/packet/privacy.html
- **Terms of Use:** https://packet-tunnels.github.io/packet/terms.html
- **Support:** https://packet-tunnels.github.io/packet/support.html
- **Contact Email:** support@packet-tunnels.app

### Enable GitHub Pages

To activate these pages:

1. Go to **Settings** → **Pages**
2. Under "Build and deployment":
   - Source: **Deploy from a branch**
   - Branch: **main**
   - Folder: **/docs**
3. Click **Save**

Pages will be live in 1-2 minutes. Add the above URLs to your iOS/Android app settings.

## License

This project is provided as-is for research and educational purposes.

## Security Notice

This software is intended for legitimate privacy and circumvention use in regions with network censorship. Users are responsible for understanding their local laws and regulations.
