# App Store Review Notes Template

Use this when submitting the iOS build for TestFlight external review or App Review.

## Reviewer Access

- Test server URL:
- Shared secret:
- Transport mode:
- Optional CDN edge / host override:

## How To Verify

1. Launch Packet.
2. Open Settings and select the provided server profile.
3. Review and accept the in-app VPN disclosure.
4. Tap Connect on the Overview tab.
5. Confirm the status changes to Connected and traffic metrics begin updating.

## Data Handling Summary

- Packet stores the server URL, shared secret, and disclosure acknowledgement locally on-device.
- The configured tunnel server receives the traffic and connection metadata required to operate the tunnel.
- Packet does not bundle ads, analytics, or tracking SDKs.

## Compliance Notes

- App category: VPN / security utility using `NETunnelProviderManager`.
- Export compliance: the app uses app-managed tunnel encryption in addition to Apple platform networking.
- If distributing in a territory that requires a VPN license, include the license details here.

## Support

- Support URL:
- Privacy Policy URL:
- Contact email:
