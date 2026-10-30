# Blender and glTF

MapCraft reads and writes glTF (`.glb` and `.gltf`), the 3D file format Blender, Godot and most other 3D tools share.
It works with every SDK, so you can block out a level in Blender and finish it here, or send a map to another tool.

## Import

**File > Import > glTF** brings in a `.glb` or `.gltf` file as brushes in a new group named after the file.
You can also drag the file onto the MapCraft window.
One **Undo** takes the whole import back.

- Each mesh object that is convex, such as a box, a ramp or a wedge, becomes one brush.
- Any other object, such as an L shape, becomes one thin brush per face, 1 unit thick behind it, and the Output panel says how many objects came in this way.
- An object whose faces point inward, such as a room, is not convex, so its walls reach outward.
- Faces take the name of their glTF material, or of its texture when the material has none, and their textures line up with the model's UVs.
- With an SDK, textures in the file that it does not have are kept as materials, as other imports keep theirs.

## Export

**File > Export > glTF** writes the map as a `.glb` that Blender, Godot and other 3D tools open.

- It holds what the views show: hidden objects, layers left out of exports, faces with `tools/` materials and faces hidden between touching brushes stay out.
- Each brush is its own object with its textures and UVs, inside one object per layer, group and brush entity that holds it.
- Displacements come along as their sculpted surface.
- Point entities and their models are not exported.

glTF counts in meters with Y up, and MapCraft converts both ways with the SDK's scale, so a map exported and imported again lands where it was.
