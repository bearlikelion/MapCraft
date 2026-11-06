# MapCraft for Godot

A C++ GDExtension that loads MapCraft maps (`.mcmap`) in Godot 4.4 and newer, from GDScript or C#.
For playing your map in SurfsUp from the editor, see the user guide's [Play it in Godot](../docs/guide/godot.md).

## Install

1. Download `MapCraft-godot-addon.zip` from the [latest MapCraft release](https://github.com/bearlikelion/MapCraft/releases/latest), or [build it yourself](#building).
2. Put its `addons/mapcraft` folder in your Godot project.
3. Open the project in the Godot editor once, so Godot registers the extension.

## Load a map

### Import as a scene

Every `.mcmap` in the project imports as a scene, so you can instance, open or load it like any `.tscn`.
The scene uses only Godot's built-in nodes, so the game needs neither this extension nor any script to load it.
To change how a map imports, use the Import dock; the guide lists the [import options](../docs/guide/godot.md#import-options).

### Edit a map in Godot

Godot opens an imported map read-only, so there are two ways to keep changes made in the editor:

- **Inherited scene:** double-click the map and pick **New Inherited**, or use **Scene > New Inherited Scene**.
  Scripts, added nodes and changed properties, such as a light's energy, are saved in the new scene, and every export from MapCraft still comes through.
  Godot does not let an inherited scene move or delete the map's own nodes.
- **Editable scene:** right-click a `.mcmap` or `.mcpck` in the FileSystem dock and pick **Save as Editable Scene...**.
  It writes a plain `.tscn` (or `.scn`) the project owns and opens it, so anything can be moved, deleted, scripted or relit.
  Exporting from MapCraft does not change it; save it again to start over from the map.
  A pack's pictures and models have no files of their own, so its editable scene carries them.

From a script, `MapCraftMap.save_as_scene(map_path, scene_path)` does the same and returns an `Error`.

### Load at runtime

For maps in user folders or downloads, with no import step:

- `load()` turns any `.mcmap` path, inside or outside the project, into a scene, built like an import with the default options.
- A `MapCraftMap` node (`Node3D`) builds a map as its children: set `map_file`, then call `build()` or leave `build_on_ready` on.
  It has the importer's settings and emits `entity_built(node, classname, properties)` for each entity, so a game can finish it off.
  Unlike an import, it does not pick the SDK's entity map on its own, so set `entity_map`.
  `build()` first removes the node's children, and `clear()` does only that.

## How maps become nodes

- Layers and groups become `Node3D` nodes named after them, nested as they are in MapCraft, so the scene keeps the map's hierarchy.
- Brushes outside any group become one mesh, `WorldMesh`, with a surface per material, and one collision body, `WorldCollision`, with a shape per brush.
  A layer or group's own brushes become its `Mesh` and `Collision` children instead.
- With trimesh collision, every world brush, grouped or not, collides as one triangle mesh in `WorldCollision`, so seams between ramp pieces in different groups cannot catch players.
- Faces with `tools/` materials (`tools/nodraw`, `tools/skip`, `tools/trigger`) collide but are not drawn.
- A brush with `tools/clip` on any face collides but draws none of its faces.
- Point entities become `Marker3D` nodes at their origin, facing the way the entity faces.
- Brush entities become `Area3D` nodes centered on their brushes, with a collision shape per brush.
- An [entity map](#entity-maps) can make an entity a different node, a scene, a model or part of the world instead.
- Entity keys become metadata on the node, typed as numbers, vectors, booleans or text (see [Details](#details)).
- Prefab instances are built from the prefab's `.mcmap` file into a `Node3D` named after it, up to 16 prefabs deep and 256 copies per map; a prefab that contains itself is left out.
- Everything outside an active cordon is left out.
- Maps are Z up in inches; Godot gets Y up in meters, at 39.37 units per meter by default.

## Materials

A face's material name is looked up under the `material_root` import option (`res://Assets/Textures` by default).
It finds `<name>.tres`, `.res` or `.material` first, then an image (`png`, `jpg`, `jpeg`, `webp`, `tga`, `bmp`) that becomes a `StandardMaterial3D`.

Anti-tiled faces and blended displacements draw through the addon's shaders in `addons/mapcraft/shaders`, so the game needs that folder too.
Only a plain `StandardMaterial3D` can take these shaders; any other material draws as it is, with a warning.
A map's embedded shaders ([the format](../docs/mcmap-format.md#embedded-shaders)) become a `ShaderMaterial`: the world's sky shader is the environment's sky, and a brush's spatial shader draws all of that brush's faces.
Their `shader_params` set the uniforms, and a `sampler2D` parameter names a material and gets its albedo picture, from the map's pack first and then the game.
The guide covers [anti-tiling](../docs/guide/godot.md#anti-tiling), [blended terrain](../docs/guide/godot.md#blended-terrain) and [which materials qualify](../docs/guide/godot.md#shader-wrappers).

## Map packs

A map built from a file picks up `<map>.mcpck` beside it: its pictures become the map's materials and its `.glb` models stand in for `model` paths, for that map only.
The pack is read into memory and checked first; nothing in it is extracted, given a path or cached, a pack with a broken container is ignored with an error in the log, and an entry that fails its checks draws as missing.
A map without one uses `<map>.mcpack`, the name older MapCraft versions wrote.
A version 3 pack can hold the map too: `load("<name>.mcpck")` or a `MapCraftMap` node whose `map_file` is the pack builds that map with the pack's own pictures and models.
[The format](../docs/mcpck-format.md) lists what a pack may hold, every limit and the threat model.

For maps downloaded from players, build them with a `MapCraftMap` node rather than `load()`: Godot's `load()` follows a `.remap` or `.import` file planted beside the path to any resource, scripts included.
Model paths, material names and SDK names with `..`, `.` or empty segments, a backslash, a colon or a control character are refused before any path is built from them.
Mounting a player's `.pck` into `res://` lets it add scenes and resources a map can then name, scripts included, so do not mount packs from untrusted sources.

## Entity maps

A `MapCraftEntityMap` resource says what each entity class becomes.
Its `classes` maps a classname to a dictionary:

| Key | Meaning |
|---|---|
| `node` | `marker`, `node`, `area`, `static`, `world` (merge into world geometry), `scene`, `model` (instance the entity's model), `environment` (sky and sun, below) or `skip` |
| `name` | Node name instead of the targetname |
| `scene` | Scene to instance for `scene` |
| `key` | The key holding the model path for `model`, `model` by default |
| `meta` | Extra metadata set on the node |
| `properties` | `false` to leave key values off the node |
| `visible` | `true` to draw a brush entity's brushes |
| `collision_layer`, `collision_mask` | Physics layers for an `area` or `static` node, as bit masks |
| `strings` | Keys whose values stay text even when they look like numbers or vectors |
| `root` | `true` to put the node at the scene's root, even when the entity is inside a layer or group, for nodes a game looks up by path |

Classes not listed use `point_default` or `solid_default`, `marker` and `area` by default.
With `trimesh_collision` on, world brushes collide as one triangle mesh, so seams between ramp pieces cannot catch players.

When an import does not name an entity map, the addon uses `res://addons/mapcraft/sdks/<sdk>.tres` for the map's SDK.
`sdks/surfsup.tres` ships with the addon.

### Sky and sun

An `environment` entity becomes a `WorldEnvironment` with a procedural sky, and a `Sun` `DirectionalLight3D` shining along the entity's `angles`.
It reads the keys `sun_color`, `sun_energy`, `shadows`, `sky_top_color`, `sky_horizon_color`, `ground_color`, `ambient_energy`, `fog_density` and `fog_color`.
Colors are `R G B` from 0 to 255.
With `default_environment` on, a map with no such entity gets one with the default values.
MapCraft's Lit view previews this sky and sun from a `light_environment` with the same keys.

## SurfsUp

The SurfsUp entity map gives SurfsUp v2 and v3 what their custom maps expect:

- The player start becomes a root `SpawnPoint` marker, wherever it sits in the map's layers and groups.
- Zones become plain `Area3D` nodes tagged with `sdk_zone`: `StartZone`, `FinishZone`, kill boxes, splits, checkpoints, teleports, boosters, respawns and point teleports.
- Zone keys arrive as metadata named and typed for the zone's script, which SurfsUp copies onto the script's properties.
- Zones watch only collision layer 2, where SurfsUp keeps the player, so world brushes they touch never set them off.
- Teleport destinations are named by their targetname.
- `bhop_zone` becomes the same checkpoint area SurfsUp's own bhop zones use.
- `func_illusionary`, such as a Surf Ramp's trim, is drawn without collision.
- `starting_platform` and `finish_line` instance the game's `Levels/Utilities/starting_platform.tscn` and `landing_platform.tscn`.

The SurfsUp SDK's Run menu copies the map to SurfsUp's `Maps/<map>.mcmap` and starts the game with `+map <map>`.
SurfsUp loads the file at runtime, so nothing is imported or packed.
For games that load packs, `tools/pack_map.gd` packs an imported map into a `.pck`.

## Building

You need CMake 3.25, Ninja, Python 3 and a C++17 compiler.
Windows builds cross-compile with MinGW.
The macOS build makes one library for both Apple Silicon and Intel.
Off a Mac it cross-compiles with [osxcross](https://github.com/tpoechtrager/osxcross): set `OSXCROSS` to its `target` folder, or keep osxcross beside the MapCraft checkout, where `build.sh` looks by default.

```sh
./build.sh linux                     # or windows, macos; add template_release for release builds
GODOT_CPP=/path/to/godot-cpp ./build.sh linux   # use a local godot-cpp instead of downloading it
./test.sh                            # imports test/course.mcmap in headless Godot and checks the scene
```

Libraries land in `addons/mapcraft/bin/<platform>/`.

Godot 4.7.2 crashes on exit when `--import` finds a GDExtension it has not seen before.
`test.sh` avoids this by listing the extension in `.godot/extension_list.cfg` first; opening the project in the editor once does the same.

## Details

- A map point `(x, y, z)` becomes `(x, z, -y) / units_per_meter`.
- Point entities look down -Z along the entity's facing.
- A brush entity mapped to `static` becomes a `StaticBody3D`, and its brushes are drawn.
- A node is named by its class's `name`, else its `targetname`, else its classname; duplicates get a number (`FinishZone2`).
- Metadata types: `true` and `false` become `bool`, whole numbers `int`, other numbers `float`, three numbers a `Vector3` (turned into Godot axes when the key ends in `direction`), and anything else a `String`.
- An entity mapped to `model` instances the `.glb` or `.gltf` its model key names (`res://` is added when missing).
  It is turned by its `angles` and scaled by its `scale` or `modelscale`, without the -Z turn markers get, so the model sits as MapCraft draws it.
- Texture coordinates use the albedo texture's size, or 64 texels when the material has no texture, the same sizes MapCraft draws with.
- Meshes get a surface per material, blend layer and anti-tiling mode, named like `rock`, `rock#macro`, `rock#hex` or `rock|grass`.
- A face whose `attrs` set `anti_tile` to `macro` or `hex` draws through `shaders/mapcraft_single_<filter>.gdshader`, which multiplies albedo by low-frequency noise in map space, the same noise MapCraft's views draw.
  With `hex`, its `hex_tiling` also samples every map as randomly shifted and turned hex tiles; a Nearest filter of any kind on either material keeps `macro`.
- A blended displacement stores its `blend` weights in vertex color alpha (missing values read as 0) and draws through `shaders/mapcraft_blend_<filter>.gdshader`, whose `uv_ratio_b` keeps the second material's texel density.
- `<filter>` follows the face's own material: `nearest` for Nearest, `nearest_mipmap` for both Nearest Mipmap filters, and `linear` for the rest.
