# Materials

Pick a material, put it on faces and align it.
Keys here are from the MapCraft preset; see [Keyboard shortcuts](shortcuts.md) for the Hammer and TrenchBroom presets.

## The current material

New brushes and the apply commands use the current material.
The material chip on the top bar, a swatch and the material's name, shows it; click the chip to open the Materials panel.

When the SDK names a default material and texture scale, they become current as the SDK loads.

![The top bar with the Material chip highlighted, showing the current material ramps-orange1](img/materials-current.webp)

## The Materials panel

Open the **Materials** tab on the left (`Ctrl+3`).
It lists every material of the map's SDK, including the [texture folders](sdks-and-plugins.md#texture-folders) you added in SDK Settings.

1. Type in **Search materials** to filter by name.
2. The **Previews** button at the right of the material count switches between pictures and a plain list of names.
   **Folders** lists the SDK's top-level material folders, or the folders its plugin puts materials in, such as one per WAD on the Quake SDK; untick one to hide its materials, or use **Hide all** and tick the ones you want.
   Searching matches folder names as well as material names.
   The choice is saved per SDK, so it holds for every map on that SDK, and faces already using a hidden material still draw.
3. Click a material to make it current.
   With brushes selected or faces picked, the click also puts it on them, as in TrenchBroom.
4. **Apply to selection** gives the current material to the picked faces, or else to every face of the selected brushes.
5. Right click a material for **Select Faces Using It**, which picks every face using it in the Face tool, or **Select Brushes Using It**, which selects every brush with a face using it.
   Hidden and locked brushes are left out.
   **Edit > Select by Material** does the same for the current material.

Without an SDK the panel asks you to pick one with **File > New Map**.

![The Materials panel with numbered markers on the search box, the Previews toggle, a material picture and Apply to selection](img/materials-panel.webp)

> [!TIP]
> To make a click only set the current material, turn off **Clicking a material applies it to the selection** in **Edit > Preferences**.
> A double click then applies it.

## Applying a material

| Do this | Puts the current material on |
|---|---|
| `Shift+T` (**Tools > Apply Current Material**) | The picked faces, or else every face of the selected brushes |
| The bucket (**Apply current material**) in the inspector | The same |
| Click a material in the Materials panel | The same, unless you turned that off |
| Alt+right click a face in the 3D view | Only that face |

Alt+right click works in any tool, so you can paint faces without switching tools.

**Tools > Clear Material** takes the material off the picked faces, or else off every face of the selected brushes, and leaves them untextured.
It is also in the right click menu and beside **Apply to selection** in the Materials panel.
Untextured faces export as `tools/toolsnodraw` in a VMF.

## Picking a material

Alt+click a face in the 3D view to make its material current.
The status bar shows **Picked** and the material's name.
MapCraft also remembers that face's alignment, for [wrapping](#wrapping-alignment).

![Alt+clicking the side of a ramp makes MetalWalkway011 the current material on the top bar](img/materials-pick.webp)

## Copying and pasting faces

1. Press `Shift+F` for the Face tool and click the face that looks right, or Shift+click it in the 3D view from the Select or Move tool.
2. Press `Ctrl+C` to copy it.
3. Pick the faces to change and press `Ctrl+V`.

A paste copies the material, the alignment and settings such as lightmap scale.
It turns the texture across shared edges, so it runs on around corners.
**Paste face** in the face panel does the same, and Alt+click also copies the face it picks.

## Wrapping alignment

Wrapping carries a texture around a corner without a seam.

1. Alt+click a face that already looks right.
2. Alt+Shift+right click the next face along.
3. Keep going, face after face.

Each face gets the material, alignment and settings such as lightmap scale of the face you Alt+clicked, turned across the edge where they meet, just as a paste does.
Until you have Alt+clicked a face, Alt+Shift+right click only applies the current material.

## The face panel

