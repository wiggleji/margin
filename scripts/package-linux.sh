#!/usr/bin/env bash
# Build Margin for Linux (x86_64) and package it:
#   dist/margin-<version>-linux-x64.tar.gz   portable: unpack and run (needs glibc ≥ the build machine's)
#   dist/margin_<version>_amd64.deb          Debian / Ubuntu: apt install ./margin_<version>_amd64.deb
#   dist/Margin-<version>-x86_64.AppImage    any distribution: one file, chmod +x and run
# each with a .sha256.
#
# The AppImage is made with appimagetool and the AppImage runtime, both
# checked against pinned SHA-256s (the runtime is only published as
# "continuous": when upstream updates it the build stops, and the new
# checksum goes in RUNTIME_SHA256 below after a look at what changed).
# NO_APPIMAGE=1 skips it.
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

# ---------------------------------------------------------------- AppImage
APPIMAGE="$DIST/Margin-$VERSION-x86_64.AppImage"
if [ "${NO_APPIMAGE:-0}" != 1 ]; then
  step "Building $(basename "$APPIMAGE")"
  TOOL_VERSION=1.9.1
  TOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
  RUNTIME_SHA256=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
  # fetch(url, file, sha256): download once, always check
  fetch() {
    if [ ! -f "$2" ]; then curl -fsSL -o "$2.part" "$1" && mv "$2.part" "$2"; fi
    echo "$3  $2" | sha256sum -c --quiet - || { echo "error: $(basename "$2") doesn't match its pinned checksum (see the comment at the top)" >&2; rm -f "$2"; exit 1; }
  }
  TOOL="$DIST/appimagetool-$TOOL_VERSION-x86_64.AppImage"
  RUNTIME="$DIST/appimage-runtime-x86_64"
  fetch "https://github.com/AppImage/appimagetool/releases/download/$TOOL_VERSION/appimagetool-x86_64.AppImage" "$TOOL" "$TOOL_SHA256"
  fetch "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64" "$RUNTIME" "$RUNTIME_SHA256"
  chmod +x "$TOOL"
  APPDIR="$DIST/Margin.AppDir"
  rm -rf "$APPDIR"
  stage_usr "$APPDIR/usr"
  cp "$ROOT/packaging/linux/margin.desktop" "$APPDIR/margin.desktop"
  cp "$ROOT/packaging/linux/margin-256.png" "$APPDIR/margin.png"
  cat > "$APPDIR/AppRun" <<'SH'
#!/bin/sh
# Margin.AppImage [folder]            open a repository
# Margin.AppImage mcp --repo <path>   the MCP server for agents
here="$(dirname "$(readlink -f "$0")")"
if [ "${1:-}" = mcp ]; then
  shift
  exec "$here/usr/bin/margin-mcp" "$@"
fi
exec "$here/usr/bin/margin" "$@"
SH
  chmod 755 "$APPDIR/AppRun"
  rm -f "$APPIMAGE"
  # (no FUSE needed to run the tool itself, e.g. in CI containers)
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$TOOL" --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "$APPIMAGE" >/dev/null
  rm -rf "$APPDIR"
  (cd "$DIST" && sha256sum "$(basename "$APPIMAGE")" > "$(basename "$APPIMAGE").sha256")
fi

step "Done"
echo "  tar.gz:   $TGZ"
echo "  deb:      $DEB"
[ "${NO_APPIMAGE:-0}" = 1 ] || echo "  AppImage: $APPIMAGE"
