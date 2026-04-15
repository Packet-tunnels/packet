# PhantomTunnel Android

This Android target shares the same Rust tunnel core as iOS:

- Rust provides the tunnel engine and JNI bridge.
- Kotlin provides the app UI, saved configuration, logs, and Android VPN service shell.

## One-time setup

Install the Rust Android targets:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
```

Make sure these exist locally:

- Android SDK
- Android NDK
- Java 17
- Gradle 8.9 or a compatible wrapper

## Build the Rust Android libraries

```bash
./phantom-android/scripts/build-rust-android.sh
```

This copies `libphantom_client.so` into `app/src/main/jniLibs`.

## Build the debug APK

```bash
./phantom-android/scripts/build-apk.sh
```

APK output:

```bash
phantom-android/app/build/outputs/apk/debug/app-debug.apk
```

## Current scope

The Android app now:

- Saves the tunnel configuration locally.
- Requests Android VPN permission and starts a dedicated `VpnService` process.
- Runs the Rust SOCKS5 client inside that service process so disconnect can terminate it cleanly.
- Bridges the Android TUN interface into the local SOCKS5 listener with `tun2socks`.
- Routes device TCP traffic through the Android VPN once the tunnel is connected.
- Streams logs back into the app UI.

Current limitation:

- UDP-heavy traffic can still degrade until SOCKS UDP relay support is complete end-to-end.