Press `Shift+F` for the Face tool and pick faces (see [Tools](tools.md#face)).
The inspector shows the face panel, with the **UV**, **Surface** and **Alignment** sections.
**UV** at the top is the [UV editor](#the-uv-editor).
Each field shows the picked faces' value, or **mixed** when they differ, and every change applies to all picked faces.
Dragging a field moves each face's own value by the same amount, so faces that differed stay different; typing a number gives them all that number.

| Control | Does |
|---|---|
| **Material** | Shows the faces' material; click it to open the Materials panel, where clicking a material applies it to the picked faces |
| The bucket (**Apply current material**) | Gives the picked faces the current material |
| **Paste face** | Gives them the material and alignment of the face you last copied (`Ctrl+C` or Alt+click), wrapped across shared edges |
| **Offset** U and V | Slides the texture, in texels |
| **Texture scale** U and V | Map units per texel; bigger stretches the texture (values near 0 are ignored) |
| **Rotation** | Turns the texture by the amount you change it |
| **Justify** expand icon (**Fit**) | Stretches the texture so one copy covers the face |
| **Justify** arrow icons | Line up the texture's left, right, top or bottom edge with the face's |
| **Justify** target icon (**Center**) | Centers the texture on the face |
| **Align** globe icon (**Align to world**) | Projects the texture straight from the nearest world axis and clears the rotation |
| **Align** face icon (**Align to face**) | Lays the texture on the face with V pointing downhill; floors and ceilings get world alignment |
| **Lightmap scale** | Sets the faces' lightmap scale; it reads **default** until set, and the curved arrow removes the setting |
| **Anti-tiling** | Hides the grid a texture repeated across a big ramp or floor shows, in the views and in Godot; see [Anti-tiling](#anti-tiling) |

The **Displacement** section below is covered in [Terrain](displacements.md).

![The Face tool with a rock face picked in cyan, and the face panel in the inspector with the face laid over its rock texture in the UV section](img/materials-face-panel.webp#wide)

## The UV editor

The UV editor lays the picked faces out flat over their texture, like Blender's UV Editor.
Pick faces with the Face tool and it shows in the **UV** section at the top of the face panel, as in TrenchBroom's face inspector.

- The section's bar shows the texture size; the target button at its right end (**Fit UV View**) fits the picked faces in the view.
- Click the bar to fold the section away; MapCraft remembers it.

For a bigger one, turn a view into the UV editor:

1. Pick faces with the Face tool.
2. Click a view's name in its header.
3. Choose **UV Editor**.
   Pick 3D, Top, Front or Side there to turn it back.

![A view's header menu open on UV Editor](img/materials-uv-menu.webp)

In the UV editor:

- The active face's texture repeats in every direction, with the first tile outlined in orange.
- Each picked face is drawn on it, filled with the color of its island (faces that share an edge and a material).
- The active face, the one you picked last, has a white outline.
- An orange box surrounds the picked faces, with square handles to scale them and a round one above to rotate them.
- Picks are shared with the Face tool: pick here or in the 3D view, and both show it.
- The inspector's UV editor and each view's keep their own zoom and pan.
- Picking other faces outside a UV editor, in the 3D view or by coming back to the Face tool, fits them in it again; picks made in a UV editor leave the zoom alone.

| Do this | To |
|---|---|
| Scroll | Zoom |
| Middle or right drag | Pan |
| Double click or `F` (**Fit UV View**) | Fit the picked faces in the view |
| Click a face | Pick only that face; Ctrl+click or Shift+click adds or removes it |
| Drag a picked face | Move the picked faces' textures; dragging another face picks it first, Ctrl or Shift adds it to the picks |
| Drag a corner handle | Scale the textures about the opposite corner; hold Shift to keep their aspect |
| Drag an edge handle | Scale the textures along one axis about the opposite edge |
| Drag the round handle | Rotate the textures about the middle of the box |
| Drag a box from empty space | Pick the faces whose middle is inside it; with Ctrl or Shift it adds them |
| Right click | Open the unwrap commands and **Fit UV View** |

While dragging a face or handle, hold `Ctrl` to snap a move to whole texels, a rotation to 15 degrees and a scale to tenths.
Press `Escape` or right click before letting go to put everything back.
Each drag is one undo step.

![The UV editor in the Top view's place, showing a ramp's faces laid out over the metal walkway texture](img/materials-uv-editor.webp)

### Grab, rotate and scale

With the pointer over a UV editor, press `G` to grab, `R` to rotate or `S` to scale the picked faces' textures.
The textures follow the pointer, in the 3D view too, until you finish.

| Key | Does |
|---|---|
| `X` or `Y` | Locks a grab or scale to U or to V; press again to unlock |
| Hold `Ctrl` | Snaps a grab to whole texels, a rotation to 15 degrees and a scale to tenths |
| Click or `Enter` | Applies the change |
| `Escape` or right click | Puts everything back |

Every picked face moves by the same amount, so an island stays whole.
The whole change is one undo step.
These keys only work over a UV editor: `R` still picks the Rotate tool in the other views.

### Unwrapping

Right click in the UV editor, or open **Tools > UV**, for four commands on the picked faces:

| Command | Does |
|---|---|
| **Unwrap** (`U` over a UV editor) | Lays each island out so the texture runs on across its shared edges without a seam |
| **Project from View** | Projects one texture onto every picked face, the way the 3D view sees it |
| **Cube Projection** | Projects each face's texture from the nearest world axis, like **Align to world** |
| **Reset** | Gives each face the mapping a new face gets |

Unwrap does what [wrapping](#wrapping-alignment) does by hand, for every face of an island at once.
A closed shape such as a box still has one seam where the faces meet again.

![The UV editor's right click menu: Unwrap, Project from View, Cube Projection and Reset](img/materials-unwrap.webp)

> [!TIP]
> The right click menu in any view has a **UV** submenu that also works on selected brushes: right click a ramp and choose **UV > Unwrap**.

## Texturing whole brushes

With brushes selected in the Select tool, the inspector's **Brushes** section has:

- **Material**, which names the material or how many the faces use; click it to open the Materials panel, or click the bucket (**Apply current material**) to put the current material on every face.
- **Texture scale**, which gives every face that scale as you drag or type; flipped textures stay flipped, and it reads **mixed** when the faces differ.
- **Shift**, four arrows that slide every face's texture 16 texels.
- **Align**, with **Align to world** and **Align to face** for every face.
- **Anti-tiling**, which sets **Off**, **Variation** or **Hex tiling** on every face, such as a whole run of ramps.

![The Brushes section for whole brushes: Material, Texture scale, Shift and Align](img/materials-brushes.webp)

## Anti-tiling

The **Anti-tiling** row in the face panel and the Brushes section has three settings:

- **Off** draws the texture as it is.
- **Variation** lays large, soft light and dark patches over the texture, which suits every texture, pixel art and bricks included.
- **Hex tiling** also breaks the texture into hexagons about one repeat across, each shifted and turned at random and blended into its neighbors, so even a close look finds no grid.

Hex tiling suits textures without a fixed direction or layout: rock, sand, gravel, dirt, grass, moss and snow.
Keep bricks, tiles, planks, panels and anything with straight lines or text on **Variation**, since the turned hexagons break their pattern apart.
Pixel textures, those of an SDK that draws textures pixelated or that the game draws without smoothing, use **Variation** even when set to **Hex tiling**, and the row says so.
The views show the result at once, and the Godot importer draws the same tiles in the same places, except as noted in [Godot](godot.md#anti-tiling).
Source and Quake have no anti-tiling, so these faces export plain, and the Output panel counts them.

## Texture lock

Texture lock is on by default.
Toggle it with `Shift+L`, the padlock (**Texture lock**) on the top bar, or **Tools > Texture Lock**.

- **On:** textures stick to brushes as you move, rotate, scale, flip, duplicate or paste them, and to a face you extrude or slide.
- **Off:** textures stay put in the world and the brushes slide under them.

Mirror groups have their own texture lock; see [Mirror groups](mirror-groups.md).

![The Texture lock button on the top bar, switched on](img/materials-texture-lock.webp)

## Replace Materials

**Tools > Replace Materials** swaps one material for another on many faces at once.

1. **Find** starts as a material of the selected brushes; type another to change it.
2. **Replace with** starts as the current material.
3. Choose **Selection** or **Whole map**.
4. Click **Replace**.

Only faces whose whole material name matches **Find** change, ignoring case.
Tick **Replace text inside names** to swap a piece of text in every name instead (matching case), for example `dev/` with `concrete/` to move a set of materials to another folder.
The status bar says how many faces changed, and one `Ctrl+Z` undoes it all.
It has no key by default; bind one in **Edit > Preferences**.

![The Replace Materials window with numbered Find, Replace with, Selection and Replace](img/materials-replace.webp)
