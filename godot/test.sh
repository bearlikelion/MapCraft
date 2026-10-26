#!/usr/bin/env bash
# Imports test/course.mcmap in headless Godot with the built extension and checks the scene.
# Usage: ./test.sh  (set GODOT to the editor binary when it is not on PATH)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
godot="${GODOT:-godot}"
ln -sfn ../addons "$here/test/addons"
# Listing the extension up front avoids a Godot 4.7 crash when --import finds a new one mid-run.
mkdir -p "$here/test/.godot"
# A cached import would hide changes to what the importer makes.
rm -rf "$here/test/.godot/imported"
echo "res://addons/mapcraft/mapcraft.gdextension" > "$here/test/.godot/extension_list.cfg"
"$godot" --headless --path "$here/test" --import
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
pack="$tmp/course.pck"
"$godot" --headless --path "$here/test" -s res://addons/mapcraft/tools/pack_map.gd -- res://course.mcmap res://maps/course/course.scn "$pack"
"$godot" --headless --path "$here/test" -s mapcraft_import_test.gd -- "$pack"
