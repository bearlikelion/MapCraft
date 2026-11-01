# Shader graph documents

Open **Materials > Shader Material > Shader Graph**, or use **File > Export > Shader Editor**.
The shader editors replace the central viewport, with **Viewport**, **Material**, **Shader Graph** and **Script** workspace tabs.
Switching tabs preserves unsaved shader source, graph and material edits.
Quitting checks all three documents and offers Save documents, Discard shader changes, or Cancel; canceled or failed saves keep the editor open.
The graph canvas uses movable nodes and visible connections.
Click an output port and then an input port, or drag between them; Escape cancels a pending connection.
The compiler orders nodes by dependency, so connections can use any node order; cycles and incompatible output types are refused without changing the graph.
Right-click a node header to delete an unused node; connected nodes must have their consumers reconnected first.
Undo and Redo preserve up to 64 graph edits, including node layout changes.
The output card accepts color nodes for albedo/emission and scalar nodes for roughness/metallic.

**Save graph** writes an editable `.mcshader` JSON document atomically, including node and output positions.
Older graph documents without layout fields open with a default arrangement.
**Generate source** puts a deterministic Godot spatial shader in the Script tab; **Save As** exports it as `.gdshader`.
Generation is disabled while the Script tab has unsaved edits.
Opening another graph is disabled while graph edits are unsaved.
Graph documents are separate from map documents and are not currently included in `.mcpack` exports.

Version 1 supports scalar constants, RGB constants, time, multiplication and sine.
Multiplication accepts scalar/scalar, scalar/color and color/color.
Sine preserves its input type.
All constants must be finite, and graphs contain at most 128 nodes.
Unsupported document versions, missing inputs and incompatible output connections return diagnostics.

The Script tab provides Godot shader syntax highlighting, line numbers and cursor position, with Undo and Redo buttons that share the text editor's history (Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z), and Edit > Undo and Redo reach it too.
Source documents are limited to 256 KiB and save atomically.
Save and Save As target the active shader document; configured save shortcuts also work while typing.
Graph undo/redo shortcuts act on the graph, and viewport editing shortcuts are inactive while a shader workspace is open.
Highlighting is lexical; compile errors show under the Script toolbar with the line they come from.

## Viewport preview

While the shader workspace is open, the Script source previews live in the 3D view.
A `shader_type sky;` shader replaces the sky, with `EYEDIR`, `SKY_COORDS`, `TIME` and the first sun as `LIGHT0`, or the views' default light when the map has no sun.
Shaders with `use_half_res_pass` or `use_quarter_res_pass` run those passes first at full resolution and read them back through `HALF_RES_COLOR` and `QUARTER_RES_COLOR`.
A `shader_type spatial;` shader runs its `fragment()` on the faces that use the current material, the one shown in the toolbar.
Uniforms take their default values, and `source_color` defaults are converted to linear as Godot does.
Positions and directions are in Godot's Y-up axes and meters; `VERTEX`, `NORMAL` and `VIEW` are relative to the camera in world orientation rather than view space.
The preview draws `ALBEDO` with simple shading plus `EMISSION`, and ignores `vertex()`, `light()` and render modes.
Up to 8 `sampler2D` uniforms preview with Godot's unassigned defaults: white, or black, transparent or a flat normal by their hint.
`hint_screen_texture` samples the frame drawn before the previewed faces, so refraction and distortion shaders work; `hint_depth_texture` reads black.
Other sampler types, `#include`, `canvas_item` and `particles` shaders cannot be previewed yet, and shaders cannot be assigned to map materials yet.

## PBR material authoring

The **Material** tab opens and saves version 1 PBR descriptors for `.mcpack` version 2.
Opening another descriptor is disabled while the current material has unsaved changes.
Save a descriptor under `materials/<name>.json` in a configured custom asset folder, with referenced images under its `textures/` folder.
Saving reloads project assets, updates the browser, and uploads the material to the viewport when the map uses it.
Select Lit mode for normal mapping, roughness, metallic and emission.
Plugin material listings do not replace a custom descriptor of the same name.
Invalid descriptors or missing dependencies produce a magenta material and an entry in the asset diagnostics.
