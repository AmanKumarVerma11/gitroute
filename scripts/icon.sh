#!/bin/sh
# Regenerates assets/gitroute.icns from scripts/icon.swift. Needs Xcode or
# the Command Line Tools (swift, sips, iconutil).
set -eu
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
swift scripts/icon.swift "$tmp/icon.png"
set_dir="$tmp/gitroute.iconset"
mkdir "$set_dir"
for s in 16 32 128 256 512; do
	sips -z $s $s "$tmp/icon.png" --out "$set_dir/icon_${s}x${s}.png" >/dev/null
	d=$((s * 2))
	sips -z $d $d "$tmp/icon.png" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o assets/gitroute.icns
cp "$tmp/icon.png" assets/icon.png
echo "wrote assets/gitroute.icns and assets/icon.png"
