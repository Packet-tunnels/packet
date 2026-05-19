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
- Settings ordered for review: configuration summary first, public legal links second, About last.
- Public HTML pages for privacy policy, terms of use, and support in `packet-public`.
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

Suggested public URLs for App Store Connect once GitHub Pages is enabled on `Packet-tunnels/packet-public`:

- Support URL: `https://packet-tunnels.github.io/packet-public/`
- Privacy Policy URL: `https://packet-tunnels.github.io/packet-public/privacy.html`
- Terms URL used in app: `https://packet-tunnels.github.io/packet-public/terms.html`
- Support page used in app: `https://packet-tunnels.github.io/packet-public/support.html`

Why these are acceptable:

- The homepage already includes a support section and contact email.
- The privacy page already includes a standalone privacy policy and support contact.

GitHub Pages setup for `packet-public`:

1. Push the `packet-public` repository to GitHub.
2. Open the `Packet-tunnels/packet-public` repository on GitHub.
3. Go to `Settings` → `Pages`.
4. Set `Build and deployment` → `Source` to `Deploy from a branch`.
5. Select branch `main` and folder `/(root)`.
6. Save and wait for Pages to publish.
7. Open the homepage and privacy URLs above and verify they load publicly over HTTPS.

Minimum TestFlight sequence:

1. Regenerate the Xcode project with XcodeGen after any file split or source change.
2. Build the Rust iOS static libraries.
3. Archive the `Packet` app in Xcode using a distribution signing identity that includes the Network Extension capability for both targets.
4. Upload the archive to App Store Connect.
5. In App Store Connect, set the Support URL and Privacy Policy URL.
6. Complete App Privacy answers and publish them.
7. Complete export compliance for the uploaded build if prompted.
8. Add TestFlight test information, including the feedback email and beta description.
9. Create an internal testing group and add the uploaded build.
10. Only after internal validation, create an external testing group if needed.
