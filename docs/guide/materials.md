# Materials

Materials are the textures on the faces of your brushes, the solid blocks a map is built from.
This page shows you how to pick a material, put it on faces and line it up so it looks right.
Keys here are from the MapCraft preset; [Keyboard shortcuts](shortcuts.md) lists the Hammer and TrenchBroom presets.

## The current material

The current material is the one new brushes get, and the one the apply commands put on faces.
You can see it on the top bar in the material chip, which shows a swatch and the material's name.
Click the chip to open the Materials panel.

When the SDK (the package that tells MapCraft about your game) names a default material and texture scale, they become current as the SDK loads.

![The top bar with the Material chip highlighted, showing the current material ramps-orange1](img/materials-current.webp)

## The Materials panel

The Materials panel is where you browse your materials and choose one.
Open the **Materials** tab on the left with `Ctrl+3`.
It lists every material of the map's SDK, including the [texture folders](sdks-and-plugins.md#texture-folders) you added in SDK Settings.

1. Type in **Search materials** to filter by name.
   Searching matches folder names as well as material names.
2. Click the **Previews** button at the right of the material count to switch between pictures and a plain list of names.
   Its icon is a grid of four tiles while it shows pictures, and a list while it shows names.
3. Click a material to make it current.
   With brushes selected, or faces picked with the Face tool, the click also puts it on them, as in TrenchBroom.
4. Click **Apply to selection** (the paint bucket) to give the current material to the picked faces, or else to every face of the selected brushes.
5. Right click a material for **Select Faces Using It**, which picks every face using it in the Face tool, or **Select Brushes Using It**, which selects every brush with a face using it.
   Hidden and locked brushes are left out.
   **Edit > Select by Material** does the same for the current material.

![The Materials panel with numbered markers on the search box, the Previews toggle, a material picture and Apply to selection outlined](img/materials-panel.webp)

To shorten a long list, open **Folders** under the search box.
It lists the SDK's top-level material folders, or the folders its plugin puts materials in, such as one per WAD file on the Quake SDK.
Untick a folder to hide its materials, or click **Hide all** and tick the ones you want.
The choice is saved per SDK, so it holds for every map on that SDK.
Faces already using a hidden material still draw.

> [!TIP]
> To make a click only set the current material, turn off **Clicking a material applies it to the selection** in **Edit > Preferences**.
> A double click then applies it.

> [!NOTE]
> Without an SDK the panel asks you to pick one with **File > New Map**.

## Applying a material

There are several ways to put the current material on faces, so use whichever is closest to hand.

| Do this | Puts the current material on |
|---|---|
| `Shift+T` (**Tools > Apply Current Material**) | The picked faces, or else every face of the selected brushes |
| The bucket (**Apply current material**) in the inspector | The same |
| Click a material in the Materials panel | The same, unless you turned that off |
| Alt+right click a face in the 3D view | Only that face |

Alt+right click works in any tool, so it is the fastest way to paint a few faces without switching tools.

To take a material off, use **Tools > Clear Material**.
It works on the picked faces, or else on every face of the selected brushes, and leaves them untextured.
You can also find it in the right click menu, and as **Clear material** (the eraser) beside **Apply to selection** in the Materials panel.

> [!NOTE]
> Untextured faces export as `tools/toolsnodraw` in a VMF, except on triggers, which export as `tools/toolstrigger`.

## Picking a material

