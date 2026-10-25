#!/usr/bin/env bash
# Builds dist/MapCraft-x86_64.AppImage with the SDKs and their plugins.
# Needs appimagetool on PATH, or set APPIMAGETOOL to its path.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
tool="${APPIMAGETOOL:-appimagetool}"

cargo xtask plugins
cargo build --release -p mapcraft

app=dist/AppDir
assets=apps/mapcraft/assets
rm -rf "$app"
mkdir -p "$app/usr/bin" "$app/usr/share/mapcraft" "$app/usr/share/doc/mapcraft" "$app/usr/share/applications" \
	"$app/usr/share/icons/hicolor/256x256/apps" "$app/usr/share/icons/hicolor/512x512/apps"
cp target/release/mapcraft "$app/usr/bin/"
# The editor looks for ../share/mapcraft/sdks next to its executable.
cp -r sdks "$app/usr/share/mapcraft/"
cp LICENSE "$app/usr/share/doc/mapcraft/"
cp "$assets/mapcraft.desktop" "$app/usr/share/applications/"
cp "$assets/mapcraft-256.png" "$app/usr/share/icons/hicolor/256x256/apps/mapcraft.png"
cp "$assets/mapcraft-512.png" "$app/usr/share/icons/hicolor/512x512/apps/mapcraft.png"
cp "$assets/mapcraft.desktop" "$app/"
cp "$assets/mapcraft-256.png" "$app/mapcraft.png"
ln -s mapcraft.png "$app/.DirIcon"
ln -s usr/bin/mapcraft "$app/AppRun"

ARCH=x86_64 "$tool" --no-appstream "$app" dist/MapCraft-x86_64.AppImage
rm -rf "$app"
