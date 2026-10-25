#!/usr/bin/env bash
# Zips godot/addons/mapcraft with the libraries godot/build.sh has built into dist/MapCraft-godot-addon.zip.
# Unzip it into a Godot project's root folder.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$root/dist"
cd "$root/godot"
if ! compgen -G "addons/mapcraft/bin/*/*" >/dev/null; then
	echo "no libraries in godot/addons/mapcraft/bin; run godot/build.sh first" >&2
	exit 1
fi
rm -f "$root/dist/MapCraft-godot-addon.zip"
zip -qr "$root/dist/MapCraft-godot-addon.zip" addons/mapcraft