To reuse a material you can see in the map, Alt+click its face in the 3D view, like an eyedropper.
It becomes the current material, and the status bar shows **Picked** and the material's name.
MapCraft also remembers that face's alignment, for [wrapping](#wrapping-alignment).
In the Face tool with a face picked, Alt+click [transfers](#transferring-a-face) instead.

![Alt+clicking the side of a ramp makes MetalWalkway011 the current material on the top bar](img/materials-pick.webp)

## Copying and pasting faces

When one face looks right, you can copy its look onto others.

1. Press `Shift+F` for the Face tool and click the face that looks right, or Shift+click it in the 3D view from the Select or Move tool.
2. Press `Ctrl+C` to copy it.
3. Pick the faces to change and press `Ctrl+V`.

A paste copies the material, the alignment and settings such as lightmap scale.
It turns the texture across shared edges, so the texture runs on around corners.
**Paste face** in the face panel does the same.
Alt+click also copies the face it picks.

## Wrapping alignment

Wrapping carries a texture around a corner without a seam, one face at a time.

1. Alt+click a face that already looks right.
2. Alt+Shift+right click the next face along.
3. Keep going, face after face.

Each face gets the material, the alignment and settings such as lightmap scale of the face you Alt+clicked.
The texture is turned across the edge where the two faces meet, as a paste does.

> [!NOTE]
> Until you have Alt+clicked a face, Alt+Shift+right click only applies the current material.

## Transferring a face

Transferring is TrenchBroom's way of passing one face's look on to others, straight from the 3D view.
It works from the active face of the Face tool, the one you picked last.

1. Pick the face that looks right: Shift+click it in the 3D view from the Select or Move tool, or click it with the Face tool.
2. Hold the modifier for what to carry:
   - `Alt` carries the material, the alignment and settings such as lightmap scale, with the texture axes exactly as they are.
   - `Alt+Shift` carries the same, but turns the texture axes onto each face about the edge it shares with the source, so the texture runs on around a corner.
   - `Ctrl+Alt` carries only the material, and each face keeps its own alignment.
3. Click, drag or double-click in the 3D view:
   - A click changes the face under the pointer.
   - A drag changes every face the pointer passes over, each one from the face changed just before it, so a drag around a pillar wraps the texture all the way around.
   - A double-click changes every face of the brush under the pointer.

The source stays picked, so you can keep clicking faces.
While you drag, each face changed lights up, and the status bar names the mode, such as **Transfer material and alignment (rotated)**.
Each click, drag or double-click is one undo step.

Content flags, kept in a face's `contents` attribute by games that have them, never transfer: every face keeps its own.

`Alt` suits faces that point the same way, such as the tops of a row of steps.
On a face turned away from the source, such as a wall below a floor, the texture smears into stripes, because the axes run across the face instead of along it.
`Alt+Shift` keeps the texture's size on every face, so use it around corners.
It works on every map, not only Valve format ones as in TrenchBroom, since MapCraft keeps free texture axes (the Valve220 kind) on every face, and the Quake map and VMF exports write them as they are.

With no face picked, Alt+click picks a material as before, and Alt+right click and Alt+Shift+right click still apply and wrap in any tool.

## The face panel

The face panel is where you fine-tune a texture on its faces: slide it, scale it, turn it or fit it.
Press `Shift+F` for the Face tool and pick faces (see [Tools](tools.md#face)).
The inspector shows the face panel, with the **UV**, **Surface** and **Alignment** sections.
**UV** at the top is the [UV editor](#the-uv-editor).

Every change applies to all picked faces.
Each field shows the picked faces' value, or **mixed** when they differ.
Dragging a field moves each face's own value by the same amount, so faces that differed stay different.
Typing a number gives them all that number.

| Control | Does |
|---|---|
| **Material** | Shows the faces' material; click it to open the Materials panel, where clicking a material applies it to the picked faces |
| The bucket (**Apply current material**) | Gives the picked faces the current material |
| **Paste face** | Gives them the material and alignment of the face you last copied (`Ctrl+C` or Alt+click), wrapped across shared edges |
| **Offset** U and V | Slides the texture, in texels (the texture's pixels) |
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

The UV editor lays the picked faces out flat over their texture, like Blender's UV Editor, so you can see exactly where the texture sits and drag it into place.
Pick faces with the Face tool and it shows in the **UV** section at the top of the face panel, as in TrenchBroom's face inspector.

- The section's bar shows the texture size.
  The target button at its right end (**Fit UV View**) fits the picked faces in the view.
- Click the bar to fold the section away; MapCraft remembers it.

For more room, turn a whole view into the UV editor:

1. Pick faces with the Face tool.
2. Click a view's name in its header.
3. Choose **UV Editor**.
   Pick 3D, Top, Front or Side there to turn it back.

![A view's header menu open on UV Editor](img/materials-uv-menu.webp)

Here is what the UV editor shows:

- The active face's texture repeats in every direction, with the first tile outlined in orange.
- Each picked face is drawn over it, filled with the color of its island.
  An island is a set of faces that share an edge and a material.
- The active face, the one you picked last, has a white outline.
- An orange box surrounds the picked faces, with square handles to scale them and a round one above to rotate them.

Picks are shared with the Face tool: pick here or in the 3D view, and both show it.

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

> [!NOTE]
> The inspector's UV editor and each view's keep their own zoom and pan.
> Picking other faces outside a UV editor, in the 3D view or by coming back to the Face tool, fits them in it again.
> Picks made in a UV editor leave the zoom alone.

### Grab, rotate and scale

If you know Blender, these keys will feel familiar.
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

> [!NOTE]
> These keys only work over a UV editor: `R` still picks the Rotate tool in the other views.

### Unwrapping

Unwrapping lays the textures out for you.
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

You do not need the Face tool to texture whole brushes.
With brushes selected in the Select tool, the inspector's **Brushes** section has:

- **Material**, which names the material, or how many the faces use.
  Click it to open the Materials panel, or click the bucket (**Apply current material**) to put the current material on every face.
- **Texture scale**, which gives every face that scale as you drag or type.
  Flipped textures stay flipped, and it reads **mixed** when the faces differ.
- **Shift**, four arrows that slide every face's texture 16 texels.
- **Align**, with **Align to world** and **Align to face** for every face.
- **Anti-tiling**, which sets **Off**, **Variation** or **Hex tiling** on every face, such as a whole run of ramps.

![The Brushes section for whole brushes: Material, Texture scale, Shift, Align and Anti-tiling](img/materials-brushes.webp)

## Anti-tiling

A texture repeated across a big ramp or floor shows a grid you can spot from far away.
Anti-tiling hides it.
The **Anti-tiling** row in the face panel and the Brushes section has three settings:

- **Off** draws the texture as it is.
- **Variation** lays large, soft light and dark patches over the texture.
  It suits every texture, pixel art and bricks included.
- **Hex tiling** also breaks the texture into hexagons about one repeat across.
  Each is shifted and turned at random and blended into its neighbors, so even a close look finds no grid.

Use **Hex tiling** on textures without a fixed direction or layout: rock, sand, gravel, dirt, grass, moss and snow.
Keep bricks, tiles, planks, panels and anything with straight lines or text on **Variation**, since the turned hexagons break their pattern apart.

The views show the result at once, and the Godot importer draws the same tiles in the same places, except as noted in [Godot](godot.md#anti-tiling).

> [!NOTE]
> Pixel textures, those of an SDK that draws textures pixelated or that the game draws without smoothing, use **Variation** even when set to **Hex tiling**, and the row says so.
> Source and Quake have no anti-tiling, so these faces export plain, and the Output panel counts them.

## Texture lock

Texture lock decides whether textures move with your brushes.
It is on by default.
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
To swap a piece of text in every name instead, tick **Replace text inside names** (this matches case).
For example, replace `dev/` with `concrete/` to move a set of materials to another folder.
The status bar says how many faces changed, and one `Ctrl+Z` undoes it all.

![The Replace Materials window with numbered Find, Replace with, Selection and Replace](img/materials-replace.webp)

> [!TIP]
> Replace Materials has no key by default; bind one in **Edit > Preferences**.
