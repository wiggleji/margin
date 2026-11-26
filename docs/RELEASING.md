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

The workflow builds on macOS, Windows, and Linux in parallel, signs and notarizes the Mac app (with the Apple secrets), creates the tag `v<version>` on that commit, and publishes one release with every download: the DMG and the in-app update zip, the Windows installer and zip (`scripts/package-windows.ps1`, Inno Setup), and the Linux AppImage, `.deb` and `.tar.gz` (`scripts/package-linux.sh`, built on Ubuntu 22.04 so it runs on glibc 2.35 and newer; appimagetool and the AppImage runtime are pinned by SHA-256 there) (marked latest, so the README download link and Margin's updater pick it up). It refuses a version that's already released, or a run from another branch.

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

| `HOMEBREW_TAP_TOKEN` | secret | optional: a token that can push to `wiggleji/homebrew-tap` (see below) |
| `WINGET_TOKEN` | secret | optional: a classic token with `public_repo`, to open winget pull requests (see below) |

### Homebrew

`brew install --cask wiggleji/tap/margin` (macOS) and `brew install wiggleji/tap/margin` (Linux) install from the **wiggleji/homebrew-tap** repository: the cask `Casks/margin.rb` (the DMG) and the formula `Formula/margin.rb` (the Linux tar.gz). The Release workflow writes both for each release from `packaging/homebrew/margin.rb.in` and `margin-linux.rb.in` (the version and each download's SHA-256) and pushes them there, when `HOMEBREW_TAP_TOKEN` is set. On Linux the formula puts `margin` and `margin-mcp` in Homebrew's bin and the menu entry under its `share` (its caveats say how to link it); Margin's own updater leaves a Homebrew install to `brew upgrade margin`. One-time setup:

1. Create the public repository **wiggleji/homebrew-tap** (the `homebrew-` prefix is what lets `brew` find it as `wiggleji/tap`), with a README.
2. Create a fine-grained personal access token with access to only that repository, **Contents: Read and write**, and add it as the `HOMEBREW_TAP_TOKEN` secret here.
3. The next release publishes the cask. To publish the current one by hand, fill in the template with its version and the `sha256` from its `Margin-<version>.dmg.sha256`, and commit it as `Casks/margin.rb` in the tap.

The cask installs `Margin.app`, links `margin` (the launcher in `Margin.app/Contents/Resources/bin`) and `margin-mcp` onto PATH, and, until the app is notarized, clears the quarantine flag so the first open isn't blocked. It's marked `auto_updates`: Margin updates itself, and `brew upgrade` leaves it alone unless run with `--greedy`.

### winget

`winget install wiggleji.Margin` installs the Windows installer (per user, adds `margin` to PATH, and installs Git for Windows first if it's missing). Packages live in **microsoft/winget-pkgs**: each version is a pull request there with three manifests, which Microsoft's bots validate (they run the installer silently) and moderators merge. The first one, which adds the package, waits for a moderator's review; later versions usually go through much faster. The manifests come from `packaging/winget/*.yaml.in`, rendered by `scripts/winget-manifests.sh` with the installer's SHA-256.

The **Submit to winget** workflow (`.github/workflows/winget.yml`) opens that pull request:

- **After each release**, automatically, but only once `wiggleji.Margin` is in winget. Until the first pull request is merged, releases skip winget (the run shows a note), so they don't pile up first-listing pull requests.
- **By hand:** *Actions → Submit to winget → Run workflow*, with a version (empty: the latest release). It submits whether or not the package is listed yet: for the first listing, for the release that came out while it waited, or to retry a failed submission.

Either way it skips a version winget already has, and it syncs your winget-pkgs fork first. A failure never affects the release itself.

1. Create a **classic** personal access token with the **public_repo** scope (wingetcreate forks microsoft/winget-pkgs to your account and opens the pull request from there), and add it as the `WINGET_TOKEN` secret here.
2. Run *Submit to winget* for the first listing. Watch the pull request on microsoft/winget-pkgs for the bots' checks; once merged, `winget install wiggleji.Margin` works (and `winget upgrade` picks up later versions, though Margin updates itself anyway). From then on, releases submit by themselves.
3. If a submission fails with *"The forked repository could not be synced"*, open your fork `<you>/winget-pkgs` on GitHub → **Sync fork** (→ Discard commits), then run *Submit to winget* again for that version.

To try a version's manifests before submitting: `scripts/winget-manifests.sh <version> <installer sha256> out`, then on Windows `winget install --manifest out\manifests\w\wiggleji\Margin\<version>` (after `winget settings --enable LocalManifestFiles`, as administrator).

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
- **Windows: Margin doesn't open (no window, no error) on a virtual machine or over Remote Desktop:** it draws with OpenGL, and without a graphics driver Windows offers only OpenGL 1.1. Margin then starts again on Direct3D 12, which falls back to Windows' software renderer (WARP), and shows a message if that fails too. `MARGIN_RENDERER=wgpu` (Direct3D) or `glow` (OpenGL) picks one. (This is also what winget's validation machines need: they have no GPU.)
- **The app opens but doesn't find Homebrew git:** Finder-launched apps get a minimal `PATH`; Margin prepends `/opt/homebrew/bin` and `/usr/local/bin` at startup. Set `MARGIN_GIT` to pin a specific git.
