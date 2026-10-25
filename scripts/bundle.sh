#!/bin/sh
# Builds dist/gitroute.app: the menu bar app, plus the credential helper and
# the CLI next to it (git is pointed at the helper inside the bundle).
set -eu
cd "$(dirname "$0")/.."

cargo build --release --locked -p gitroute-app -p git-credential-gitroute -p gitroute-cli

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/gitroute.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/gitroute-app "$app/Contents/MacOS/gitroute-app"
cp target/release/git-credential-gitroute "$app/Contents/MacOS/git-credential-gitroute"
cp target/release/gitroute "$app/Contents/MacOS/gitroute"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>gitroute</string>
	<key>CFBundleDisplayName</key><string>gitroute</string>
	<key>CFBundleIdentifier</key><string>io.github.amankumarverma11.gitroute</string>
	<key>CFBundleExecutable</key><string>gitroute-app</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$version</string>
	<key>CFBundleVersion</key><string>$version</string>
	<key>LSMinimumSystemVersion</key><string>12.0</string>
	<key>LSUIElement</key><true/>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so the bundle is consistent; no Developer ID needed for
# local use (Keychain access goes through /usr/bin/security either way).
codesign --force --deep --sign - "$app"
echo "built $app"
