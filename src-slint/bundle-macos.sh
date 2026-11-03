#!/bin/sh
# Build the Slint front end and wrap it in a macOS app bundle:
# target/release/Moose Rack.app.
#
# The bundle is what lets macOS ask for the local network permission. Without
# it the server on the LAN can't be reached: a bare program that opens a window
# is refused with "No route to host" (os error 65) and never asked. The same
# string the desktop app uses (src-tauri/Info.plist) is what the prompt shows.
#
# Put the bundle in the folder that holds config.toml; the app keeps all its
# files beside itself (src/datadir.rs).
set -eu
cd "$(dirname "$0")"
cargo build --release

app="target/release/Moose Rack.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/moose-slint "$app/Contents/MacOS/moose-slint"
cp ../src-tauri/icons/icon.icns "$app/Contents/Resources/icon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>io.github.mizaimao.moose-rack.slint</string>
  <key>CFBundleName</key>
  <string>Moose Rack</string>
  <key>CFBundleDisplayName</key>
  <string>Moose Rack</string>
  <key>CFBundleExecutable</key>
  <string>moose-slint</string>
  <key>CFBundleIconFile</key>
  <string>icon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Moose Rack reads your game library from your server on this network.</string>
</dict>
</plist>
PLIST
# Ad hoc, as the desktop app is: macOS ties the permission to the signature.
codesign --force --sign - "$app"
echo "$app"
