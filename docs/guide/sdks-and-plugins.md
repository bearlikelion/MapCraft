# SDKs and plugins

MapCraft does not know any game by itself.
An **SDK** describes one game, and **plugins** add the code that game needs, such as surf ramp generators and map importers.
This page shows how to pick and set up an SDK, play your map in the game, and use what plugins add.

## What an SDK is

An SDK is a folder that tells MapCraft about one game:

- which entities exist, such as spawns, triggers and lights (entities are the things in a map that are not plain geometry);
- where the game's materials are, and which one new brushes (the solid blocks a map is built from) get;
- how far a map may reach, for games with a limit (Quake's is -4096 to 4096 on every axis, Source's -16384 to 16384);
- which plugins to load;
- which folders and programs on your computer it needs;
- how to compile and play a map, listed in the **Run** menu.

Your game's files stay where they are: the SDK only points at them.
SDKs for SurfsUp, Counter-Strike: Source and Quake install from the SDK Manager; MapCraft offers them the first time it starts.
To write your own, see [SDK manifests](../sdk-format.md).

## Each map remembers its SDK

You pick the SDK when you make a map with **File > New Map**.
The map remembers it, so opening the map loads that SDK again.
If the SDK is not installed, the status bar says so, and entities and materials are unavailable.

### Switch a map's SDK

1. Click the SDK name in the status bar at the bottom of the window, or choose **File > Switch SDK**.
2. Pick another SDK, or **None**.

MapCraft reloads materials, entities, plugins and settings for the new SDK.
The map keeps all its brushes and entities.

- If the new SDK needs folders you have not set yet, **SDK Settings** opens by itself.
- If the map uses entities the new SDK does not define, the status bar names them, and **Check for Problems** (`Alt+P`) lists each one.

> [!NOTE]
> The SDK is saved with the map, so switching makes the map unsaved.
> Switching is not an undo step: to go back, switch to the old SDK again.

![The New Map window with its SDK list open: None, Counter-Strike: Source, Quake and SurfsUp](img/sdks-new-map.webp)

## The SDK Manager

**File > SDK Manager** lists every SDK MapCraft found, with these buttons on each:

- **Configure** opens the SDK's settings.
- **Check** tests the SDK and lists what passed, what has warnings and what failed.
- **Test** opens the SDK's sample map, or checks the SDK when it has none.
- **Open Folder** shows the SDK's folder.
- **Remove…**, on SDKs you installed or created, deletes the SDK's folder after asking.

![The SDK Manager after clicking Check next to SurfsUp: 10 passed, 3 warnings and 0 failed, with a line for each check](img/sdks-manager-check.webp)

The **Available** tab installs more SDKs from SDK registries, which are online lists of SDKs.
Each SDK shows as a card with its logo, version, description, author and size, and an **Install…** button, or **Update…** when a newer version is out.
Once an SDK is installed, its **SDK Settings** open so you can point it at your game.
If the open map is untitled, still empty and has no SDK, it starts over on the new SDK; any other map keeps its own, and **File > New Map** starts one on the new SDK.
The **Create** tab starts a new SDK from a template, or [imports a TrenchBroom game made with func_godot](trenchbroom.md).
To host your own registry, see [SDK registries](../sdk-registry.md).

### The first launch

MapCraft comes without SDKs, so the first time it starts, the **Add an SDK** window opens with the same cards as the **Available** tab.
Install the SDK for your game there, or click **Not now**; the window comes back at the next launch until an SDK is installed.
If the list does not load, for example without internet, it says so and offers **Retry**.

## Where MapCraft looks for SDKs

SDKs you install or create in the SDK Manager go into the `sdks` folder of MapCraft's data folder:

| System | Folder |
|---|---|
| Linux | `~/.local/share/mapcraft/sdks` |
| Windows | `%LOCALAPPDATA%\mapcraft\sdks` |
| macOS | `~/Library/Application Support/mapcraft/sdks` |

To use SDKs kept somewhere else, open **Edit > Preferences** (`Ctrl+Comma`) and add the folder that holds them under **Extra SDK folders**.
Clicking **Add folder…**, or the trash can button (**Remove this folder**) next to a folder, rescans right away.

