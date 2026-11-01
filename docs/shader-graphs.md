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
