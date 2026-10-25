# Installing

MapCraft runs on Linux and Windows.
It draws with wgpu, so it needs a graphics driver with Vulkan, DirectX 12 or OpenGL.

## Linux

1. Download `MapCraft-x86_64.AppImage` from the latest release on GitHub.
2. Make it executable: `chmod +x MapCraft-x86_64.AppImage`.
3. Run it.

The AppImage carries the first-party SDKs and their plugins, so there is nothing else to install.
If it does not start because your system lacks FUSE, run it with `--appimage-extract-and-run`.

## Windows

1. Download `MapCraft-windows-x86_64.zip` from the latest release on GitHub.
2. Unzip it anywhere, for example into your Documents folder.
3. Run `MapCraft.exe`.

Keep the `sdks` folder next to `MapCraft.exe`: that is where MapCraft finds its SDKs and plugins.
Windows may warn that the app comes from an unknown publisher; choose **More info > Run anyway**.

## The Godot addon

To load maps in a Godot project, download `MapCraft-godot-addon.zip` from the same release and unzip it into the project's root folder.
See [The Godot workflow](godot.md).

## Build from source

You need Rust 1.95 or newer and the WebAssembly target for the plugins.

```sh
git clone --recursive https://github.com/bearlikelion/MapCraft.git
cd MapCraft
rustup target add wasm32-wasip2
cargo xtask plugins
cargo run --release -p mapcraft
```

`sdks/` is the [MapCraftSDKs](https://github.com/bearlikelion/MapCraftSDKs) repository as a git submodule; in a clone made without `--recursive`, run `git submodule update --init` first.
`cargo xtask plugins` builds the plugins in `plugins/` and copies each into the SDKs that use it.
A build run from the source tree finds the SDKs in `sdks/` by itself.

On Debian and Ubuntu, install the windowing headers first:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev
```

To make the release packages yourself, run `packaging/appimage.sh` or `packaging/windows-zip.sh`; they write to `dist/`.

## More SDKs

MapCraft also looks for SDKs in the folders listed under `MAPCRAFT_SDK_PATH` and in the extra folders you add in **Edit > Preferences**.
See [SDKs and plugins](sdks-and-plugins.md).
