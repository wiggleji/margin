#!/bin/sh
# The `margin` command shipped inside Margin.app (Contents/Resources/bin/margin),
# linked onto PATH by Homebrew (`brew install --cask wiggleji/tap/margin`).
#   margin [folder]   open a repository in Margin (default: this folder)
# Finds the app through the link, so it works wherever the app is installed.
self=$0
while [ -L "$self" ]; do
  link=$(readlink "$self")
  case $link in
    /*) self=$link ;;
    *) self=$(dirname "$self")/$link ;;
  esac
done
exe=$(cd "$(dirname "$self")/../../MacOS" 2>/dev/null && pwd -P)/margin
[ -x "$exe" ] || { echo "margin: can't find Margin.app next to this command ($self)" >&2; exit 1; }
case "$1" in -h|--help|--version) exec "$exe" "$@" ;; esac
dir="${1:-.}"
abs=$(cd "$dir" 2>/dev/null && pwd -P) || { echo "margin: no such folder: $dir" >&2; exit 1; }
nohup "$exe" "$abs" >/dev/null 2>&1 &
