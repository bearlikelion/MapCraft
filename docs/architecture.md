# Architecture

## Layers

Dependencies only point down.
`cargo xtask layers` enforces this table; a new crate must be added to it and to `LAYERS` in `xtask/src/main.rs`.

| Layer | Crates | Notes |
|---|---|---|
| L0 | `geom` | Math and the brush kernel |
| L1 | `map`, `fgd` | Document model, entity definitions |
| L2 | `format`, `sdk` | `.mcmap` files, SDK manifests and assets |
| L3 | `plugin-host`, `render` | WebAssembly plugins, wgpu renderer |
| L4 | `engine` | Session, commands, undo, selection |
| L5 | `sdk-manager` | SDK registries, installs, packaging and checks |
| L6 | `ui` | egui shell |
| L7 | `mapcraft`, `cli` | Binaries |

Nothing below L6 may depend on egui, eframe, egui-wgpu, winit, rfd, muda or craft-ui.

## Brushes

A brush is a convex solid stored as its faces.
Each face stores three points that define its plane, plus its material and texture axes.
The plane points are the source of truth for files; the polygon of each face is derived by clipping a world-sized cube by every face plane (`geom::Polyhedron`).
Faces whose plane cuts nothing away are redundant and dropped.

## Document model

A map is a tree of nodes owned by `map::Map` and addressed by stable `NodeId`s:

- `World` (worldspawn) is the root.
- `Layer` nodes are exclusive parents directly under the world.
- `Group` nodes group anything; a group can carry a generator (plugin, command, params).
- `Entity` nodes hold key/values, I/O connections and, for brush entities, brush children.
- `Brush` nodes hold faces.
- `Prefab` nodes instance another `.mcmap` file with a transform and per-node overrides.

Tags are many-to-many labels on nodes (Hammer visgroups, Godot groups).
Faces carry stable ids because Source overlays reference them.
`extra` maps on the map, nodes and faces keep data a plugin needs to round-trip but the core does not model.

## Prefabs

A `Prefab` node names another `.mcmap` file, relative to the map that holds it, plus an origin, angles and key/value overrides.
`engine::flatten` makes a copy of the map with every instance expanded into a group of its contents, moved into place; the editor draws and picks that copy and maps hits back to the instance.
Nested prefabs expand up to 16 levels and a prefab that contains itself is refused.
Make Local runs the same expansion on the real map.

## Mirror groups

A group can carry a `Mirror` (an axis and the plane's position along it); its children are the edited half and the other half is derived from them (`map::mirror`).
`map::mirror::expand` adds the derived nodes to a copy of the map and records which original each one copies; `engine::flatten` runs it before expanding prefabs, so the editor draws the mirrored half and a click on it reaches the original or its group.
The `.mcmap` writer saves the derived half as children flagged `derived`, so importers need nothing new, and the reader drops them; validators check the expanded map.
Moving a mirror group moves its plane; Make Mirror Real inserts the derived half into the map as ordinary nodes in one undo step.

## Boolean groups

A group can carry the `boolean` flag; each brush directly inside it carries an `Operation`, Add or Subtract, which reshaping keeps (`map::boolean`).
Its result, each Add brush less every Subtract brush as convex pieces, is derived like a mirror half, so it is never stale and undo needs nothing extra.
`map::mirror::expand` gives the map as games get it: the pieces replace the group's own brushes, and the `.mcmap` writer saves them as `derived` children with the own brushes in `sources`, which the reader turns back into children.
`map::mirror::expand_live`, which `engine::flatten` runs for the views, keeps the own brushes and puts the pieces in a derived group inside, so the renderer draws the own brushes as outlines and a click on a piece reaches the Add brush it came from.
Exports, compiles and plugin checks take `engine::flatten_for_game` instead, so plugins never see a Subtract brush.

## Plugins

Game-specific code lives in WebAssembly plugins (`plugin-host`, API in [`plugin-api.md`](plugin-api.md)).
The host compiles each plugin with wasmtime, links WASI with no folders, environment or network, and offers read-only access to the SDK's asset folders.
Plugin output arrives as `format::Fragment` trees, the same type the clipboard uses, so the engine inserts it like a paste.
A generator group stores the plugin, command, parameters and placement in its `Generator`; the UI runs the plugin again when a parameter changes and the engine swaps the group's children in one undo step.
A resize or tilt the placement cannot hold goes in the generator's `stretch`; when a resize ends, the UI asks the plugin to fold it into the parameters and rebuilds the group as part of the same undo step.

## Viewports

Each viewport renders into its own offscreen texture (`render::Target`, 4x MSAA) that egui shows as an image.
The scene is rebuilt from the map whenever it changes; brush faces are batched by material.

## Undo

Every edit is a list of node changes (insert, remove, replace) applied by `engine::Session::apply` as one transaction.
The transaction records the previous value of each touched node, so undo swaps them back.

## Coordinates

Map space is right-handed with Z up, 1 unit = 1 inch, 39.37 units per meter, matching Source, Quake and SurfsUp.
The Godot extension maps `(x, y, z)` to `(x, z, -y) / 39.37`.

## glTF

`render::export_glb` and `render::import_gltf` move whole maps to and from glTF 2.0, beside the loader that reads the same files as point entity models.
glTF `(x, y, z)` in meters is map `(x, -z, y)` times the SDK's `units_per_meter`, both ways.
Export writes a node per visible brush, with a primitive per material, under nodes for the layers, groups and brush entities that hold it; tool materials and faces hidden between brushes stay out.
Import makes each mesh node one brush when every triangle lies on its convex hull facing out, closing hull sides no triangle covers; any other node becomes one brush per triangle or flat quad, 1 unit thick behind it.
Texture axes are fitted so each triangle's corners land on their UVs.
