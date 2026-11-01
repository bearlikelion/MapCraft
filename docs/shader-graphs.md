# Shader graph documents

Open **File > Export > Shader Editor**, then choose **Graph**.
The current editor uses node cards with numbered connections.
Inputs must reference earlier nodes, making cycles invalid by construction.
The output card accepts color nodes for albedo/emission and scalar nodes for roughness/metallic.

**Save graph** writes an editable `.mcshader` JSON document atomically.
**Generate source** puts a deterministic Godot spatial shader in the Source tab; **Save As** exports it as `.gdshader`.
Generation is disabled while the Source tab has unsaved edits.
Opening another graph is disabled while graph edits are unsaved.
Graph documents are separate from map documents and are not currently included in `.mcpack` exports.

Version 1 supports scalar constants, RGB constants, time, multiplication and sine.
Multiplication accepts scalar/scalar, scalar/color and color/color.
Sine preserves its input type.
All constants must be finite, and graphs contain at most 128 nodes.
Unsupported document versions, missing inputs and incompatible output connections return diagnostics.

Shader compilation, viewport rendering, texture nodes, graphical wire manipulation and map material assignment are not implemented yet.

## PBR material authoring

The **PBR Material** tab opens and saves version 1 PBR descriptors for `.mcpack` version 2.
Opening another descriptor is disabled while the current material has unsaved changes.
Save a descriptor under `materials/<name>.json` in a configured custom asset folder, with referenced images under its `textures/` folder.
Saving reloads project assets, updates the browser, and uploads the material to the viewport when the map uses it.
Select Lit mode for normal mapping, roughness, metallic and emission.
Plugin material listings do not replace a custom descriptor of the same name.
Invalid descriptors or missing dependencies produce a magenta material and an entry in the asset diagnostics.
