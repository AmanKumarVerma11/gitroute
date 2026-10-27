#!/bin/sh
# Builds dist/gitroute.app: the menu bar app, plus the credential helper and
# the CLI next to it (git is pointed at the helper inside the bundle).
#
#   scripts/bundle.sh              for this Mac's architecture
#   scripts/bundle.sh --universal  Apple silicon and Intel in one app
#
# Signed ad hoc unless GITROUTE_SIGN_IDENTITY names a "Developer ID
# Application: …" identity; then it's signed with the hardened runtime and a
# secure timestamp, as notarization requires (see scripts/dmg.sh).
set -eu
cd "$(dirname "$0")/.."

universal=false
case "${1:-}" in
--universal) universal=true ;;
"") ;;
*)
	echo "usage: $0 [--universal]" >&2
	exit 2
	;;
esac

bins="gitroute-app git-credential-gitroute gitroute"
# Matches LSMinimumSystemVersion below.
export MACOSX_DEPLOYMENT_TARGET=12.0
build() {
	cargo build --release --locked -p gitroute-app -p git-credential-gitroute -p gitroute-cli "$@"
}
if $universal; then
	build --target aarch64-apple-darwin
	build --target x86_64-apple-darwin
	out=target/universal
	mkdir -p "$out"
	for bin in $bins; do
		lipo -create -output "$out/$bin" \
			"target/aarch64-apple-darwin/release/$bin" \
			"target/x86_64-apple-darwin/release/$bin"
	done
else
	build
	out=target/release
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/gitroute.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for bin in $bins; do
	cp "$out/$bin" "$app/Contents/MacOS/$bin"
done
cp assets/gitroute.icns "$app/Contents/Resources/gitroute.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>gitroute</string>
	<key>CFBundleDisplayName</key><string>gitroute</string>
	<key>CFBundleIdentifier</key><string>io.github.amankumarverma11.gitroute</string>
	<key>CFBundleExecutable</key><string>gitroute-app</string>
	<key>CFBundleIconFile</key><string>gitroute</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$version</string>
	<key>CFBundleVersion</key><string>$version</string>
	<key>LSMinimumSystemVersion</key><string>12.0</string>
	<key>LSUIElement</key><true/>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# The helper and the CLI first, then the bundle, which signs gitroute-app.
# Keychain access goes through /usr/bin/security, so the signature doesn't
# affect which tokens the app can read.
identity=${GITROUTE_SIGN_IDENTITY:--}
if [ "$identity" = - ]; then
	set -- --sign -
else
	set -- --sign "$identity" --options runtime --timestamp
fi
for bin in git-credential-gitroute gitroute; do
	codesign --force "$@" "$app/Contents/MacOS/$bin"
done
codesign --force "$@" "$app"
codesign --verify --strict --deep "$app"
echo "built $app ($(lipo -archs "$app/Contents/MacOS/gitroute-app"))"
