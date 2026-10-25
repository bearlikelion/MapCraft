# Your first map

This walkthrough builds a small surf course with the SurfsUp SDK, saves it and sends it to the game.
Keys are the MapCraft preset's defaults; see [Keyboard shortcuts](shortcuts.md).

## Start a map

1. Choose **File > New Map** (`Ctrl+N`).
2. Pick **SurfsUp** in the SDK list and click **Create**.

If the current map has unsaved changes, MapCraft asks whether to save them first.
When MapCraft starts without a file, it opens a new map with the SDK you used last.

## SDK Settings opens by itself

The SurfsUp SDK reads its materials from the `Assets/Textures` folder of your SurfsUp Godot project, so it cannot list any until it knows where that project is.
While such a path is unset, **SDK Settings** opens on its own and the status bar says which path to set.

1. Click **Browse** next to **SurfsUp Godot project folder** and pick the project folder.
2. Click **Apply**, or just close the window: closing applies the change too.

A path that does not exist shows in red.
The other paths (Godot editor, game, Maps folder, Counter-Strike: Source folder) only matter for the Run menu and for BSP import, so you can set them later with **File > SDK Settings** (`Ctrl+Shift+Comma`).
See [SDKs and plugins](sdks-and-plugins.md#sdk-settings) for what each one is for.

## Draw a block

1. Press `B` for the Block tool.
2. Drag a rectangle in the Top view.

The new block takes the depth of the last brush you drew (64 units at first) and the material shown in the top bar.
In the 3D view you can drag on the ground or on any face instead, holding `Shift` to drag the height.
Pick another material in the Materials tab (`Ctrl+3`), and double-click one to put it on the selection.
See [Tools](tools.md), [Views and navigation](views.md) and [Materials](materials.md) for the details.

## Place a surf ramp

With the Block tool active, the top bar lists the plain shapes followed by the SDK's generators: **Surf Ramp**, **Bhop Platforms**, **Surf Window** and **Surf Stage**.

1. Click **Surf Ramp**.
   Its parameters appear in the inspector and a ghost of the ramp follows the pointer.
2. Set **Turn**, or click **90°** for a quarter turn.
3. Click in a view to place it: on the ground or a face in the 3D view, or at the last brush's depth in a 2D view.

The placed ramp is a group that remembers its parameters.
Press `Q` for the Select tool, click the ramp (in the default **Groups** selection mode) and change any parameter in the inspector: the ramp is rebuilt as you type.
[Surf mapping](../surf-mapping.md) explains each parameter and the angles that keep a ramp surfable.

**Surf Stage** is a quick way to start a course: it makes start and end platforms with their zones, an arrival point and a kill zone under the stage.

## Add a spawn

1. Press `N` for the Entity tool.
   It places `info_player_start` until you pick another class in the Entities tab.
2. Click on top of your start platform in the 3D view.

The spawn becomes the game's spawn point.
See [Entities and zones](entities.md) for the inspector and the other classes.

## Add zones

1. Open the Entities tab (`Ctrl+4`) and click `zone_start`.
2. Select the start platform.
3. Choose **Tools > Zone from Selection**.

The zone covers the platform's top and stands 256 units tall.
Do the same with `zone_finish` on the last platform.


## Save

Press `Ctrl+S`.
The first save asks where to put the map; MapCraft maps are `.mcmap` files.
**File > Save As** (`Ctrl+Shift+S`) saves a copy under a new name.
The window title starts with `*` while there are unsaved changes.

## Autosave and crash recovery

- Every 5 minutes, while there are unsaved changes, MapCraft writes `<map name>.autosave.mcmap` (`untitled` for a new map) into the `autosave` folder of its data folder: `~/.local/share/mapcraft/autosave` on Linux.
- Change the interval with **Autosave every** in **Edit > Preferences** (`Ctrl+Comma`); 0 turns autosave off.
- Saving the map, or quitting normally, deletes its autosave.

If MapCraft did not close cleanly, the next start shows **Recover unsaved work?** with the autosaves it found, newest first.

- **Open** loads one as an untitled map with unsaved changes, so the next save asks for a name.
- **Delete autosaves** removes them all.
- **Not now** closes the prompt and leaves the files where they are.

## Check for problems

Press `Alt+P`, choose **Tools > Check for Problems**, or click **Check** at the right of the top bar.
The Issues tab opens below the views with errors in red and warnings in yellow.

- Click an issue to select the object and focus the views on it.
- **Fix** repairs one issue and **Fix all** repairs every fixable one, as one undo step.
- **Check again** runs the checks once more.

## Run it

The **Run** menu, and the **Run** button at the right of the top bar, list the SDK's compile profiles.
The SurfsUp SDK has **Import into Godot**, **Build map pack** and **Build and play**.

- Running a profile saves the map first.
- The first time you run anything from an SDK, MapCraft asks **Run this SDK's programs?**; click **Allow** if you trust the SDK, and MapCraft remembers it.
- Progress shows in the Output panel.
- `F9` runs the last profile again, `Shift+F9` cancels a running one.

The SurfsUp profiles need the MapCraft addon in your Godot project and a few paths in SDK Settings: see [The Godot workflow](godot.md).

## Where next

- [Organizing](organizing.md): groups, layers, the scene tree, tags and cordons.
- [Mirror groups](mirror-groups.md): build half a map and get the other half for free.
- [Prefabs](prefabs.md): reuse pieces across maps.
- [Importing BSP maps](importing-bsp.md): zone an existing Counter-Strike: Source surf map.