> [!NOTE]
> When two folders hold an SDK with the same id, the one found first wins.
> With a map that has no SDK, **File > SDK Settings** lists every folder searched.
> The full search order, including the `MAPCRAFT_SDK_PATH` variable, is in [SDK manifests](../sdk-format.md#where-sdks-are-found).

![Edit > Preferences with the Extra SDK folders section and its Add folder button highlighted](img/sdks-extra-folders.webp)

## SDK Settings

**File > SDK Settings** (`Ctrl+Shift+Comma`) tells the current SDK where things are on your computer, such as the game's folder or its executable.

- Type a path, or click **Browse…** to pick the file or folder.
- A path that does not exist turns red.
- Click **Apply**, or close the window, to reload the SDK with the new paths.

Settings that materials or entities need come first; hover a name to see why.
While one of those is empty, the window opens by itself when the SDK loads.
Paths are saved per SDK, so every map made with that SDK shares them.

![SurfsUp SDK Settings with three paths; the SurfsUp game executable path is red and outlined because that file does not exist](img/sdks-settings.webp#wide)

### The SurfsUp settings

These are the paths the SurfsUp SDK asks for, and what each one is for:

| Setting | Used for |
|---|---|
| SurfsUp Godot project folder | More materials (from `Assets/Textures`) and props |
| SurfsUp game executable | **Play in SurfsUp**, which copies the map into the `Maps` folder next to it and makes that folder if needed |
| Counter-Strike: Source game folder | Stock textures of [imported BSP maps](importing-bsp.md) |

### Texture folders

The **Texture folders** list, below the SDK's own paths, adds more material folders, or single files, to any SDK.
Use it when your textures live somewhere the SDK does not look.

- Type a folder or a file and click **Add**.
- Or click **Browse…** to pick a folder, or **File…** to pick one file, such as a texture WAD for the Quake SDK.
- An entry that does not exist turns red.
  The trash can button (**Take it off the list**) removes it.
- Click **Apply**, or close the window, to reload the materials.

Materials in an added folder are named by their path inside it, without the extension, the same way as in the SDK's own folders.
An added file goes by its own name, as if it sat at the top of a folder.
Plugins read the added folders and files too.
The Godot plugin lists the `.tres` materials in them as well as their pictures, and the Quake plugin lists the textures in every added WAD, each WAD in its own folder.

For the SDK of the open map, **Folders shown in the Materials panel** lists its top-level material folders, each with a checkbox.
It is the same list as **Folders** in the [Materials panel](materials.md#the-materials-panel).
Untick a folder to hide it for every map on this SDK; faces that use it still draw.
**Show all** and **Hide all** above the list tick or untick every folder at once.

The SDK's own folders are searched first, then the added ones in list order, so when two folders hold the same name, the first one wins.
The list is saved per SDK, under `sdk_textures` in MapCraft's `config.toml`, and unticked folders under `hidden_textures`.
Pictures that imports bring in are kept per SDK too, so a Quake map's textures only show on the SDK it was imported with.

> [!NOTE]
> The game has to find the same names.
> Godot looks materials up under the import's [Material Root](godot.md#import-options), `res://Assets/Textures` for SurfsUp, so a material from another folder shows in MapCraft but not in the game until it is there too.

## Compile profiles

A compile profile builds or plays your map: it copies files and starts programs, one step after another.
Pick one from the **Run** menu, or from the orange play button at the top right (**Compile and play the map**).

- Running a profile saves the map first, and asks for a file name if it has none.
- The Output panel opens and shows each step, then `== finished ==` or `== failed: ... ==`.
- `F9` (**Run Last Profile**) runs the last profile again, or the first one if none has run yet.
- `Shift+F9` (**Run > Cancel**) stops a running profile.
- A profile stops at the first step that fails, and a path left empty in SDK Settings stops it with an error.
- When a compile leaks, the leak trail it writes is loaded and drawn in the views (see [Leak trails](views.md#leak-trails)).

![The Run menu with the SurfsUp profile Play in SurfsUp, then Run Last Profile and Cancel](img/sdks-run-menu.webp)

![The Output panel after Play in SurfsUp: the copy step, the game starting with +map surf_mapcraft, then == finished ==](img/sdks-output.webp#wide)

### The trust prompt

Profiles run programs the SDK names, so the first time you run one from an SDK, MapCraft asks **Run this SDK's programs?**
**Allow** runs it and remembers your answer for that SDK.
**Cancel** runs nothing.

> [!WARNING]
> Only allow SDKs you trust.

![The Run this SDK's programs? prompt with Allow and Cancel buttons](img/sdks-trust.webp)

## Plugins

Plugins add commands, generators, importers, exporters and material lists, each covered below.
An SDK loads the plugins it lists from its own `plugins` folder.
Every `.wasm` file in the `plugins` folder of MapCraft's data folder (`~/.local/share/mapcraft/plugins` on Linux) loads too, for every map, even one with no SDK.
If two plugins share an id, the SDK's wins.
Problems loading a plugin show in the Output panel (`Ctrl+5`).

Plugins can be written in Rust, C, C++, C#, Go, Python, JavaScript or Zig, and they run in a sandbox:

- They have no network, cannot start other programs, and cannot read environment variables.
- They can only read the SDK's material folders and the game folders it lists.
- A plugin that runs too long or uses too much memory is stopped with an error instead of freezing the editor.
- A plugin that crashes is turned off until the SDK loads again.

To write your own, see the [Plugin API](../plugin-api.md).
`plugins/stairs-py` is a small generator written in Python.

### Commands

Plugin commands live in the **Plugins** menu.
Each one opens a window with its settings.
Click **Create** to put the result at the center of the selection, or where the pointer last was, snapped to the grid.
The bottom of the menu lists the loaded plugins and their versions.

![The Plugins menu with Surf Ramp and Bhop Platforms, then the loaded plugins Godot, Source BSP and SurfsUp with their versions](img/sdks-plugins-menu.webp)

### Generators

Generators are commands whose result stays editable, such as Surf Ramp.
They also appear as shapes of the Block tool, with a ghost (a see-through preview) that you place by clicking.

- The result is a group that remembers its settings.
  Select it and change any of them in the inspector to rebuild it, in one undo step.
- Moving or turning the group moves the generator too.
- Resizing the group keeps its new size when you change a setting; a plugin that supports it, such as SurfsUp's Surf Ramp, also updates the settings to the new size.
- The scene tree does not let you drag other objects into the group.
- When the plugin is not loaded, the inspector names the plugin that made the group, and you can still edit its brushes by hand.

![The Block tool's shapes on the top bar, with the SurfsUp generators Surf Ramp and Bhop Platforms highlighted after the Box, Wedge, Cylinder, Spike, Sphere and Arch icons](img/sdks-generators.webp#wide)

### Importers and material lists

Importers appear under **File > Import**, and exporters under **File > Export**.
What an importer brings in goes into a new layer named after the file.
Dropping a file on the MapCraft window also runs the importer for its extension.

Material lists replace the plain list of images from the SDK's material folders.
The Godot plugin does this with a project's `.tres` materials.

![The File menu with Import open, showing the Source BSP and SurfsUp SDK map importers](img/sdks-import-menu.webp)

### The first-party plugins

The official SDKs bring these plugins with them:

| Plugin | Adds |
|---|---|
| `godot` | Lists a Godot project's `.tres` materials, previewed by their texture or color, plus images no material uses |
| `surfsup` | The Surf Ramp and Bhop Platforms generators (see [Surf ramps and bhop](surf.md)), and **File > Import > SurfsUp SDK map** for maps made with the SurfsUp SDK in Godot (see [Import a SurfsUp SDK map](importing-sdk.md)) |
| `source-bsp` | **File > Import > Source BSP**; see [Import a BSP map](importing-bsp.md) |
| `source` | A Source game's materials, and **File > Import** and **File > Export** of Hammer `.vmf` files; see [Counter-Strike: Source](cstrike.md) |
| `quake` | Textures from Quake WAD files, Quake's models in the views, and **File > Import > Quake map** and **File > Export > Quake map (Valve220)**; see [TrenchBroom and func_godot](trenchbroom.md) |
