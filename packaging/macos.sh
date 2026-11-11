#!/bin/bash
set -euo pipefail
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --locked --target aarch64-apple-darwin
cargo build --release --locked --target x86_64-apple-darwin
APP=Tror.app
rm -rf "$APP" tror.iconset Tror-macos.zip
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" tror.iconset
lipo -create -output "$APP/Contents/MacOS/tror" \
  target/aarch64-apple-darwin/release/tror target/x86_64-apple-darwin/release/tror
for size in 16 32 128 256 512; do
  sips -z $size $size assets/tror-1024.png --out tror.iconset/icon_${size}x${size}.png >/dev/null
  double=$((size * 2))
  sips -z $double $double assets/tror-1024.png --out tror.iconset/icon_${size}x${size}@2x.png >/dev/null
done
iconutil -c icns tror.iconset -o "$APP/Contents/Resources/tror.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Tror</string>
  <key>CFBundleDisplayName</key><string>Tror</string>
  <key>CFBundleIdentifier</key><string>io.github.jozef2j.tror</string>
  <key>CFBundleExecutable</key><string>tror</string>
  <key>CFBundleIconFile</key><string>tror</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Text</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSItemContentTypes</key>
      <array><string>public.text</string><string>public.data</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
ditto -c -k --keepParent "$APP" Tror-macos.zip
rm -rf tror.iconset
