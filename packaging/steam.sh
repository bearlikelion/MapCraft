#!/usr/bin/env bash
# Builds Steam depots of a branded build into dist/steam: content/<os> and the steamcmd build scripts.
# Usage: packaging/steam.sh <config> [linux|windows]..., linux by default; see packaging/steam/surfsup.env.
# Build linux inside packaging/linux-container.sh; windows needs x86_64-pc-windows-msvc, through cargo xwin off Windows.
# Then: steamcmd +login <account> +run_app_build "$PWD/dist/steam/scripts/app_build.vdf" +quit
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
config="$1"
shift
# shellcheck source=/dev/null
. "$config"
export MAPCRAFT_BRAND="$BRAND" MAPCRAFT_APP_ID="$APP_ID"
target_dir="${CARGO_TARGET_DIR:-target}"
out=dist/steam
mkdir -p "$out/scripts"

command -v rustup >/dev/null && rustup target add wasm32-wasip2
cargo xtask plugins

for os in "${@:-linux}"; do
	var="DEPOT_${os^^}"
	depot="${!var:-}"
	if [[ -z "$depot" ]]; then
		echo "$config: set $var" >&2
		exit 1
	fi
	content="$out/content/$os"
	rm -rf "$content"
	mkdir -p "$content/sdks"
	case "$os" in
	linux)
		# steam_api sits next to the binary, as Steam installs it.
		RUSTFLAGS='-C link-arg=-Wl,-rpath,$ORIGIN' cargo build --release -p mapcraft --features steam
		cp "$target_dir/release/mapcraft" "$content/"
		lib="$(ls -t "$target_dir"/release/build/steamworks-sys-*/out/libsteam_api.so | head -1)"
		;;
	windows)
		triple=x86_64-pc-windows-msvc
		if [[ "${OSTYPE:-}" == msys* || "${OSTYPE:-}" == cygwin* ]]; then
			cargo build --release -p mapcraft --features steam --target "$triple"
		else
			cargo xwin build --release -p mapcraft --features steam --target "$triple"
		fi
		cp "$target_dir/$triple/release/mapcraft.exe" "$content/"
		lib="$(ls -t "$target_dir/$triple"/release/build/steamworks-sys-*/out/steam_api64.dll | head -1)"
		;;
	*)
		echo "unknown system: $os" >&2
		exit 1
		;;
	esac
	cp "$lib" "$content/"
	for id in $SDKS; do
		cp -r "sdks/$id" "$content/sdks/"
	done
	cp LICENSE ATTRIBUTION.md "$content/"
	cat >"$out/scripts/depot_$os.vdf" <<-EOF
		"DepotBuild"
		{
			"DepotID" "$depot"
			"ContentRoot" "../content/$os"
			"FileMapping"
			{
				"LocalPath" "*"
				"DepotPath" "."
				"Recursive" "1"
			}
		}
	EOF
	echo "$content"
done

# Every depot built so far goes in the app build, so a windows folder copied in from another machine uploads too.
{
	echo '"AppBuild"'
	echo '{'
	printf '\t"AppID" "%s"\n' "$STEAM_APP"
	printf '\t"Desc" "%s %s"\n' "$BRAND" "$(git describe --tags --always 2>/dev/null || echo dev)"
	printf '\t"BuildOutput" "../output"\n'
	printf '\t"Depots"\n\t{\n'
	for vdf in "$out"/scripts/depot_*.vdf; do
		depot="$(sed -n 's/.*"DepotID" "\(.*\)"/\1/p' "$vdf")"
		printf '\t\t"%s" "%s"\n' "$depot" "$(basename "$vdf")"
	done
	printf '\t}\n}\n'
} >"$out/scripts/app_build.vdf"
echo "$out/scripts/app_build.vdf"
