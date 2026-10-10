#!/usr/bin/env bash
# Builds an unsigned navidog.app and .dmg. Signing, notarization and Sparkle come in M15.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="navidog"))')
cargo build --release -p navidog

out=target/bundle
app="$out/navidog.app"
rm -rf "$out" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/navidog "$app/Contents/MacOS/navidog"

iconset="$out/navidog.iconset"
swift "$PWD/scripts/raster-icon.swift" "$PWD/crates/navidog/resources/navidog.svg" "$iconset"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/navidog.icns"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Navidog</string>
  <key>CFBundleDisplayName</key><string>Navidog</string>
  <key>CFBundleIdentifier</key><string>ci.nn.navidog</string>
  <key>CFBundleExecutable</key><string>navidog</string>
  <key>CFBundleIconFile</key><string>navidog</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>zh-Hans</string></array>
</dict>
</plist>
PLIST

hdiutil create -volname Navidog -srcfolder "$app" -ov -format UDZO "$out/navidog-${version}.dmg"
echo "Built $out/navidog-${version}.dmg"
