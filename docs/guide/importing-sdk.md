# Import a SurfsUp SDK map

Import a map built with the SurfsUp SDK in Godot and keep editing it in MapCraft.

## Import

1. Open or make a map with the SurfsUp SDK.
2. Choose **File > Import > SurfsUp SDK map**.
3. Pick the map's `.pck` pack, or its level scene (`.tscn` or `.scn`) from the SDK project.

![The File menu with Import open, showing the SurfsUp SDK map importer](img/sdks-import-menu.webp)

- Packs exported from Godot 4 come in whether their scenes are text or binary (Godot's default), compressed or not.
- Everything goes into a new layer named after the file, as one undo step, and the views frame it.
- The Output panel (`Ctrl+5`) sums up what came in, and lists everything left out by kind.

![An imported SurfsUp SDK map: its zones, spawn, sky and ramp groups in the scene tree, with a curved ramp selected](img/sdk-imported.webp#wide)

## What comes in

- **CSG shapes:** boxes, cylinders and polygons (extruded, spun or swept along a path) become convex brushes, one group per shape, named after its Godot node.
- **Solid or not:** a shape with collision on is solid, and so is one inside a baked collision mesh, the way SDK ramps are made. Everything else becomes `func_illusionary`, which you can see but not touch.
- **Simple meshes:** boxes, planes, quads, cylinders and prisms become brushes too. Planes become slabs one unit thick.
- **Baked ramps:** a static body's concave collision mesh, such as the `ArrayMesh` ramps a track generator bakes, splits into convex brushes, one group per ramp.
  Surf faces stay exact, curved ramps become a brush per segment, and one sided surfaces get 16 units of thickness behind them.
  Each face takes the material of the mesh surface drawn over it; collision with no mesh drawn over it becomes player clip.
- **Zones:** zones placed with the SDK's **Add Zone** become the matching SurfsUp entity, with all their settings. Older maps are read by node name (`StartZone`, `FinishZone`, `KillBox` and the others) and by the game's utility scenes.
- **Teleport targets:** a node that a teleport, respawn or point teleport names becomes a `teleport_destination` with that name.
- **Spawn:** `SpawnPoint` becomes `info_player_start`, facing the same way.
- **Sky and sun:** the scene's sky and its first sun become a `light_environment`, so the [Lit view](views.md#lit) shows the map's sky.
- **Materials:** a texture from SurfsUp's `Assets/Textures` keeps its SDK name. Other materials become a color swatch named after the map, such as `surf_synthwave/g_pink`, saved in MapCraft's data folder like [BSP textures](importing-bsp.md#what-comes-in).
- **Size:** Godot meters become map units (39.37 per meter), and Godot's Y up becomes Z up.

## Limits

- Encrypted packs are refused with a message.
- CSG subtractions and intersections, spheres, tori and CSG meshes are left out.
- Imported models, `Label3D` text, particles and lights other than the sun are left out.
- Meshes that are not Godot primitives and are not drawn over a collision mesh, such as decoration, are left out.
- Collision meshes shaped to the ground inside zones are left out; add a `kill_box` brush there instead.
- A kill zone with a world boundary becomes a slab 512 units thick under the boundary, reaching 4096 units past the map.
- Shapes reaching past MapCraft's world, 131072 units from the origin, are cut to fit.

> [!NOTE]
> Swatch materials exist only in MapCraft.
> Swap them for SurfsUp textures before you [play the map in SurfsUp](godot.md), or the game will not find them.
