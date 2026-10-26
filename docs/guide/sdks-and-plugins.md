# SDKs and plugins

MapCraft knows no game by itself.
An **SDK** describes one game, and **plugins** add the code a game needs.

## What an SDK is

An SDK is a folder with an `sdk.toml` file that says:

- which entity classes exist (FGD files);
- how far maps may reach, when the game has a limit (Quake's is -4096 to 4096 on every axis, Source's -16384 to 16384);
- where materials live, which material new brushes get and at what scale;
- which plugins to load;
- which folders and programs the user has to point it at;
- how to compile and launch a map (compile profiles, shown in the Run menu).

Game content stays in your own game install or project; the SDK only points at it.
Each map stores the id of the SDK it was made with, and opening the map loads that SDK.
When it is not installed, the status bar says so and entities and materials are unavailable.

[SDK manifests](../sdk-format.md) is the full reference for writing one.

## Where MapCraft looks for SDKs

MapCraft looks for SDK folders directly inside these folders, in this order:

1. `sdks` in MapCraft's data folder: `~/.local/share/mapcraft/sdks` on Linux, `%LOCALAPPDATA%\mapcraft\sdks` on Windows, `~/Library/Application Support/mapcraft/sdks` on macOS.
   Installed and newly created SDKs go here.
2. An `sdks` folder next to the MapCraft program, or in any of the four folders above it, so a build in `target/release` finds the source tree's SDKs.
3. `share/mapcraft/sdks` beside the program's folder, where installed packages put them.
4. The `sdks` folder of the source tree MapCraft was built from.
5. Each folder in the `MAPCRAFT_SDK_PATH` environment variable, a path list like `PATH`.
6. The **Extra SDK folders** in **Edit > Preferences** (`Ctrl+Comma`); **Add folder** and **Remove** rescan right away.

Each folder counts once.
When two folders hold an SDK with the same id, the one found first wins.
With a map that has no SDK, **File > SDK Settings** lists the folders searched.

**File > SDK Manager** lists the SDKs found, with a **Check** of each one, and installs more from SDK registries; see [SDK registries](../sdk-registry.md).

## SDK Settings

**File > SDK Settings** (`Ctrl+Shift+Comma`) sets the folders and programs the current SDK asks for.

- Type a path or click **Browse**, which picks a file or a folder as the SDK expects.
- A path that does not exist shows in red.
- Paths that materials or entities depend on come first; hover their name to see that.
- While one of those is unset, the window opens by itself when the SDK loads.
- **Apply**, or closing the window, reloads the SDK with the new paths.

Paths are saved per SDK in your preferences, so every map made with that SDK shares them.

The SurfsUp SDK asks for:

| Setting | Used for |
|---|---|
| SurfsUp Godot project folder | Materials (`Assets/Textures`), and where the Run menu copies the map |
| Godot editor executable | Importing and packing the map in the Run menu |
| SurfsUp game executable | **Build and play** |
| Folder SurfsUp loads map packs from | Where **Build map pack** writes the `.pck` file |
| Counter-Strike: Source game folder | Stock textures of [imported BSP maps](importing-bsp.md) |

## Compile profiles

The **Run** menu and the **Run** button at the right of the top bar list the SDK's compile profiles.
A profile is a list of steps: copy a file, or run a program with arguments built from the SDK paths and the map's path and name.

- Running a profile saves the map first, asking for a file name if it has none.
- The Output panel opens and shows each command and its output, then `== finished ==` or `== failed: ... ==`.
- A profile stops at the first step that fails, and a path left unset stops it with an error.
- `F9` (**Run Last Profile**) runs the last profile again, or the first one if none has run yet.
- `Shift+F9` (**Run > Cancel**) stops a running profile.

### The trust prompt

Compile profiles run programs the SDK names, so the first time you run one from an SDK, MapCraft asks **Run this SDK's programs?**.
**Allow** runs it and remembers the answer for that SDK; **Cancel** runs nothing.
Only allow SDKs you trust.

## Plugins

Plugins are WebAssembly components, so they can be written in Rust, C, C++, C#, Go, Python, JavaScript or Zig.
They run sandboxed:

- no network, no other programs and no environment variables;
- read-only access to the SDK's material folders and the extra game folders it lists, and nothing else on disk;
- a time and memory budget per call, so a plugin that runs away is stopped with an error instead of freezing the editor;
- a plugin that crashes is turned off until the SDK loads again.

### Where plugins come from

- Each plugin id in the SDK's `plugins` list loads from `plugins/<id>.wasm` in the SDK folder.
- Every `.wasm` file in the `plugins` folder of MapCraft's data folder (`~/.local/share/mapcraft/plugins` on Linux) loads too, whatever the SDK, even for a map with no SDK.
- If two plugins share an id, the first one loaded (the SDK's) wins.

Problems loading a plugin are listed in the Output panel (`Ctrl+5`).

### What plugins add

- **Commands** in the **Plugins** menu.
  Each opens a window with its parameters; **Create** puts the result at the center of the selection, or where the pointer last was, snapped to the grid.
  The bottom of the menu lists the loaded plugins and their versions.
- **Generators**, commands whose output stays editable.
  They also show as shapes of the Block tool, with a ghost to place them by clicking.
  The result is a group that keeps the parameters: select it and change any parameter in the inspector to rebuild it, as one undo step.
  Moving or turning the group moves the generator too, and the scene tree does not let you drag other objects into it.
  When the plugin is not loaded, the inspector says which plugin made the group, and its brushes can still be edited by hand.
- **Importers**, listed under **File > Import**.
  The imported content goes into a new layer named after the file.
- **Material lists**, which replace the plain list of images in the SDK's material folders.

The first-party plugins:

| Plugin | Adds |
|---|---|
| `godot` | Lists a Godot project's `.tres` materials, previewed by their texture or color, plus images no material uses |
| `surfsup` | Surf Ramp, Bhop Platforms, Surf Window and Surf Stage generators; see [Surf mapping](../surf-mapping.md) |
| `source-bsp` | **File > Import > Source BSP**; see [Importing BSP maps](importing-bsp.md) |
| `source` | Lists a Source game's materials, and **File > Import** and **File > Export** of Hammer `.vmf` files; see [Counter-Strike: Source](cstrike.md) |

To write your own, see the [plugin API](../plugin-api.md); `plugins/stairs-py` is a small generator written in Python.
