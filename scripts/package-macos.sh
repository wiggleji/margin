#!/usr/bin/env bash
# Build, sign, notarize and package Margin for macOS.
#
#   scripts/package-macos.sh                 # release: universal, signed, notarized
#   scripts/package-macos.sh --dev           # local test build: ad-hoc signed, no notarization
#   scripts/package-macos.sh --no-notarize   # signed with Developer ID, skip notarization
#   scripts/package-macos.sh --native        # build only for this Mac's architecture
#   scripts/package-macos.sh --dry-run       # print every step without running it (any OS)
#
# Release builds need (see docs/RELEASING.md):
#   MARGIN_LICENSE_PUBKEY   licence public key (from `margin-license keygen`)
#   MARGIN_SIGN_IDENTITY    "Developer ID Application: Your Name (TEAMID)"
#   MARGIN_NOTARY_PROFILE   notarytool keychain profile (xcrun notarytool store-credentials)
# Optional:
#   MARGIN_BUNDLE_ID        default io.github.wiggleji.margin
#   MARGIN_BUILD_NUMBER     default: number of commits
#   MARGIN_NOTARY_KEYCHAIN  keychain holding the notary profile (CI)
#
# Output: dist/Margin.app, dist/Margin-<version>.dmg (+ .sha256)

set -euo pipefail

DEV=0
NOTARIZE=1
DRY=0
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

for arg in "$@"; do
  case "$arg" in
    --dev) DEV=1; NOTARIZE=0 ;;
    --no-notarize) NOTARIZE=0 ;;
    --dry-run) DRY=1 ;;
    --native)
      if [ "$(uname -m)" = "arm64" ]; then TARGETS=(aarch64-apple-darwin); else TARGETS=(x86_64-apple-darwin); fi ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/packaging/macos"
DIST="$ROOT/dist"
APP="$DIST/Margin.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
DMG="$DIST/Margin-$VERSION.dmg"
BUNDLE_ID="${MARGIN_BUNDLE_ID:-io.github.wiggleji.margin}"
BUILD_NUMBER="${MARGIN_BUILD_NUMBER:-$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 1)}"
# Embedded build date = last commit date: reproducible, and decides which
# licence keys cover this version ("updates until").
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || date +%s)}"
export SOURCE_DATE_EPOCH

step() { printf '\n==> %s\n' "$*"; }
die() { echo "error: $*" >&2; exit 1; }
# Run a command, or just print it with --dry-run.
run() {
  if [ "$DRY" = 1 ]; then printf '  + %s\n' "$*"; else "$@"; fi
}

# ------------------------------------------------------------------ checks
step "Margin $VERSION (build $BUILD_NUMBER) · ${TARGETS[*]} · $([ "$DEV" = 1 ] && echo dev || echo release)"

if [ "$DEV" = 1 ]; then
  SIGN_ID="-" # ad-hoc: runs on this Mac, not distributable
  [ -n "${MARGIN_LICENSE_PUBKEY:-}" ] || echo "note: MARGIN_LICENSE_PUBKEY not set — this build accepts development licence keys only"
else
  [[ "${MARGIN_LICENSE_PUBKEY:-}" =~ ^[0-9a-fA-F]{64}$ ]] \
    || die "MARGIN_LICENSE_PUBKEY must be the 64-hex public key from 'margin-license keygen' (or use --dev)"
  SIGN_ID="${MARGIN_SIGN_IDENTITY:-}"
  [[ "$SIGN_ID" == "Developer ID Application:"* ]] \
    || die "MARGIN_SIGN_IDENTITY must be your 'Developer ID Application: Name (TEAMID)' certificate (security find-identity -v -p codesigning)"
  if [ "$NOTARIZE" = 1 ]; then
    [ -n "${MARGIN_NOTARY_PROFILE:-}" ] || die "MARGIN_NOTARY_PROFILE not set (xcrun notarytool store-credentials …), or pass --no-notarize"
  fi
fi

if [ "$DRY" = 0 ]; then
  [ "$(uname -s)" = "Darwin" ] || die "packaging needs macOS (try --dry-run elsewhere)"
  for tool in cargo rustup lipo codesign hdiutil iconutil plutil ditto shasum; do
    command -v "$tool" >/dev/null || die "missing tool: $tool"
  done
  if [ "$NOTARIZE" = 1 ]; then xcrun --find notarytool >/dev/null || die "notarytool missing (install Xcode command line tools)"; fi
