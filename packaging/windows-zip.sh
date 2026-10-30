#!/usr/bin/env bash
# Builds dist/MapCraft-windows-x86_64.zip, a portable folder; SDKs install from the SDK Manager.
# Usage: packaging/windows-zip.sh [target], x86_64-pc-windows-gnu (MinGW cross build) by default.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
target="${1:-x86_64-pc-windows-gnu}"

cargo build --release -p mapcraft --target "$target"

stage=dist/MapCraft
rm -rf "$stage" dist/MapCraft-windows-x86_64.zip
mkdir -p "$stage"
cp "target/$target/release/mapcraft.exe" "$stage/MapCraft.exe"
cp LICENSE "$stage/LICENSE.txt"

cd dist
if command -v 7z >/dev/null; then
	7z a -tzip -bso0 MapCraft-windows-x86_64.zip MapCraft
else
	zip -qr MapCraft-windows-x86_64.zip MapCraft
fi
rm -rf MapCraft
