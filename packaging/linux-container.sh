#!/usr/bin/env bash
# Runs a command from the repository root inside Ubuntu 22.04 as the calling user, so Linux builds
# only need that glibc. Rust, cargo's downloads, the target folder and ccache stay in $CACHE between runs.
# Usage: packaging/linux-container.sh packaging/appimage.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache="${CACHE:-/build/ci-cache/u22}"
mkdir -p "$cache/home"
docker build -q -t mapcraft-u22 -f "$root/packaging/u22.Dockerfile" "$root/packaging" >/dev/null
docker run --rm --user "$(id -u):$(id -g)" -v "$root:$root" -w "$root" -v "$cache:/cache" \
	-e HOME=/cache/home -e RUSTUP_HOME=/cache/rustup -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target \
	-e CCACHE_DIR=/cache/ccache -e CMAKE_C_COMPILER_LAUNCHER=ccache -e CMAKE_CXX_COMPILER_LAUNCHER=ccache \
	-e APPIMAGETOOL=/cache/appimagetool -e APPIMAGE_EXTRACT_AND_RUN=1 -e MAPCRAFT_VERSION \
	mapcraft-u22 bash -euo pipefail -c '
export PATH=/cache/cargo/bin:$PATH
command -v rustup >/dev/null || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
rustup update stable --no-self-update
if [ ! -x "$APPIMAGETOOL" ]; then
	curl -sSL -o "$APPIMAGETOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
	chmod +x "$APPIMAGETOOL"
fi
"$@"' bash "$@"
