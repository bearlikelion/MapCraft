# User guide

This guide shows you how to build maps with MapCraft, from your first block to playing the map in your game.
Never used mapping software? Start with the [Tutorial](../tutorial/README.md), which builds a room from scratch.
If you have mapped before, [install MapCraft](installing.md) and read [Getting around](views.md), then pick the pages you need.

## The editor at a glance

![The MapCraft window: the top bar, the panels, the tool column, the four views, the inspector and the status bar](img/editor-overview.webp#wide)

1. **Top bar**: the menus, the selection mode, the grid size, snapping and the current material.
   **Check** (a checklist) and **Run** (an orange play button) sit at its right end.
   Hover any icon to see its name and shortcut.
2. **Panels**: the Scene tree, Tags, Materials, Entities and Models tabs, shown as icons.
3. **Tools**: Select, Move, Extrude, Rotate, Scale, Block, Entity, Clip, Vertex, Face, Sculpt and Path.
4. **Views**: 3D, Top, Front and Side.
   Press `Space` to fill the window with the one under the pointer.
5. **Inspector**: shows the settings of whatever is selected, and lets you change them.
6. **Status bar**: what is selected, the grid, and a hint for the current tool.


## Getting started

- [Install MapCraft](installing.md): download it for Linux, Windows or macOS, and add the SDK for your game.
- [Tutorial: your first room](../tutorial/README.md): never mapped before? Build a room from scratch, one step at a time.
- [Getting around](views.md): the four views, the 3D camera, the grid and snapping.
- [Hotkeys](hotkeys.md): the keys to learn first, shown on a keyboard.

## Building

- [Tools](tools.md): every tool in the tool column, and the right click menu.
- [Materials](materials.md): pick, apply and line up textures.
- [Entities and zones](entities.md): spawns, triggers, zones, lights and props.
- [Surf ramps and bhop](surf.md): place and tune surf ramps and bunny hop platforms.
- [Paths, sweeps and spins](curves.md): curved roads, rails, arches and round towers.
- [Boolean groups](boolean-groups.md): cut live holes, such as a round window, that follow when you move the cutting brush.
- [Terrain](displacements.md): sculpt uneven ground with displacements.

## Organizing

- [Organizing](organizing.md): selection modes, groups, layers, the scene tree, tags and cordons.
- [Mirror groups](mirror-groups.md): build one half of a symmetric map and MapCraft draws the other half.
- [Prefabs](prefabs.md): reuse a piece of a map across maps.

## Games

- [Play it in Godot](godot.md): send your map to a Godot game such as SurfsUp.
- [TrenchBroom and func_godot](trenchbroom.md): turn your func_godot game's TrenchBroom setup into an SDK, and send maps back to Godot.
- [Import a BSP map](importing-bsp.md): bring in a compiled Counter-Strike: Source map to zone it.
- [Import a SurfsUp SDK map](importing-sdk.md): bring in a map made with the SurfsUp SDK in Godot.
- [Blender and glTF](gltf.md): bring in a Blender blockout as brushes, or send a map to any 3D tool.
- [Counter-Strike: Source](cstrike.md): make CS:S maps with Hammer `.vmf` files and Valve's compilers.
- [SDKs and plugins](sdks-and-plugins.md): how MapCraft supports a game, and how to add more games.

## Reference

- [Keyboard shortcuts](shortcuts.md): every shortcut, the Hammer and TrenchBroom presets, and how to change them.
- [Technical docs](../README.md): building from source, SDKs, plugins and file formats; you do not need them to make maps.
