#!/bin/sh
# Packages dist/gitroute.app into dist/gitroute-<version>.dmg: the app next
# to an Applications link to drag it onto. Run scripts/bundle.sh first.
#
# For a download that opens without warnings, set both:
#   GITROUTE_SIGN_IDENTITY   "Developer ID Application: …", also used by
#                            scripts/bundle.sh to sign the app
#   GITROUTE_NOTARY_PROFILE  a profile saved with
#                            `xcrun notarytool store-credentials <name>`
# The disk image is then signed, notarized, and the ticket stapled to it.
set -eu
cd "$(dirname "$0")/.."

app=dist/gitroute.app
if [ ! -d "$app" ]; then
	echo "no $app: run scripts/bundle.sh first" >&2
	exit 1
fi
identity=${GITROUTE_SIGN_IDENTITY:-}
profile=${GITROUTE_NOTARY_PROFILE:-}
if [ -n "$profile" ] && [ -z "$identity" ]; then
	echo "notarizing needs GITROUTE_SIGN_IDENTITY too" >&2
	exit 1
fi

version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")
dmg=dist/gitroute-$version.dmg
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/gitroute.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname "gitroute $version" -srcfolder "$stage" \
	-fs HFS+ -format UDZO "$dmg"

if [ -n "$identity" ]; then
	codesign --force --sign "$identity" --timestamp "$dmg"
fi
if [ -n "$profile" ]; then
	xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
	# Fails unless Apple accepted the submission.
	xcrun stapler staple "$dmg"
	spctl --assess --type open --context context:primary-signature --verbose "$dmg"
fi

echo "built $dmg"
shasum -a 256 "$dmg"
