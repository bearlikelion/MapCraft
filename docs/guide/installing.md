# Install MapCraft

MapCraft runs on Linux, Windows and macOS, on any graphics driver with Vulkan, DirectX 12, Metal or OpenGL.

## Linux

1. Download `MapCraft-x86_64.AppImage` from the [latest release](https://github.com/bearlikelion/MapCraft/releases/latest).
2. Make it executable: in your file manager's **Properties**, allow running it as a program, or run `chmod +x MapCraft-x86_64.AppImage`.
3. Double click it.

The AppImage includes the SurfsUp, Counter-Strike: Source and Quake SDKs and their plugins, so there is nothing else to install.

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

## macOS

1. Download `MapCraft-macos.zip` from the [latest release](https://github.com/bearlikelion/MapCraft/releases/latest).
2. Double click it to unzip, and drag `MapCraft.app` into your Applications folder.
3. Double click `MapCraft.app`.

It needs macOS 11 or newer, on Apple Silicon or Intel, and includes the same SDKs and plugins.

> [!NOTE]
> MapCraft is not signed by Apple yet, so the first time you open it, macOS says it cannot check it for malicious software.
> Click **Done**, open **System Settings > Privacy & Security**, and click **Open Anyway** next to MapCraft.
> You only need to do this once.

## Check that it works

1. Choose **File > SDK Manager**.
2. Click **Test** next to SurfsUp.

The SurfsUp sample map opens: a curved surf ramp with a start and a finish.
If **SDK Settings** opens too, MapCraft needs the folder of your SurfsUp project; see [Your first map](first-map.md#2-tell-it-where-your-game-is).

![The SDK Manager listing Counter-Strike: Source, Quake and SurfsUp, with the Test button next to SurfsUp highlighted](img/install-sdk-manager.webp)

![The SurfsUp sample map open in MapCraft: a curved orange ramp over a lava floor in the 3D view, and the course from above in the Top view](img/install-sample.webp)

## Add the Godot addon

To play your maps in a Godot game such as SurfsUp, also download `MapCraft-godot-addon.zip` from the same release and unzip it into your Godot project's folder.
It works in Godot 4.4 and newer on Linux, Windows and macOS.
Then follow [Play it in Godot](godot.md).

## Other ways to get it

- To build MapCraft yourself, see [Building from source](../building.md).
- For more games, open **File > SDK Manager** or add folders of SDKs in **Edit > Preferences**; see [SDKs and plugins](sdks-and-plugins.md#where-mapcraft-looks-for-sdks).
