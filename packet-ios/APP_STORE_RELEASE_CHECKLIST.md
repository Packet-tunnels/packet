# App Store Release Checklist

Items handled in this repo:

- Dedicated in-app VPN data-use disclosure before first connect.
- Privacy manifests bundled for both the app and the packet tunnel extension.
- Network Extension entitlements already present for the app and extension.
- Explicit app version/build settings in `project.yml`.

Manual items still required in App Store Connect or during review:

- Submit the app from an Apple Developer organization account for VPN distribution.
- Add a valid privacy policy URL and support URL.
- Complete App Privacy answers in App Store Connect to match the app's real behavior.
- Answer export compliance questions for the app's encryption usage.
- Include demo credentials, a reachable test server, or clear reviewer instructions in App Review Notes.
- If any territory requires a VPN license, include the license details in App Review Notes.
- Verify the production signing profile includes the Network Extension capability for both targets.
