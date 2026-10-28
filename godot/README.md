# MapCraft for Godot

A C++ GDExtension that loads MapCraft maps (`.mcmap`) in Godot 4.4 and newer, from GDScript or C#.

## What it adds

- **Import plugin**: every `.mcmap` in a project imports as a scene, so it can be instanced, opened or loaded like any `.tscn`. The scene uses only built-in nodes (`MeshInstance3D`, `StaticBody3D`, `Area3D`, `Marker3D`), so a game needs neither this extension nor any script to load it.
- **`MapCraftMap`** (`Node3D`): builds a `.mcmap` at runtime, for maps loaded from user folders or downloads. Set `map_file`, then call `build()` or leave `build_on_ready` on. It emits `entity_built(node, classname, properties)` for each entity so a game can finish it off.
- **`MapCraftEntityMap`** (`Resource`): says what each entity class becomes.
- **Runtime loading**: in a running game, `load()` turns any `.mcmap` path, inside or outside the project, into a scene the same way the importer does, so a game can load maps from a folder with no import step.

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

A face whose `attrs` set `anti_tile` to `macro` or `hex` gets its material wrapped in `shaders/mapcraft_single_<filter>.gdshader`, which multiplies albedo by low-frequency noise in map space, the same noise MapCraft's views draw.
With `hex` the wrapper's `hex_tiling` also samples every map as randomly shifted and turned hex tiles over the UVs, as the views do, unless either material uses a Nearest filter, which keeps `macro`.
The wrapper is a `ShaderMaterial` that copies the material's albedo, UV scale and offset, roughness, metallic, specular, normal and ambient occlusion maps and next pass; `<filter>` is `nearest` for Nearest, `nearest_mipmap` for both Nearest Mipmap filters and `linear` for the rest.
Only a `StandardMaterial3D` with Godot's default modes (per-pixel shading, back-face culling, Mix blending, Burley diffuse, Schlick GGX specular, texture repeat), no transparency, fades, billboard or grow, no features beyond normal and ambient occlusion maps, and none of the flags that change shading (triplanar, vertex color, forced sRGB, no shadows and the like) can be wrapped; any other material is used as it is, with a warning naming the reason.
Inside a prefab, the game places the noise by the prefab's own origin, so it repeats with each copy and differs from MapCraft's views.
A displacement whose `disp` names a `blend_material` (other than empty, its own material or a `tools/` one) stores its `blend` weights in vertex color alpha and draws through `shaders/mapcraft_blend_<filter>.gdshader`, which copies both materials, keeps each one's texel density (`uv_ratio_b`) and mixes them by weight shaped by each texture's brightness; `<filter>` follows the face's own material, both materials must pass the rule above, and missing `blend` values read as 0.
Meshes get a surface per material, blend layer and anti-tiling mode, named like `rock#macro`, `rock#hex` or `rock|grass`.

## Entity maps

`classes` maps a classname to a dictionary:

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

Classes not listed use `point_default` or `solid_default`.
An `environment` entity becomes a `WorldEnvironment` with a procedural sky and a `Sun` `DirectionalLight3D` shining along the entity's `angles`, from the keys `sun_color`, `sun_energy`, `shadows`, `sky_top_color`, `sky_horizon_color`, `ground_color`, `ambient_energy`, `fog_density` and `fog_color` (colors as `R G B` from 0 to 255).
With `default_environment` on, a map that has no such entity gets one with those keys' defaults.
MapCraft's Lit view previews this sky and sun from a `light_environment` with these keys.
With `trimesh_collision` on, world brushes collide as one triangle mesh, as if every import turned on Trimesh Collision; seams between ramp pieces then cannot catch players.
When an import does not name an entity map, the extension uses `res://addons/mapcraft/sdks/<sdk>.tres` for the SDK the map was made with; `sdks/surfsup.tres` ships with the addon.

## SurfsUp

The SurfsUp entity map produces what SurfsUp v2 and v3 custom maps expect: a root `SpawnPoint` marker, zones as plain `Area3D` nodes tagged with `sdk_zone` (`StartZone`, `FinishZone`, kill boxes, splits, checkpoints, teleports, boosters, respawns and point teleports), their settings as typed metadata, and teleport destinations named by their targetname.
Zones watch only collision layer 2, where SurfsUp keeps the player, so world brushes they touch never set them off.
`bhop_zone` becomes the same checkpoint area SurfsUp's own bhop zones use, and `starting_platform` and `finish_line` instance the game's `Levels/Utilities/starting_platform.tscn` and `landing_platform.tscn`.
Every zone key arrives as metadata named and typed for the zone's script, which SurfsUp copies onto the script's properties.

The SurfsUp SDK's Run menu copies the map to SurfsUp's `Maps/<map>.mcmap` and starts it with `+map <map>`; SurfsUp loads the file through the runtime loader, so nothing is imported or packed.
`tools/pack_map.gd` still packs an imported map into a `.pck` for games that load packs.
Copy `addons/mapcraft` into the project and open the project in the Godot editor once first.

## Building

Requires CMake 3.17, Ninja, Python 3 and a C++17 compiler; Windows builds cross-compile with MinGW, and the macOS build (a universal library for Apple Silicon and Intel) has to run on a Mac.

```sh
./build.sh linux                     # or windows, macos; add template_release for release builds
GODOT_CPP=/path/to/godot-cpp ./build.sh linux   # use a local godot-cpp instead of downloading it
./test.sh                            # imports test/course.mcmap in headless Godot and checks the scene
```

Libraries land in `addons/mapcraft/bin/<platform>/`.

Godot 4.7.2 crashes on exit when `--import` finds a GDExtension it has not seen before, so `test.sh` lists the extension in `.godot/extension_list.cfg` first; opening a project in the editor once does the same.
