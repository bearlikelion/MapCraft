# Materials

Keys on this page are the MapCraft preset's defaults.
Every shortcut can be rebound, and the Hammer and TrenchBroom presets change some of them: see [Keyboard shortcuts](shortcuts.md).

## The current material

The current material is the one new brushes get and the one the apply commands use.
The **Material** chip on the top bar shows its picture and name; click it to open the Materials panel.
When the map's SDK names a default material and texture scale, they become current as the SDK loads (see [SDK manifests](../sdk-format.md)).

## The Materials panel

Open the **Materials** tab of the left panel (`Ctrl+3`).
It lists the materials of the map's SDK; without an SDK it asks you to pick one with **File > New Map**.

- Type in **Search materials** to filter by name.
- **Previews** switches between pictures and a plain list of names.
- Click a material to make it current; it is highlighted.
  With brushes selected or faces picked, the click also applies it to them, as in TrenchBroom.
- **Apply to selection** gives the current material to the picked faces, else to every face of the selected brushes.

To only make a clicked material current, turn off **Clicking a material applies it to the selection** in **Edit > Preferences**; a double click then applies it.

## Applying a material

| Do this | Applies the current material to |
|---|---|
| `Shift+T` (**Tools > Apply Current Material**) | The picked faces in the Face tool, else every face of the selected brushes |
| **Apply current material** in the inspector | The same |
| Clicking a material in the Materials panel | The same, unless turned off in Preferences |
| Alt+right click a face in the 3D view | That one face |

Alt+right click works with any tool, so you can paint faces without leaving the one you are using.

## Picking a material

Alt+click a face in the 3D view to make its material current.
The status bar shows **Picked** and the name, and MapCraft also remembers that face's alignment for wrapping.

## Copying and pasting faces

In the Face tool, `Ctrl+C` copies the active face (the one last clicked) and `Ctrl+V` pastes it on every picked face as one undo step.
A paste gives the material, the alignment and face attributes such as lightmap scale, and turns the texture across shared edges as wrapping does, so it runs on around corners.
**Paste face** in the face panel does the same, and Alt+click also copies the face it picks.

## Wrapping alignment

Alt+Shift+right click a face in the 3D view to give it the material and alignment of the face you last Alt+clicked.
The texture is turned across the edge where the two faces meet, so it runs on around a corner without a seam.
Until you have Alt+clicked a face, Alt+Shift+right click just applies the current material.

To texture a run of faces:

1. Alt+click a face that already looks right.
2. Alt+Shift+right click the next face along, then the next.

## The face panel

