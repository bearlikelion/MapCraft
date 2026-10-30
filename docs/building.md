# Building from source

MapCraft is a Rust workspace; its first-party plugins are a separate workspace built for WebAssembly.
Most people should use the [downloads](guide/installing.md) instead.

## Requirements

- Rust 1.95 or newer, with the `wasm32-wasip2` target for the plugins.
- A graphics driver with Vulkan, DirectX 12 or OpenGL.
- On Debian and Ubuntu, the windowing headers:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev
```

## Build and run

```sh
git clone --recursive https://github.com/bearlikelion/MapCraft.git
cd MapCraft
rustup target add wasm32-wasip2
cargo xtask plugins
cargo run --release -p mapcraft
```

- `sdks/` is the [MapCraftSDKs](https://github.com/bearlikelion/MapCraftSDKs) repository as a git submodule; in a clone made without `--recursive`, run `git submodule update --init` first.
- `cargo xtask plugins` builds the plugins in `plugins/` and copies each into the SDKs that list it, as `plugins/<id>.wasm`.
- A build run from the source tree finds the SDKs in `sdks/` by itself.
- On Linux, `cargo xtask desktop [path to the mapcraft binary]` installs a desktop entry and icons for a source build, so it shows in the app launcher.

The headless tools are in `mapcraft-cli`:

```sh
cargo build --release -p mapcraft-cli
```

## More SDK folders

Besides the folders it always searches, MapCraft reads SDKs from each folder in the `MAPCRAFT_SDK_PATH` environment variable, a path list like `PATH`, and from the extra SDK folders in **Edit > Preferences**.
[SDK manifests](sdk-format.md#where-sdks-are-found) has the full search order.

## Release packages

The packaging scripts write to `dist/`; the first three build the editor first.
The packages hold no SDKs: on its first launch MapCraft offers to install one from the [SDK registry](sdk-registry.md#the-first-launch), so the plugins and the `sdks/` submodule are not needed to package it.

- `packaging/appimage.sh` writes `MapCraft-x86_64.AppImage`.
  It needs `appimagetool` on `PATH`, or its path in `APPIMAGETOOL`.
- `packaging/windows-zip.sh [target]` writes `MapCraft-windows-x86_64.zip`.
  It cross builds for `x86_64-pc-windows-gnu` (MinGW) unless given a target.
- `packaging/macos-app.sh` writes `MapCraft-macos.zip`, a universal (Apple Silicon and Intel) `MapCraft.app`.
  It runs on a Mac with the `aarch64-apple-darwin` and `x86_64-apple-darwin` Rust targets, joins the two builds with `lipo`, makes the icon with `iconutil`, and signs the app ad hoc, since Apple Silicon only runs signed code.
- `packaging/godot-addon.sh` writes `MapCraft-godot-addon.zip` from `godot/addons/mapcraft`, after `godot/build.sh` has built its libraries.

Pushing a `v*` tag runs the Release workflow, which builds the AppImage on Ubuntu 22.04 (to keep the glibc requirement low), the Windows zip with MSVC, the macOS app, and the Godot addon with its Linux, Windows and macOS libraries (the macOS one built on a Mac), then publishes all four as a GitHub release.
Running the workflow by hand builds everything without publishing, to try a release first.

## Checks

Run `cargo xtask ci` before committing: it runs rustfmt, clippy with warnings as errors, the tests, and the layer check described in [Architecture](architecture.md).
