#!/bin/sh
# winget manifests for one release, from packaging/winget/*.yaml.in.
#   scripts/winget-manifests.sh <version> <installer sha256> <out dir>
# writes <out dir>/manifests/w/wiggleji/Margin/<version>/ (the layout of
# microsoft/winget-pkgs). Test one locally on Windows with
#   winget install --manifest <that folder>
# (after `winget settings --enable LocalManifestFiles` as administrator).
set -eu
[ $# -eq 3 ] || { echo "usage: $0 <version> <installer sha256> <out dir>" >&2; exit 2; }
version=$1
sha=$(printf %s "$2" | tr '[:lower:]' '[:upper:]')
case $sha in *[!0-9A-F]*|"") echo "not a SHA-256: $2" >&2; exit 2 ;; esac
[ ${#sha} -eq 64 ] || { echo "not a SHA-256: $2" >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
dir="$3/manifests/w/wiggleji/Margin/$version"
mkdir -p "$dir"
date=$(date -u +%Y-%m-%d)
for f in "$root"/packaging/winget/*.yaml.in; do
  name=$(basename "$f" .in)
  sed -e "s/@VERSION@/$version/g" -e "s/@SHA256@/$sha/g" -e "s/@DATE@/$date/g" "$f" > "$dir/$name"
done
echo "$dir"