Press `Shift+F` for the Face tool and pick faces (see [Tools](tools.md#face)); the inspector then shows the face panel.
It shows the values of the first picked face, and every change applies to all picked faces.

| Control | Does |
|---|---|
| **Apply current material** | Gives the picked faces the current material |
| **Paste alignment** | Gives the picked faces the material and alignment of the face you last Alt+clicked, wrapped across shared edges |
| **Offset** U and V | Slides the texture, in texels |
| **Scale** U and V | Map units per texel; larger values stretch the texture |
| **Rotation** | Turns the texture on each picked face by the amount you change it |
| **Justify Fit** | Stretches the texture so one copy covers the face exactly |
| **Justify L**, **R**, **T**, **B** | Lines the texture's left, right, top or bottom edge up with the face's |
| **Justify C** | Centers the texture on the face |
| **Align to world** | Projects the texture straight from the nearest world axis and clears the rotation |
| **Align to face** | Lays the texture on the face with V pointing downhill; floors and ceilings get world alignment |
| **Lightmap scale** | Sets the face's lightmap scale; clear it to remove the setting |

Offset and scale set every picked face to the numbers shown, and a scale of 0 is ignored.
The **Displacement** section below these controls is covered in [Displacements](displacements.md).

## The UV editor

The UV editor shows the picked faces laid out over their texture, as Blender's UV Editor does.
Click a view's name in its header and pick **UV Editor** to turn that view into one; pick 3D, Top, Front or Side to turn it back.

- The texture of the active face, the face you picked last, is tiled in every direction, with the first tile outlined in orange and a faint texel grid on top.
- Each picked face is drawn on it as its outline, filled with the color of its island: faces that share an edge and a material form an island.
- The active face has a white outline; the other faces of the same brushes are thin gray outlines, so you can pick them too.
- Faces with another material are scaled so their texture lines up with the one shown.
- The header shows the material, its size in texels and the zoom.

Picks are shared with the Face tool: faces picked in the 3D view show here, and faces picked here are outlined in the 3D view.

| Do this | To |
|---|---|
| Scroll | Zoom |
| Middle or right drag | Pan |
| Double click | Fit the picked faces in the view |
| Click a face | Pick just that face; Ctrl+click or Shift+click adds or removes it |
| Drag a box | Pick the faces whose middle lies inside it; with Ctrl or Shift it adds them |
| Right click | Open the unwrap commands |

### Grab, rotate and scale

With the pointer over a UV editor, press `G` to grab, `R` to rotate or `S` to scale the picked faces' textures.
The textures follow the pointer, in the 3D view too, until you apply or cancel:

| Key | Does |
|---|---|
| `X` or `Y` | Locks a grab or scale to U or to V; press it again to unlock |
| Hold `Ctrl` | Snaps a grab to whole texels, a rotation to 15 degree steps and a scale to tenths |
| Click or `Enter` | Applies the change |
| `Escape` or right click | Puts everything back |

Rotate and scale turn and stretch around the middle of the picked faces.
Every picked face moves by the same amount, so an island stays whole and its texture stays continuous across the edges inside it.
The whole change is one undo step, and the view's top left corner shows how far it has gone.
While a change runs, the editor's other keys and clicks wait for it.

These keys only work over a UV editor, where they win over the same keys elsewhere: `R` still picks the Rotate tool in the other views.

### Unwrapping

**Tools > UV** and the UV editor's right click menu hold four commands for the picked faces:

| Command | Does |
|---|---|
| **Unwrap** (`U` over a UV editor) | Lays each island out from one face so the texture runs on across its shared edges without a seam; that face, the active one when the island holds it, keeps its mapping |
| **Project from View** | Projects one texture onto every picked face along the 3D view's line of sight, upright as the view sees it, at the active face's scale |
| **Cube Projection** | Projects each face's texture from the nearest world axis, like **Align to world** |
| **Reset** | Gives each face the mapping a new face gets: world aligned, no offset or rotation, at the SDK's texture scale |

Unwrap does what [wrapping alignment](#wrapping-alignment) does by hand, for every face of an island at once, much like Blender's Follow Active Quads.
A closed shape such as a box still has a seam where the faces come around to meet again.
Displaced faces take part like any other face, and their sculpted surface follows the new mapping.

## Texturing whole brushes

With brushes selected (and the Face tool off), the inspector offers:

- **Apply current material** for every face.
- **Align to world** and **Align to face** for every face.
- **Texture scale** with **Set**, which gives every face that U and V scale.
- Four arrow buttons that slide every face's texture 16 texels left, right, up or down.

## Texture lock

Texture lock is on by default.
Toggle it with `Shift+L`, the **Texture lock** button on the top bar, or **Tools > Texture Lock**; the status bar shows whether it is on.

- On: textures stay fixed to brushes as you move, rotate, scale, flip, duplicate or paste them.
- Off: textures stay put in the world and the brushes slide under them.

Mirror groups have a texture lock setting of their own; see [Mirror groups](mirror-groups.md).

## Replace Materials

**Tools > Replace Materials** swaps one material for another on many faces at once.
It has no key by default; you can bind one in **Edit > Preferences**.

1. Type the material to find in **Find** and the new one in **Replace with**.
   The window opens with **Find** set to a material of the selected brushes and **Replace with** set to the current material.
2. Choose **Selection** or **Whole map**; it starts on **Selection** when something is selected.
3. Click **Replace**.

Normally only faces whose whole material name matches **Find**, ignoring case, change.
Tick **Replace text inside names** to replace that text wherever it appears in a name, matching case, for example "dev/" with "concrete/" to move a set of materials to another folder.
The status bar reports how many faces changed, and one `Ctrl+Z` undoes the lot.
