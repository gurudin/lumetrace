# macOS direct-download distribution

## Current preparation

The next Community version is 1.0.1. Keep the existing v1.0.0 tag unchanged. The user-facing Release notes live in [releases/v1.0.1.md](releases/v1.0.1.md); download and installation text must be finalized against the actual installer before publication. Developer verification belongs here or in CI, not in the user-facing Release notes.

On 2026-09-11, the system keychain exposed an Apple Development identity but no Developer ID Application identity. Xcode's notarytool and stapler are available. Apple Development is not a replacement for direct-distribution Developer ID signing. No signed/notarized 1.0.1 installer has been produced or published by this preparation.

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
src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/LumeTrace_1.0.1_aarch64.dmg
```

Confirm actual output paths. Do not rename an older-version binary to make it look like 1.0.1. Do not install development builds into `/Applications` or launch automated acceptance against the user's live workspace.

## Verify the final artifacts

Run these against the freshly built paths, checking exit codes and output:

```sh
codesign --verify --deep --strict --verbose=2 'path/to/LumeTrace.app'
codesign -dv --verbose=4 'path/to/LumeTrace.app'
xcrun stapler validate 'path/to/LumeTrace.app'
spctl --assess --type execute --verbose=4 'path/to/LumeTrace.app'
hdiutil verify 'path/to/LumeTrace_1.0.1_aarch64.dmg'
shasum -a 256 'path/to/LumeTrace_1.0.1_aarch64.dmg'
```

The signature must show Developer ID Application and hardened runtime. Check Apple's notarization result and the stapled app ticket; do not infer success from the presence of a DMG. If the DMG itself is separately notarized/stapled, validate its ticket too. Inspect the app version, arm64 architecture, bundle identity, and actual minimum macOS version.

On an isolated test account/Mac, verify the downloaded DMG opens, the app can be copied and launched, and macOS recognizes its verified publisher without disabling security protections. Test with synthetic files. Include checks for preview, saving, file history, search, and the new Help Center. Signed release execution and bundled runtime dependencies require this acceptance even after debug and unit tests pass.

## Prepare the GitHub Release

- Retain v1.0.0 unchanged. Agree on the exact v1.0.1 commit and tag before creating a new tag. Do not switch or merge main automatically.
- Prepare a draft Release with [v1.0.1 notes](releases/v1.0.1.md), the verified DMG, and its SHA-256 checksum. Verify the artifact name matches Download.
- Replace the Installation placeholder with the actual tested installation and first-launch steps. State the verified minimum macOS version. Do not advertise signing or notarization if either is unfinished.
- Obtain final publication approval after the concrete draft and installer are reviewable. Public release creation is separate from ordinary develop commits.
- After publication, verify public downloads and update README/website download links. Any Community repository changes must also be pinned and verified in Pro; this does not authorize a paid Pro release.

## Official references

- [Tauri: macOS code signing](https://v2.tauri.app/distribute/sign/macos/)
- [Apple: Developer ID](https://developer.apple.com/developer-id/)
- [Apple: Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
