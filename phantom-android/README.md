# PhantomTunnel Android

This Android target is the same simple model as iOS:

- Rust provides the tunnel engine and JNI bridge.
- Kotlin provides a small test app with configuration fields and live logs.

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

The Android app is a test harness for the Rust client and log output.
It does not yet integrate with Android's full-device `VpnService`.
