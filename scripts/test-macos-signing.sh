#!/usr/bin/env bash
# Exercise ad-hoc signing and whole-bundle updates with the real macOS tools.
set -euo pipefail

if [ "$(uname -s)" != Darwin ]; then
  echo "macOS signing checks require macOS" >&2
  exit 1
fi

repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/navidog-signing.XXXXXX")
mount="$work/Mounted image"
mounted=false

cleanup() {
  status=$?
  trap - EXIT
  if [ "$mounted" = true ]; then
    if ! hdiutil detach "$mount" >/dev/null; then
      echo "Could not detach the test image; keeping $work" >&2
      exit 1
    fi
  fi
  rm -rf "$work"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

app="$work/Signing fixture.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cat > "$work/main.c" <<'C'
#include <stdio.h>
int main(void) {
    puts("navidog signing fixture");
    return 0;
}
C
clang "$work/main.c" -o "$app/Contents/MacOS/signing-fixture"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>ci.nn.navidog.signing-test</string>
  <key>CFBundleName</key><string>Signing fixture</string>
  <key>CFBundleExecutable</key><string>signing-fixture</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>
PLIST
echo "original resource" > "$app/Contents/Resources/sealed.txt"

verify_and_run() {
  codesign --verify --deep --strict --all-architectures "$1"
  test "$("$1/Contents/MacOS/signing-fixture")" = "navidog signing fixture"
}

code_hash() {
  codesign --display --verbose=4 "$1" 2>&1 | sed -n 's/^CDHash=//p'
}

"$repo/scripts/sign-macos.sh" "$app"
verify_and_run "$app"

# A changed sealed resource must be detected before an artifact is published.
echo "changed resource" >> "$app/Contents/Resources/sealed.txt"
if codesign --verify --deep --strict --all-architectures "$app" >"$work/tamper.log" 2>&1; then
  echo "Verification unexpectedly accepted a modified resource" >&2
  exit 1
fi
"$repo/scripts/sign-macos.sh" "$app"
verify_and_run "$app"
expected_hash=$(code_hash "$app")
test -n "$expected_hash"

stage="$work/Disk image contents"
mkdir -p "$stage" "$mount" "$work/Applications"
ditto "$app" "$stage/Signing fixture.app"
hdiutil create -volname "Navidog signing check" -srcfolder "$stage" \
  -format UDZO "$work/Signing fixture.dmg" >/dev/null
hdiutil attach -nobrowse -readonly -mountpoint "$mount" \
  "$work/Signing fixture.dmg" >/dev/null
mounted=true

# Match update.rs: ditto the complete new bundle, move the old bundle aside,
# then move the staged bundle into its place in a user-writable directory.
installed="$work/Applications/Signing fixture.app"
staged="$work/New fixture.app"
aside="$work/Old fixture.app"
ditto "$app" "$installed"
ditto "$mount/Signing fixture.app" "$staged"
hdiutil detach "$mount" >/dev/null
mounted=false
mv "$installed" "$aside"
mv "$staged" "$installed"
verify_and_run "$installed"
test "$(code_hash "$installed")" = "$expected_hash"
echo "macOS signing, tamper detection, DMG copy and bundle replacement passed"
