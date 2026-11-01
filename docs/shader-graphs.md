# Shaders and shader graphs

Open **Materials > Shader Material > Shader Graph**, or use **File > Export > Shader Editor**.
The shader editors replace the central viewport, with **Viewport**, **Material**, **Shader Graph** and **Script** workspace tabs.
Switching tabs preserves unsaved graph and material edits.
Quitting checks the graph and material documents and offers Save documents, Discard shader changes, or Cancel; canceled or failed saves keep the editor open.

## Embedded shaders

Godot shaders live in the map itself, one per node: the World node can carry a sky shader and each brush can carry a spatial shader, so a pond and a sky shader draw at the same time.
They are saved inside the `.mcmap` (see the `shader` field in [the format reference](mcmap-format.md)), so the map's own save and unsaved state cover them.
**Tools > Shader > Add Shader**, or **Add shader** in the Brushes section of the inspector, gives the selected brushes a starter spatial shader.
With no brush selected, or in the World inspector's **Shader** section (**Add sky shader**), it gives the World a starter sky shader.
**Remove Shader** and **Remove shader** take them off again; every add, remove and edit is an undo step.

The left panel's **Shaders** tab (Ctrl+7, also opened by **Tools > Shader Script**) lists every node with a shader, such as `World (sky)` or `Brush #12`, under a **Filter shaders** field.
The shader being edited is highlighted, and a shader that does not compile shows in red with its error on hover.
Clicking an entry selects that node and opens its source in the Script tab.
Typing in the Script tab writes through to the map; a run of typing undoes as one step, ended when the editor loses focus.
**Import .gdshader** replaces the picked shader with a file's source and **Export .gdshader** writes it to a file.
Save, Undo and Redo in the Script tab are the map's own.

### Shader parameters

Selecting the World or a single brush with a shader shows a **Shader Parameters** section in the inspector, beside whichever workspace tab is open.
It lists the shader's uniforms in source order under foldable `group_uniforms` and `subgroup_uniforms` headers, as Godot's inspector does.
`hint_range` uniforms are sliders that keep the hint's step, `int` ones whole; other numbers are drag fields, `bool` a checkbox, vectors one drag field per component, and `source_color` a color picker, with alpha for `vec4`.
A `sampler2D` is a material slot: its button opens the Materials panel, and the bucket button puts the current material in the slot; an empty slot shows the Godot default its hint gives.
`hint_screen_texture` is filled by the renderer and shows read only.
A value that differs from the shader's default shows a revert button that returns it to the default.
Each change is an undo step, and dragging one value undoes as one step.
Values are stored as the node's `shader_params` (see [the format reference](mcmap-format.md)), keep through source edits for uniforms that still exist, and go away with the shader.
Materials named by texture parameters load and pack into `.mcpck` like face materials.
The graph canvas uses movable nodes and visible connections.
Click an output port and then an input port, or drag between them; Escape cancels a pending connection.
The compiler orders nodes by dependency, so connections can use any node order; cycles and incompatible output types are refused without changing the graph.
Right-click a node header to delete an unused node; connected nodes must have their consumers reconnected first.
Undo and Redo preserve up to 64 graph edits, including node layout changes.
The output card accepts color nodes for albedo/emission and scalar nodes for roughness/metallic.

**Save graph** writes an editable `.mcshader` JSON document atomically, including node and output positions.
Older graph documents without layout fields open with a default arrangement.
**Generate source** replaces the shader picked in the Shaders list with a deterministic Godot spatial shader and opens it in the Script tab; with none picked it waits for the next one.
Opening another graph is disabled while graph edits are unsaved.
Graph documents are separate from map documents and are not currently included in `.mcpck` exports.

Version 1 supports scalar constants, RGB constants, time, multiplication and sine.
Multiplication accepts scalar/scalar, scalar/color and color/color.
Sine preserves its input type.
All constants must be finite, and graphs contain at most 128 nodes.
Unsupported document versions, missing inputs and incompatible output connections return diagnostics.

The Script tab provides Godot shader syntax highlighting, line numbers and cursor position.
Its Undo and Redo buttons are the map's; while typing, Ctrl+Z steps the text editor's own history, and each change still reaches the map.
Shader source is limited to 256 KiB.
Save and Save As save the map from the Script tab and the active document from the Graph and Material tabs; configured save shortcuts also work while typing.
Graph undo/redo shortcuts act on the graph, and viewport editing shortcuts are inactive while a shader workspace is open.
Highlighting is lexical; compile errors show under the Script toolbar with the line they come from.

## Drawing in the 3D views

Embedded shaders draw live in the 3D views whether or not a shader workspace is open, and animate their `TIME` while any of them draws.
The World's `shader_type sky;` shader replaces the sky, with `EYEDIR`, `SKY_COORDS`, `TIME` and the first sun as `LIGHT0`, or the views' default light when the map has no sun.
Shaders with `use_half_res_pass` or `use_quarter_res_pass` run those passes first at full resolution and read them back through `HALF_RES_COLOR` and `QUARTER_RES_COLOR`.
Shader parameters replace the uniforms' defaults; textures sample the named material's albedo, and a missing material samples the magenta checker.
A brush's `shader_type spatial;` shader runs its `fragment()` on all of that brush's faces, tool materials included.
A sky shader on a brush or a spatial shader on the World is an error, and a brush whose shader does not compile draws its materials.
Each distinct source and parameter set compiles once and is reused until it changes, so dragging a value recompiles it each step; up to 64 distinct ones draw at once and the rest report an error.
Uniforms take their parameter or default values, and `source_color` values are converted to linear as Godot does.
Positions and directions are in Godot's Y-up axes and meters; `VERTEX`, `NORMAL` and `VIEW` are relative to the camera in world orientation rather than view space.
The preview draws `ALBEDO` with simple shading plus `EMISSION`, and ignores `vertex()`, `light()` and render modes.
Up to 8 `sampler2D` uniforms draw: with a material parameter, its albedo; without, Godot's unassigned default for their hint (white, black, transparent or a flat normal).
`hint_screen_texture` samples the frame drawn before any shaded brush, so refraction and distortion shaders work, though shaded brushes do not see each other through it; `hint_depth_texture` reads black.
Other sampler types, `#include`, `canvas_item` and `particles` shaders cannot be drawn yet.
Duplicating a brush keeps its shader; copy and paste does not carry shaders yet.

## PBR material authoring

The **Material** tab opens and saves version 1 PBR descriptors for `.mcpck` version 2.
Opening another descriptor is disabled while the current material has unsaved changes.
Save a descriptor under `materials/<name>.json` in a configured custom asset folder, with referenced images under its `textures/` folder.
Saving reloads project assets, updates the browser, and uploads the material to the viewport when the map uses it.
Select Lit mode for normal mapping, roughness, metallic and emission.
Plugin material listings do not replace a custom descriptor of the same name.
Invalid descriptors or missing dependencies produce a magenta material and an entry in the asset diagnostics.
