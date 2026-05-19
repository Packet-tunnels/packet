# ADB Psiphon / Packet Diagnostics

Date: 2026-05-19 MDT / 2026-05-20 IRST

## Device

- Model: `21061110AG` / `chopin`
- Android package for Psiphon: `com.psiphon3.subscription`
- Android package for Packet: `com.resolo.packet`
- Active mobile network: LTE `mtnirancell`
- Active interface observed by Android: `ccmni0`
- Mobile IP observed by Android: `22.187.6.135/32`
- Android DNS servers: `10.255.255.254`, `10.10.10.10`
- Carrier HTTP proxy advertised by Android: `10.131.26.138:8080`

## ADB Setup

ADB was not installed on the workstation, so a user-space Android platform tools package was extracted under:

```text
/tmp/adb-deb/usr/lib/android-sdk/platform-tools/adb
```

The working invocation is:

```bash
/usr/bin/env LD_LIBRARY_PATH=/tmp/adb-deb/usr/lib/x86_64-linux-gnu:/tmp/adb-deb/usr/lib/x86_64-linux-gnu/android /tmp/adb-deb/usr/lib/android-sdk/platform-tools/adb devices -l
```

The device repeatedly re-enumerated over USB. When it appeared as `no permissions`, the host needed the current `/dev/bus/usb/001/...` node permissions fixed.

## Psiphon Capture Attempt

Commands used:

```bash
adb shell am force-stop com.resolo.packet
adb shell am force-stop com.psiphon3.subscription
adb logcat -c
adb shell monkey -p com.psiphon3.subscription -c android.intent.category.LAUNCHER 1
adb shell input tap 810 2084
adb shell dumpsys connectivity
adb logcat -d --pid=<psiphon-pid>
```

Psiphon launched and focused as:

```text
com.psiphon3.subscription/com.psiphon3.MainActivity
```

The connect button was found at approximately:

```text
resource-id=com.psiphon3.subscription:id/start_button_container bounds=[540,2018][1080,2150]
center=(810,2084)
```

MIUI blocked ADB input injection:

```text
SecurityException: Injecting input events requires the caller ... android.permission.INJECT_EVENTS
```

Direct service start was also blocked because Psiphon's VPN service is not exported:

```text
Error: Requires permission not exported from uid 10285
```

When ADB connectivity was available, Android connectivity still showed only the LTE network. No VPN network was observed:

```text
Active default network: 252
NetworkAgentInfo{network{252} ni{MOBILE[LTE] CONNECTED extra: mtnirancell}
Transports: CELLULAR ... NOT_VPN ...
```

After the screen timed out, `uiautomator dump` showed the keyguard overlay instead of the Psiphon UI:

```xml
<hierarchy>
  <node package="com.android.systemui" resource-id="com.android.systemui:id/keyguard_message_area_container" />
</hierarchy>
```

Psiphon-only logcat after launch did not include a tunnel connection attempt. It only showed app/UI startup and billing/update failures, for example:

```text
PlayCore AppUpdateService : requestUpdateInfo(com.psiphon3.subscription)
BillingClient: getSkuDetails() failed for queryProductDetailsAsync. Response code: 12
```

Conclusion: no Psiphon escape path was captured from ADB in this run because MIUI blocked both automated tapping and direct service start, and the phone later locked. The only network-side fact captured before that blocker is the active IranCell LTE network plus its advertised HTTP proxy.

## Packet ADB Attempt

Commands used:

```bash
adb shell am force-stop com.psiphon3.subscription
adb shell am start -n com.resolo.packet/.MainActivity
adb shell am start-foreground-service -n com.resolo.packet/.TunnelVpnService -a com.resolo.packet.action.CONNECT
adb logcat -d --pid=<packet-pid>
```

Packet launched successfully:

```text
Starting: Intent { cmp=com.resolo.packet/.MainActivity }
```

Direct ADB service start was blocked because the VPN service is intentionally non-exported:

```text
Starting service: Intent { act=com.resolo.packet.action.CONNECT cmp=com.resolo.packet/.TunnelVpnService }
Error: Requires permission not exported from uid 10302
```

Packet-only logcat showed app startup and native library load, not a tunnel attempt:

```text
Load ... libphantom_client.so ... ok
```

Conclusion: ADB could launch the Packet UI, but could not start the VPN/tunnel path without UI interaction or an exported debug entry point.

## Code Change Applied

The active TLS fragmentation path already matches the v2rayNG/Hiddify-style `tlshello` behavior in:

```text
packet-client/src/tls_fragment.rs
```

The change was therefore kept to the TLS fingerprint path:

- Enable BoringSSL extension permutation.
- Enable OCSP stapling and signed certificate timestamp extensions.
- Pin Chrome-style supported-group ordering.
- Mark the built-in Packet Chain Trojan URI with `fp=chrome`.

This targets the observed symptom: v2ray/Psiphon survives the TLS handshake while Packet's BoringSSL ClientHello was still too static.

## Follow-Up Capture Requirements

To get a real Psiphon escape log on this MIUI device, one of these must be true:

- Manually unlock the phone, tap Psiphon Connect, and keep the screen on while ADB polls `dumpsys connectivity` and `logcat`.
- Enable MIUI Developer Options -> `USB debugging (Security settings)`, then ADB `input tap` should work.
- Install a debug build with an exported diagnostic-only start action for Packet, so ADB can start the tunnel without UI tapping.
