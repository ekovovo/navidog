#!/usr/bin/env bash
# Ad-hoc sign a complete app after its executable and resources are in place.
set -euo pipefail

if [ "$(uname -s)" != Darwin ]; then
  echo "macOS app signing requires macOS" >&2
  exit 1
fi
if [ "$#" -ne 1 ] || [ ! -d "$1" ] || [ ! -f "$1/Contents/Info.plist" ]; then
  echo "Usage: $0 path/to/Navidog.app" >&2
  exit 2
fi

app="$(cd "$1" && pwd)"
# No certificate, keychain, network timestamp or administrator access is needed.
# Nested code must already be signed; do not use --deep to sign it implicitly.
codesign --force --sign - --timestamp=none "$app"
codesign --verify --deep --strict --all-architectures --verbose=2 "$app"