fi

# ------------------------------------------------------------------- build
step "Building release binaries"
BIN_DIR="$DIST/bin"
run mkdir -p "$BIN_DIR"
for t in "${TARGETS[@]}"; do
  run rustup target add "$t"
  run cargo build --release --locked --target "$t" -p margin-app -p margin-mcp --manifest-path "$ROOT/Cargo.toml"
done
for b in margin margin-mcp; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$ROOT/target/$t/release/$b"); done
  # One binary that runs natively on Apple Silicon and Intel.
  run lipo -create -output "$BIN_DIR/$b" "${inputs[@]}"
done

# ------------------------------------------------------------------ bundle
step "Assembling Margin.app"
run rm -rf "$APP"
run mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
run cp "$BIN_DIR/margin" "$BIN_DIR/margin-mcp" "$APP/Contents/MacOS/"
# Info.plist from the template (generated even in --dry-run so it can be checked).
mkdir -p "$DIST"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@BUILD@/$BUILD_NUMBER/g" \
    -e "s/@BUNDLE_ID@/$BUNDLE_ID/g" -e "s/@YEAR@/$(date +%Y)/g" \
    "$PKG/Info.plist.in" > "$DIST/Info.plist"
run cp "$DIST/Info.plist" "$APP/Contents/Info.plist"
run plutil -lint "$APP/Contents/Info.plist"
run iconutil -c icns "$PKG/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
run cp "$ROOT/LICENSE" "$ROOT/THIRD_PARTY_NOTICES.md" "$APP/Contents/Resources/"

# -------------------------------------------------------------------- sign
step "Signing ($([ "$DEV" = 1 ] && echo ad-hoc || echo "$SIGN_ID"))"
SIGN_ARGS=(--force --options runtime --entitlements "$PKG/entitlements.plist" --sign "$SIGN_ID")
[ "$DEV" = 1 ] || SIGN_ARGS+=(--timestamp) # secure timestamp: required for notarization
# Inside-out: nested executables first, then the bundle (which seals them).
run codesign "${SIGN_ARGS[@]}" "$APP/Contents/MacOS/margin-mcp"
run codesign "${SIGN_ARGS[@]}" "$APP"
run codesign --verify --strict --verbose=2 "$APP"

notarize() { # $1 = file to submit
  local kc=()
  # CI keeps the notary credentials in a temporary keychain.
  [ -n "${MARGIN_NOTARY_KEYCHAIN:-}" ] && kc=(--keychain "$MARGIN_NOTARY_KEYCHAIN")
  # ${kc[@]+…}: an empty array under `set -u` crashes macOS's bash 3.2.
  run xcrun notarytool submit "$1" --keychain-profile "$MARGIN_NOTARY_PROFILE" ${kc[@]+"${kc[@]}"} --wait
}

if [ "$NOTARIZE" = 1 ]; then
  step "Notarizing the app"
  ZIP="$DIST/Margin-$VERSION-notarize.zip"
  run ditto -c -k --keepParent "$APP" "$ZIP"
  notarize "$ZIP"
  # Staple the ticket so Gatekeeper accepts the app even offline.
  run xcrun stapler staple "$APP"
  run rm -f "$ZIP"
fi

# --------------------------------------------------------------------- dmg
step "Building $(basename "$DMG")"
STAGE="$DIST/dmg-stage"
run rm -rf "$STAGE" "$DMG"
run mkdir -p "$STAGE"
run cp -R "$APP" "$STAGE/"
run ln -s /Applications "$STAGE/Applications" # drag-to-install
run hdiutil create -volname "Margin $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
run rm -rf "$STAGE"
if [ "$DEV" = 0 ]; then
  run codesign --force --timestamp --sign "$SIGN_ID" "$DMG"
fi

if [ "$NOTARIZE" = 1 ]; then
  step "Notarizing the DMG"
  notarize "$DMG"
  run xcrun stapler staple "$DMG"
  step "Gatekeeper check"
  run spctl --assess --type execute --verbose=2 "$APP"
  run spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
fi

run sh -c "cd '$DIST' && shasum -a 256 '$(basename "$DMG")' > '$(basename "$DMG").sha256'"

step "Done"
echo "  app: $APP"
echo "  dmg: $DMG"
[ "$NOTARIZE" = 1 ] || echo "  (not notarized: other Macs will warn on first open; fine for local testing)"
