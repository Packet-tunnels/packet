# PhantomTunnel iOS

This is the simplest workable setup for this repo:

- Rust keeps the tunnel engine and FFI surface.
- Swift keeps the iOS app shell and Apple-native integration.
- XcodeGen generates the `.xcodeproj` from [`project.yml`](./project.yml).

Using Flutter or Tauri would not remove the native iOS work here, because system routing and VPN-style integration still must go through Apple's native APIs.

## One-time setup

Install XcodeGen:

```bash
brew install xcodegen
```

Install the Rust iOS targets:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
```

## Build the Rust libraries

```bash
./phantom-ios/scripts/build-rust-ios.sh
```

This produces:

- `target/aarch64-apple-ios/release/libphantom_client.a`
- `target/aarch64-apple-ios-sim/release/libphantom_client.a`
- `target/x86_64-apple-ios/release/libphantom_client.a`

## Generate the Xcode project

```bash
./phantom-ios/scripts/generate-xcodeproj.sh
```

## Build the iOS app from the terminal

```bash
./phantom-ios/scripts/build-app.sh
```

## Open in Xcode

```bash
open phantom-ios/PhantomTunnel.xcodeproj
```

## Current scope

The iOS target now includes a `PacketTunnel` extension and starts the Rust client from inside the extension.

Current behavior:

- The Rust core still exposes a local SOCKS5 proxy and keeps the WebSocket/CDN bypass architecture unchanged.
- The Packet Tunnel installs a PAC-based proxy configuration that points HTTP/HTTPS traffic at that local SOCKS5 listener.
- The SwiftUI app shows live tunnel logs plus connection telemetry such as endpoint, ping, upload/download totals, throughput, and active stream counts.

Current limitation:

- The extension does **not** yet consume `packetFlow` packets directly, so this is not a full TUN-to-socket implementation. The iOS path is proxy-routed rather than raw packet forwarding.
