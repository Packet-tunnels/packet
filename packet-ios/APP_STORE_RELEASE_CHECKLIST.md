# App Store Release Checklist

Checked against Apple documentation on April 18, 2026.

Primary Apple sources reviewed:

- App Review Guidelines 5.4 VPN Apps: https://developer.apple.com/app-store/review/guidelines/
- App Review Guidelines 2.1(a) App Completeness: https://developer.apple.com/app-store/review/guidelines/
- Manage app privacy: https://developer.apple.com/help/app-store-connect/manage-app-information/manage-app-privacy
- Support URL reference: https://developer.apple.com/help/app-store-connect/reference/app-review-information
- Overview of export compliance: https://developer.apple.com/help/app-store-connect/manage-app-information/overview-of-export-compliance/
- Export compliance documentation for encryption: https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption/
- `ITSAppUsesNonExemptEncryption`: https://developer.apple.com/documentation/BundleResources/Information-Property-List/ITSAppUsesNonExemptEncryption

Implemented in this repo:

- Dedicated in-app VPN disclosure before first connect, with explicit local-storage, traffic-handling, and tracking statements.
- Re-openable privacy and security summary in Settings.
- Privacy manifests bundled for both the app and the packet tunnel extension.
- App-level export-compliance declaration in the generated Info.plist settings.
- Network Extension entitlements already present for the app and extension.
- Explicit app version and build settings in `project.yml`.

Manual items still required in App Store Connect or during review:

- Submit the app from an Apple Developer organization account for VPN distribution.
- Add a valid privacy policy URL. Apple requires this for iOS apps.
- Add a valid support URL with real contact information.
- Publish App Privacy answers in App Store Connect to match the app's actual behavior.
- Complete export compliance for the app's custom tunnel encryption. If distributing in France, include the French encryption declaration required by Apple.
- Include reviewer instructions, plus a reachable test server or demo configuration, in App Review Notes.
- If any territory requires a VPN license, include the license details in App Review Notes.
- Verify the production signing profile includes the Network Extension capability for both targets.

Known product caveat:

- The current iOS implementation uses a packet tunnel extension in proxy-routed mode. Keep the product description and App Review Notes accurate; do not describe it as a full raw-packet VPN unless packetFlow routing is implemented.
