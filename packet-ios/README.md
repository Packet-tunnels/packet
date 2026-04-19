# Packet iOS

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
./packet-ios/scripts/build-rust-ios.sh
```

This produces:

- `target/aarch64-apple-ios/release/libphantom_client.a`
- `target/aarch64-apple-ios-sim/release/libphantom_client.a`
- `target/x86_64-apple-ios/release/libphantom_client.a`

## Generate the Xcode project

```bash
./packet-ios/scripts/generate-xcodeproj.sh
```

## Build the iOS app from the terminal

```bash
./packet-ios/scripts/build-app.sh
```

## Open in Xcode

```bash
open packet-ios/Packet.xcodeproj
```

## Current scope

The current iOS build installs a Packet Tunnel extension and creates an iOS VPN configuration.
Today it operates in proxy-routed mode through the packet tunnel's proxy settings, using the Rust client as the tunnel core.

It does **not** yet forward raw `packetFlow` traffic as a full-device VPN, so release copy and App Review Notes should describe the product accordingly.
