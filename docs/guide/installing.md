# Install MapCraft

MapCraft runs on Linux and Windows, on any graphics driver with Vulkan, DirectX 12 or OpenGL.
Download it, start it, and open the sample map to check that everything works.

## Linux

1. Download `MapCraft-x86_64.AppImage` from the [latest release](https://github.com/bearlikelion/MapCraft/releases/latest).
2. Make it executable: in your file manager's **Properties**, allow running it as a program, or run `chmod +x MapCraft-x86_64.AppImage`.
3. Double click it.

The AppImage already holds the SDKs MapCraft ships with (SurfsUp, Counter-Strike: Source and Quake) and their plugins, so there is nothing else to install.

> [!TIP]
> If it does not start because your system lacks FUSE, run it with `--appimage-extract-and-run`.

## Windows

1. Download `MapCraft-windows-x86_64.zip` from the [latest release](https://github.com/bearlikelion/MapCraft/releases/latest).
2. Unzip it anywhere, for example into your Documents folder.
3. Double click `MapCraft.exe`.

If Windows warns that the app comes from an unknown publisher, choose **More info**, then **Run anyway**.

> [!WARNING]
> Keep the `sdks` folder next to `MapCraft.exe`.
> That is where MapCraft finds its SDKs and their plugins.

## Check that it works

1. Choose **File > SDK Manager**.
2. Click **Test** next to SurfsUp.

The SurfsUp sample map opens: a curved surf ramp with a start and a finish.
If **SDK Settings** opens too, MapCraft is asking where your SurfsUp project is; [Your first map](first-map.md) walks you through it.

![The SDK Manager listing Counter-Strike: Source, Quake and SurfsUp, with the Test button next to SurfsUp highlighted](img/install-sdk-manager.webp)

![The SurfsUp sample map open in MapCraft: a curved orange ramp over a lava floor in the 3D view, and the course from above in the Top view](img/install-sample.webp)

## Add the Godot addon

To play your maps in a Godot game such as SurfsUp, also download `MapCraft-godot-addon.zip` from the same release and unzip it into your Godot project's folder.
[Play it in Godot](godot.md) has the rest.

## Other ways to get it

- There is no macOS download yet.
- To build MapCraft yourself, see [Building from source](../building.md).
- For more games, open **File > SDK Manager** or add folders of SDKs in **Edit > Preferences**; see [SDKs and plugins](sdks-and-plugins.md#where-mapcraft-looks-for-sdks).
