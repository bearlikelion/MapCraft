# The Godot workflow

The MapCraft addon for Godot 4.4 and newer turns `.mcmap` files into Godot scenes.
The [addon's README](../../godot/README.md) has the full details.

## Install the addon

1. Put the addon in your project as `addons/mapcraft`: unzip `MapCraft-godot-addon.zip` from a MapCraft release (see [Installing](installing.md)) into the project's root folder, or copy `godot/addons/mapcraft` from the MapCraft source tree after building it with `./build.sh`.
2. Open the project in the Godot editor once, so Godot registers the extension.

The second step matters for the Run menu: it imports with a headless Godot, and Godot 4.7.2 crashes on exit when a headless import meets an extension it has not seen before.

## Maps as scenes

Every `.mcmap` file in the project imports as a scene, so you can instance it, open it or load it like a `.tscn`.
The scene uses only built-in nodes, so the game needs neither the extension nor any script to load it.

- World brushes merge into one `WorldMesh` with a surface per material, and one `WorldCollision` body.
- Faces with `tools/` materials (`tools/nodraw`, `tools/clip`, `tools/trigger`) collide but are not drawn.
- Point entities become `Marker3D` nodes, and brush entities `Area3D` nodes, unless the entity map says otherwise.
- Key values become metadata on the nodes.
- Prefab instances load the prefab's `.mcmap` as a scene instance; see [Prefabs](prefabs.md#prefabs-in-godot).
- Props (classes the entity map sets to `model`, such as SurfsUp's `prop_static`) instance the scene Godot imported from the `.glb` or `.gltf` their `model` key names, as `res://<path>`, turned by their `angles` and scaled by `scale` or `modelscale`; see [Models](entities.md#models).
- Everything outside an active [cordon](organizing.md#cordons) is left out.

A face's material is looked up under the material root by its name: first `<name>.tres`, `.res` or `.material`, then an image (`png`, `jpg`, `webp`, `tga`, `bmp`).

## Import options

Select the `.mcmap` in the FileSystem dock and change these in the Import dock, then click **Reimport**:

| Option | Default | Effect |
|---|---|---|
| **Units Per Meter** | 39.37 | Map units per Godot meter |
| **Material Root** | `res://Assets/Textures` | Where material names are looked up |
| **Entity Map** | empty | A `MapCraftEntityMap` saying what each entity class becomes; empty uses the addon's `sdks/<sdk>.tres` for the map's SDK when there is one |
| **Collision** | on | Builds collision |
| **Trimesh Collision** | off | The whole world collides as one triangle mesh instead of one convex shape per brush |

Turn on Trimesh Collision for surf maps: seams between the pieces of a curved ramp can catch players when each brush collides on its own.

Faces hidden between touching world brushes, such as the joins between ramp pieces, are left out of the meshes and of trimesh collision, so they cost nothing to draw and leave no seams; per-brush collision still uses whole brushes.

The addon ships `sdks/surfsup.tres`, so SurfsUp maps get the SurfsUp entity map without setting anything: the player start becomes a root `SpawnPoint` marker, zones become `Area3D` nodes tagged with `sdk_zone`, `func_detail` merges into the world, and `prop_static` instances its model.
Props get no collision of their own; give the model collision in its Godot import settings when players should hit it.

## Building at runtime

A `MapCraftMap` node (a `Node3D`) builds a map while the game runs, for maps loaded from user folders or downloads.

- Set **Map File** and call `build()`, or leave **Build On Ready** on.
  Building first removes the node's children, and `clear()` does only that.
- It has the same **Entity Map**, **Units Per Meter**, **Material Root**, **Collision** and **Trimesh Collision** settings as the importer.
- It emits `entity_built(node, classname, properties)` for each entity, so the game can finish it off.

## The SurfsUp Run menu

The SurfsUp SDK's compile profiles drive Godot for you.
Each one first copies the map to `<project>/maps/<map>/<map>.mcmap`, where `<project>` is the SurfsUp Godot project folder and `<map>` the map's file name.

| Profile | What it does | SDK Settings it needs |
|---|---|---|
| **Import into Godot** | Runs a headless Godot import of the project, which turns the map into a scene | Project folder, Godot editor |
| **Build map pack** | Also packs `res://maps/<map>/<map>.scn` into `<map>.pck` in the map pack folder, with the addon's `tools/pack_map.gd` | Project folder, Godot editor, map pack folder |
| **Build and play** | Also starts the game with `+map <map>` | All of the above, and the SurfsUp game executable |

- The map pack holds only the map's scene; materials, textures and prop models stay paths into the game, so the game must have them at the same paths.
- Only the map file is copied, so make prefabs local before building (see [Prefabs](prefabs.md#prefabs-in-godot)).
- Turn off cordons first, or the scene only holds what touches them.

See [SDKs and plugins](sdks-and-plugins.md#compile-profiles) for running profiles and the trust prompt.
