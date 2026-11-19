# Releasing Margin for macOS

Margin ships as a **Developer ID–signed, notarized** `Margin.app` inside a `.dmg`. It's universal (Apple Silicon and Intel), and opens on any Mac without Gatekeeper warnings. It's not on the Mac App Store, because a git client can't work inside the App Sandbox (see `packaging/macos/entitlements.plist`).

```
scripts/package-macos.sh       build → bundle → sign → notarize → dmg → checksum
packaging/macos/               Info.plist template, entitlements, app icon (iconset)
scripts/make-icon.py           regenerates the placeholder icon
.github/workflows/release-macos.yml   the same thing on GitHub Actions
```

`Margin.app/Contents/MacOS/` also contains `margin-mcp`. Point agents' MCP configuration at `/Applications/Margin.app/Contents/MacOS/margin-mcp`.

## One-time setup

1. **Apple Developer Program** (99 USD/year): https://developer.apple.com/programs/

2. **Developer ID Application certificate.** In Xcode → Settings → Accounts → Manage Certificates → **+** → *Developer ID Application*. Check it's in your keychain:
   ```bash
   security find-identity -v -p codesigning
   ```
   Note the full name, `Developer ID Application: Your Name (TEAMID)`.

3. **Notarization credentials.** In App Store Connect → Users and Access → Integrations → *App Store Connect API* → generate a key with the *Developer* role. Download the `.p8` once, and note the Key ID and Issuer ID. Save a local profile:
   ```bash
   xcrun notarytool store-credentials margin-notary --key AuthKey_XXXX.p8 --key-id XXXX --issuer YYYY
   ```

## Release from your Mac

```bash
export MARGIN_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export MARGIN_NOTARY_PROFILE=margin-notary
```

```bash
scripts/package-macos.sh
```

This produces `dist/Margin-<version>.dmg` and `dist/Margin-<version>-macos.zip` (the app for in-app updates), each with a `.sha256`. Notarization usually takes 1–5 minutes per submission, and the app and the DMG are submitted separately.

Other modes:

| Command | Use |
|---|---|
| `scripts/package-macos.sh --dev` | Local test build: ad-hoc signed and not notarized. No Apple account needed. |
| `scripts/package-macos.sh --no-notarize` | Developer ID signed, skip notarization (e.g. to check signing). |
| `scripts/package-macos.sh --native` | Only this Mac's architecture (faster). |
| `scripts/package-macos.sh --dry-run` | Print every step without running anything (works on any OS). |

Bump the version in the root `Cargo.toml` (`[workspace.package] version`) before a release. The build number is the commit count.

## Release from GitHub Actions

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and merge it into `main`.
2. *Actions → Release → Run workflow*, on `main`, with **Publish a release** ticked.

The workflow builds on macOS, Windows, and Linux in parallel, signs and notarizes the Mac app (with the Apple secrets), creates the tag `v<version>` on that commit, and publishes one release with every download: the DMG and the in-app update zip, the Windows installer and zip (`scripts/package-windows.ps1`, Inno Setup), and the Linux `.deb` and `.tar.gz` (`scripts/package-linux.sh`, built on Ubuntu 22.04 so it runs on glibc 2.35 and newer) (marked latest, so the README download link and Margin's updater pick it up). It refuses a version that's already released, or a run from another branch.

Untick **Publish a release** to only build: the DMG and zip are then artifacts of the run.

Pushing a tag works too (`git tag v0.2.0 && git push origin v0.2.0`); the tag must match the Cargo version.

**Without the Apple secrets** the same workflow still builds and releases a DMG, ad-hoc signed instead of notarized: macOS blocks the first open until the user runs `xattr -dr com.apple.quarantine /Applications/Margin.app` or clicks **Open Anyway** in System Settings → Privacy & Security, and the release notes say so. Add the secrets later and the next release is signed and notarized.

Every push to `main` and every pull request also runs **CI** (`.github/workflows/ci.yml`): build and tests on Linux and macOS, plus an unsigned test package of the app.

Configure these in *Settings → Secrets and variables → Actions*:

| Name | Kind | Value |
|---|---|---|
| `MACOS_CERTIFICATE_P12` | secret | `base64 -i DeveloperID.p12`. In Keychain Access, export the Developer ID certificate together with its private key as .p12. |
| `MACOS_CERTIFICATE_PASSWORD` | secret | password chosen when exporting the .p12 |
| `MACOS_SIGN_IDENTITY` | secret | `Developer ID Application: Your Name (TEAMID)` |
| `NOTARY_API_KEY_P8` | secret | `base64 -i AuthKey_XXXX.p8` |
| `NOTARY_API_KEY_ID` | secret | the API key's Key ID |
| `NOTARY_API_ISSUER_ID` | secret | the Issuer ID |

macOS runners use about 10× the minutes of Linux runners against a private repo's Actions quota. One release run takes roughly 15–25 minutes.

## Verify a DMG

```bash
spctl --assess --type open --context context:primary-signature -vv Margin-0.1.0.dmg
```

```bash
xcrun stapler validate Margin-0.1.0.dmg
```

```bash
codesign -dv --verbose=4 /Applications/Margin.app
```

## Troubleshooting

- **`errSecInternalComponent` while signing (CI):** the keychain is locked or the partition list isn't set. The workflow handles both; check that the .p12 includes the private key.
- **Notarization "Invalid":** fetch the log with `xcrun notarytool log <submission-id> --keychain-profile margin-notary`. Usual causes are a missing `--options runtime` or `--timestamp` (both set by the script) or an unsigned nested binary.
- **The app opens but doesn't find Homebrew git:** Finder-launched apps get a minimal `PATH`; Margin prepends `/opt/homebrew/bin` and `/usr/local/bin` at startup. Set `MARGIN_GIT` to pin a specific git.
