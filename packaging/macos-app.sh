#!/usr/bin/env bash
# Builds dist/MapCraft-macos.zip: a universal (Apple Silicon and Intel) MapCraft.app; SDKs install from the SDK Manager.
# Runs on macOS with the aarch64-apple-darwin and x86_64-apple-darwin Rust targets installed.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
version="${MAPCRAFT_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
	cargo build --release -p mapcraft --target "$target"
done

app=dist/MapCraft.app
rm -rf "$app" dist/MapCraft-macos.zip
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/mapcraft" target/aarch64-apple-darwin/release/mapcraft target/x86_64-apple-darwin/release/mapcraft
cp LICENSE "$app/Contents/Resources/LICENSE.txt"

icons=dist/mapcraft.iconset
rm -rf "$icons"
mkdir -p "$icons"
for size in 16 32 128 256 512; do
	sips -z "$size" "$size" apps/mapcraft/assets/mapcraft-512.png --out "$icons/icon_${size}x${size}.png" >/dev/null
	double=$((size * 2))
	[ "$double" -le 512 ] && sips -z "$double" "$double" apps/mapcraft/assets/mapcraft-512.png --out "$icons/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$icons" -o "$app/Contents/Resources/MapCraft.icns"
rm -rf "$icons"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>MapCraft</string>
	<key>CFBundleDisplayName</key><string>MapCraft</string>
	<key>CFBundleIdentifier</key><string>io.github.bearlikelion.mapcraft</string>
	<key>CFBundleExecutable</key><string>mapcraft</string>
	<key>CFBundleIconFile</key><string>MapCraft.icns</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>${version}</string>
	<key>CFBundleVersion</key><string>${version}</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>NSHighResolutionCapable</key><true/>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key><string>MapCraft map</string>
			<key>CFBundleTypeExtensions</key><array><string>mcmap</string></array>
			<key>CFBundleTypeRole</key><string>Editor</string>
		</dict>
	</array>
</dict>
</plist>
EOF

# Apple Silicon only runs signed code; an ad-hoc signature is enough for a downloaded app opened with Open Anyway.
codesign --force --deep --sign - "$app"
ditto -c -k --keepParent "$app" dist/MapCraft-macos.zip
rm -rf "$app"
