# macOS direct-download distribution

## Current preparation

The next Community version is 1.0.4. Keep the existing release tags unchanged. The user-facing Release notes live in [releases/v1.0.4.md](releases/v1.0.4.md); download and installation text must be finalized against the actual installer before publication. Developer verification belongs here or in CI, not in the user-facing Release notes.

On 2026-09-18, the system keychain exposed no valid code-signing identity. Xcode's notarytool and stapler are available. The 1.0.4 build cannot be described as Developer ID signed, notarized, or Apple-verified unless a valid identity and notarization credentials become available before publication.

## Interim Apple Development build

Use the existing valid Apple Development identity from the signing Mac's keychain. Keep its personal name and identifier out of tracked configuration:

```sh
env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
  -u APPLE_API_ISSUER -u APPLE_API_KEY -u APPLE_API_KEY_PATH \
  APPLE_SIGNING_IDENTITY='Apple Development: YOUR NAME (IDENTIFIER)' \
  npm run tauri:build -- --target aarch64-apple-darwin --bundles dmg
```

This deliberately omits notarization credentials. The private key stays in Keychain; approve a signing-key access prompt locally if macOS requests it. Do not commit the certificate/private key or account credentials.

Check the artifact's version, architecture, signature integrity and DMG integrity using the commands below. For this interim build, the expected authority is Apple Development. Gatekeeper rejection and the absence of a notarization ticket are expected and must be disclosed; they are not successful public-distribution verification. Do not replace the development signature with ad-hoc signing silently if it fails.

Before publication, test the downloaded artifact on an isolated Mac/account and document the actual first-launch behavior. macOS may require an explicit exception in System Settings → Privacy & Security → Open Anyway. Do not claim this works until tested, and do not instruct users to disable Gatekeeper globally. Certificate validity and provisioning restrictions can affect whether a development-signed app runs on another Mac.

The following Developer ID instructions remain the upgrade path when membership is available.

## Obtain the correct certificate

1. Enroll in the paid Apple Developer Program if needed. Use the account holder or an appropriately authorized team workflow to create a **Developer ID Application** certificate for distribution outside the Mac App Store. A Developer ID Installer certificate is for installer packages, not a substitute for signing the app inside a DMG.
2. On the signing Mac, create a Certificate Signing Request with Keychain Access. Keep the generated private key on that Mac. Upload the CSR through Apple Developer → Certificates, Identifiers & Profiles → Certificates, choosing Developer ID Application.
3. Download and open the issued `.cer` file in the login keychain. It must be paired with the private key that created the CSR. If a certificate was created on another Mac, importing the certificate alone is insufficient; use the team's approved certificate/private-key transfer process.
4. Confirm the identity is valid:

```sh
security find-identity -v -p codesigning
```

Do not export private keys into this repository, send passwords in chat, or commit Apple credentials. Signing certificate creation and Apple account setup are performed by the account holder.

## Configure local Tauri signing and notarization

The tracked Tauri configuration enables hardened runtime. Keep the account-specific identity in the build environment, not source control:

```sh
export APPLE_SIGNING_IDENTITY='Developer ID Application: YOUR LEGAL NAME (TEAMID)'
```

Replace this example with the exact valid identity reported by Keychain. Never use `Apple Development` or `-` as the public Developer ID identity.

Choose one notarization credential method supported by Tauri:

- **Apple account:** set `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_PASSWORD`. The password must be an **app-specific password**, not the normal account password. Enter it through a secure local secret mechanism, not a literal command retained in shell history.
- **App Store Connect team API key:** set `APPLE_API_ISSUER`, `APPLE_API_KEY` (Key ID), and `APPLE_API_KEY_PATH` (the local `.p8` file). Keep the private key outside the repository. Grant only the access required by the signing workflow.

Do not configure both methods at once. Signing identifies the publisher; notarization is a separate Apple check. A valid signature alone is not evidence of notarization.

After both the signing identity and notarization credentials are available, build from the agreed, clean release commit on the signing Mac:

```sh
npm ci
npm run tauri:build -- --target aarch64-apple-darwin --bundles dmg
```

The explicit target requires the `aarch64-apple-darwin` Rust target. Tauri signs and submits to Apple when the relevant credentials are present. The expected output is:

```text
src-tauri/target/aarch64-apple-darwin/release/bundle/macos/LumeTrace.app
src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/LumeTrace_1.0.4_aarch64.dmg
```

Confirm actual output paths. Do not rename an older-version binary to make it look like 1.0.4. Do not install development builds into `/Applications` or launch automated acceptance against the user's live workspace.

## Verify the final artifacts

Run these against the freshly built paths, checking exit codes and output:

```sh
codesign --verify --deep --strict --verbose=2 'path/to/LumeTrace.app'
codesign -dv --verbose=4 'path/to/LumeTrace.app'
xcrun stapler validate 'path/to/LumeTrace.app'
spctl --assess --type execute --verbose=4 'path/to/LumeTrace.app'
hdiutil verify 'path/to/LumeTrace_1.0.4_aarch64.dmg'
shasum -a 256 'path/to/LumeTrace_1.0.4_aarch64.dmg'
```

For the Developer ID distribution path, the signature must show Developer ID Application and hardened runtime. Check Apple's notarization result and the stapled app ticket; do not infer success from the presence of a DMG. If the DMG itself is separately notarized/stapled, validate its ticket too. Inspect the app version, arm64 architecture, bundle identity, and actual minimum macOS version.

On an isolated test account/Mac, verify the downloaded DMG opens and the app can be copied and launched. For the Developer ID path, also verify macOS recognizes its verified publisher without disabling security protections; for the interim development build, record the actual warning and required exception instead. Test with synthetic files. Include checks for preview, saving, file history, search, and the new Help Center. Signed release execution and bundled runtime dependencies require this acceptance even after debug and unit tests pass.

## Prepare the GitHub Release

- Retain existing release tags unchanged. Agree on the exact v1.0.4 commit and tag before creating a new tag. Do not switch or merge main automatically.
- Prepare a draft Release with [v1.0.4 notes](releases/v1.0.4.md), the verified DMG, and its SHA-256 checksum. Verify the artifact name matches Download.
- Replace the Installation placeholder with the actual tested installation and first-launch steps. State the verified minimum macOS version. Do not advertise signing or notarization if either is unfinished.
- Obtain final publication approval after the concrete draft and installer are reviewable. Public release creation is separate from ordinary develop commits.
- After publication, verify public downloads and update README/website download links. Any Community repository changes must also be pinned and verified in Pro; this does not authorize a paid Pro release.

## Official references

- [Tauri: macOS code signing](https://v2.tauri.app/distribute/sign/macos/)
- [Apple: Developer ID](https://developer.apple.com/developer-id/)
- [Apple: Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
