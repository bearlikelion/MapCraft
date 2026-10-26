# MapCraft for Godot

A C++ GDExtension that loads MapCraft maps (`.mcmap`) in Godot 4.4 and newer, from GDScript or C#.

## What it adds

- **Import plugin**: every `.mcmap` in a project imports as a scene, so it can be instanced, opened or loaded like any `.tscn`. The scene uses only built-in nodes (`MeshInstance3D`, `StaticBody3D`, `Area3D`, `Marker3D`), so a game needs neither this extension nor any script to load it.
- **`MapCraftMap`** (`Node3D`): builds a `.mcmap` at runtime, for maps loaded from user folders or downloads. Set `map_file`, then call `build()` or leave `build_on_ready` on. It emits `entity_built(node, classname, properties)` for each entity so a game can finish it off.
- **`MapCraftEntityMap`** (`Resource`): says what each entity class becomes.

## How maps become nodes

- World brushes and brush entities mapped to `world` merge into one `WorldMesh` with a surface per material, and one `WorldCollision` body with a convex shape per brush.
- Faces with `tools/` materials (`tools/nodraw`, `tools/clip`, `tools/trigger`) collide but are not drawn.
- A point entity becomes a `Marker3D` (or the node kind its class maps to) at its origin, looking down -Z along the entity's facing.
- A brush entity becomes an `Area3D` (or `StaticBody3D`) centered on its brushes, with a convex shape per brush.
- Entity key values become metadata: whole numbers as `int`, other numbers as `float`, three numbers as `Vector3` (turned into Godot axes when the key ends in `direction`), anything else as `String`.
- A node is named by its class's `name`, else its `targetname`, else its classname; duplicates get a number (`FinishZone2`).
- Prefab instances load the prefab `.mcmap` as a scene instance.
- A point entity mapped to `model` instances the scene of the `.glb` or `.gltf` its model key names (`res://` is added when missing), turned by its `angles` and scaled by its `scale` or `modelscale`, without the -Z facing turn markers get, so the model sits as MapCraft draws it.
- Map space is Z up in inches; a point becomes `(x, z, -y) / units_per_meter`.

## Materials

A face's material name is looked up under the import option `material_root` (`res://Assets/Textures` by default): first `<name>.tres`, `.res` or `.material`, then an image (`png`, `jpg`, `webp`, `tga`, `bmp`) that becomes a `StandardMaterial3D`.
Texture coordinates use the albedo texture's size, or 64 texels when the material has no texture, the same sizes MapCraft draws with.

## Entity maps

`classes` maps a classname to a dictionary:

| Key | Meaning |
|---|---|
| `node` | `marker`, `node`, `area`, `static`, `world` (merge into world geometry), `scene`, `model` (instance the entity's model) or `skip` |
| `name` | Node name instead of the targetname |
| `scene` | Scene to instance for `scene` |
| `key` | The key holding the model path for `model`, `model` by default |
| `meta` | Extra metadata set on the node |
| `properties` | `false` to leave key values off the node |
| `visible` | `true` to draw a brush entity's brushes |

Classes not listed use `point_default` or `solid_default`.
When an import does not name an entity map, the extension uses `res://addons/mapcraft/sdks/<sdk>.tres` for the SDK the map was made with; `sdks/surfsup.tres` ships with the addon.

## SurfsUp

The SurfsUp entity map produces what SurfsUp v3 custom maps expect: a root `SpawnPoint` marker, zones as plain `Area3D` nodes tagged with `sdk_zone` (`StartZone`, `FinishZone`, kill boxes, splits, checkpoints, teleports, boosters, respawns and point teleports), their settings as typed metadata, and teleport destinations named by their targetname.

The SurfsUp SDK's Run menu imports the map into the Godot project, packs it with `addons/mapcraft/tools/pack_map.gd` into `Maps/<map>.pck` at `res://maps/<map>/<map>.scn`, and starts the game with `+map <map>`.
Copy `addons/mapcraft` into the project and open the project in the Godot editor once first.

## Building

Requires CMake 3.17, Ninja, Python 3 and a C++17 compiler; Windows builds cross-compile with MinGW.

```sh
./build.sh linux                     # or windows, macos; add template_release for release builds
GODOT_CPP=/path/to/godot-cpp ./build.sh linux   # use a local godot-cpp instead of downloading it
./test.sh                            # imports test/course.mcmap in headless Godot and checks the scene
```

Libraries land in `addons/mapcraft/bin/<platform>/`.

Godot 4.7.2 crashes on exit when `--import` finds a GDExtension it has not seen before, so `test.sh` lists the extension in `.godot/extension_list.cfg` first; opening a project in the editor once does the same.
