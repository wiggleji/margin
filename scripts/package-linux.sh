#!/usr/bin/env bash
# Build Margin for Linux (x86_64) and package it:
#   dist/margin-<version>-linux-x64.tar.gz   portable: unpack and run (needs glibc ≥ the build machine's)
#   dist/margin_<version>_amd64.deb          Debian / Ubuntu: apt install ./margin_<version>_amd64.deb
# each with a .sha256.
#
#   scripts/package-linux.sh            build, then package
#   scripts/package-linux.sh --no-build package an existing target/release build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0

step() { printf '\n==> %s\n' "$*"; }

if [ "$BUILD" = 1 ]; then
  step "Building Margin $VERSION (release)"
  (cd "$ROOT" && cargo build --release --locked -p margin-app -p margin-mcp)
fi
BIN="$ROOT/target/release"
for b in margin margin-mcp; do
  [ -x "$BIN/$b" ] || { echo "error: $BIN/$b not found (build first)" >&2; exit 1; }
done
mkdir -p "$DIST"

# The files, laid out as under /usr (used by both packages).
stage_usr() { # $1 = prefix dir
  local p="$1"
  install -Dm755 "$BIN/margin" "$p/bin/margin"
  install -Dm755 "$BIN/margin-mcp" "$p/bin/margin-mcp"
  install -Dm644 "$ROOT/packaging/linux/margin.desktop" "$p/share/applications/margin.desktop"
  install -Dm644 "$ROOT/packaging/linux/margin-256.png" "$p/share/icons/hicolor/256x256/apps/margin.png"
  install -Dm644 "$ROOT/packaging/linux/margin-512.png" "$p/share/icons/hicolor/512x512/apps/margin.png"
  install -Dm644 "$ROOT/LICENSE" "$p/share/doc/margin/copyright"
  install -Dm644 "$ROOT/THIRD_PARTY_NOTICES.md" "$p/share/doc/margin/THIRD_PARTY_NOTICES.md"
}

# ------------------------------------------------------------------ tar.gz
NAME="margin-$VERSION-linux-x64"
TGZ="$DIST/$NAME.tar.gz"
step "Building $(basename "$TGZ")"
STAGE="$DIST/$NAME"
rm -rf "$STAGE" && mkdir -p "$STAGE"
stage_usr "$STAGE"
cat > "$STAGE/install.sh" <<'SH'
#!/bin/sh
# Install Margin for this user (~/.local): the app, the margin command, the
# menu entry and icon. Run again to update; remove those files to uninstall.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dest="${PREFIX:-$HOME/.local}"
for f in bin/margin bin/margin-mcp share/applications/margin.desktop \
         share/icons/hicolor/256x256/apps/margin.png share/icons/hicolor/512x512/apps/margin.png; do
  mkdir -p "$dest/$(dirname "$f")"
  cp "$here/$f" "$dest/$f"
done
command -v update-desktop-database >/dev/null && update-desktop-database "$dest/share/applications" 2>/dev/null || true
echo "Installed Margin in $dest. Run: margin /path/to/repo (make sure $dest/bin is on your PATH)."
SH
chmod 755 "$STAGE/install.sh"
cp "$ROOT/README.md" "$STAGE/README.md"
tar -C "$DIST" -czf "$TGZ" "$NAME"
rm -rf "$STAGE"

# --------------------------------------------------------------------- deb
DEB="$DIST/margin_${VERSION}_amd64.deb"
step "Building $(basename "$DEB")"
PKG="$DIST/deb-stage"
rm -rf "$PKG" && mkdir -p "$PKG/DEBIAN"
stage_usr "$PKG/usr"
SIZE_KB="$(du -sk "$PKG/usr" | cut -f1)"
cat > "$PKG/DEBIAN/control" <<CTRL
Package: margin
Version: $VERSION
Section: vcs
Priority: optional
Architecture: amd64
Maintainer: Margin contributors <https://github.com/wiggleji/margin>
Installed-Size: $SIZE_KB
Depends: libc6 (>= 2.35), git
Recommends: zenity | kdialog
Homepage: https://github.com/wiggleji/margin
Description: Git client with notes in the margin
 A light, fast git client with Markdown notes on commits, branches and
 tasks, GitHub pull request status, task tracker links, and an MCP server
 (margin-mcp) so AI agents can read and write the same notes.
CTRL
dpkg-deb --root-owner-group --build "$PKG" "$DEB" >/dev/null
rm -rf "$PKG"

(cd "$DIST" && sha256sum "$(basename "$TGZ")" > "$(basename "$TGZ").sha256" && sha256sum "$(basename "$DEB")" > "$(basename "$DEB").sha256")
step "Done"
echo "  tar.gz: $TGZ"
echo "  deb:    $DEB"
