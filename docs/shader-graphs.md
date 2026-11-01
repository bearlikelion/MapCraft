# Shader graph documents

Open **Materials > Shader Material > Shader Graph**, or use **File > Export > Shader Editor**.
The shader editors replace the central viewport, with **Viewport**, **Material**, **Shader Graph** and **Script** workspace tabs.
Switching tabs preserves unsaved shader source, graph and material edits.
Quitting checks all three documents and offers Save documents, Discard shader changes, or Cancel; canceled or failed saves keep the editor open.
The material browser and the material properties can each be collapsed.
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

The Script tab provides Godot shader syntax highlighting, line numbers and cursor position, with the text editor's native editing undo.
Source documents are limited to 256 KiB and save atomically.
Save and Save As target the active shader document; configured save shortcuts also work while typing.
Graph undo/redo shortcuts act on the graph, and viewport editing shortcuts are inactive while a shader workspace is open.
Highlighting is lexical, not compilation or semantic validation.
Shader compilation, viewport shader rendering, texture nodes and shader assignment to map materials are not implemented yet.

## PBR material authoring

The **Material** tab opens and saves version 1 PBR descriptors for `.mcpack` version 2.
Opening another descriptor is disabled while the current material has unsaved changes.
Save a descriptor under `materials/<name>.json` in a configured custom asset folder, with referenced images under its `textures/` folder.
Saving reloads project assets, updates the browser, and uploads the material to the viewport when the map uses it.
Select Lit mode for normal mapping, roughness, metallic and emission.
Plugin material listings do not replace a custom descriptor of the same name.
Invalid descriptors or missing dependencies produce a magenta material and an entry in the asset diagnostics.
