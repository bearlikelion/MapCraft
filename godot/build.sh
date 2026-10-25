#!/usr/bin/env bash
# Builds the MapCraft GDExtension into addons/mapcraft/bin.
# Usage: ./build.sh [linux|windows|macos] [template_debug|template_release]
# Set GODOT_CPP to a local godot-cpp checkout to skip the download.
set -euo pipefail

platform="${1:-linux}"
target="${2:-template_debug}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
build="$here/build/$platform-$target"

args=(-G Ninja -DCMAKE_BUILD_TYPE=Release -DGODOTCPP_TARGET="$target")
if [[ -n "${GODOT_CPP:-}" ]]; then
	args+=("-DFETCHCONTENT_SOURCE_DIR_GODOT-CPP=$GODOT_CPP")
fi
case "$platform" in
	linux) ;;
	windows) args+=(-DCMAKE_TOOLCHAIN_FILE="$here/mingw-w64-x86_64.cmake") ;;
	macos) args+=(-DCMAKE_OSX_ARCHITECTURES="arm64;x86_64") ;;
	*) echo "unknown platform $platform" >&2; exit 2 ;;
esac

cmake -S "$here" -B "$build" "${args[@]}"
cmake --build "$build"
