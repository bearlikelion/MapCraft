# Tools

Keys on this page are the MapCraft preset's defaults.
Every shortcut can be rebound, and the Hammer and TrenchBroom presets change some of them: see [Keyboard shortcuts](shortcuts.md).

The tools sit in the column of buttons left of the views; hover one to see its key.
The status bar shows a hint for the current tool.
`Escape` returns to the Select tool from any other.
A whole drag is one undo step, so `Ctrl+Z` takes all of it back.

## Select

Press `Q`.

- Click an object in any view to select it.
- Click the same spot again to select the object behind it.
- Ctrl+click or Shift+click adds an object to the selection, or takes it out.
- Click empty space to select nothing.

What a click selects follows the selection mode on the top bar: **Groups** (`Alt+1`) selects the outermost group around what you click, **Objects** (`Alt+2`) the entity or loose brush and ignores groups, **Solids** (`Alt+3`) single brushes.
See [Organizing](organizing.md) for groups and layers.

In a 2D view:

- Drag an object to move the selection; an object that is not selected yet gets selected first.
- Drag from empty space to draw a box; it selects everything that lies fully inside it.
- Drag the white handles around the selection to resize it; the opposite side stays put, and it cannot shrink below one grid step.
  The sides that move are highlighted in red in every view, and labels show the size in units.
- The arrow keys nudge the selection one grid step.

Shift+drag a face of a selected brush to extrude it, as the [Extrude](#extrude) tool does; Ctrl+Shift+drag splits off a new brush and Ctrl+Alt+Shift+drag stamps one.
Dragging in the 3D view does nothing else with Select; use Move there.

## Move

Press `W`.

Move works like Select and adds three arrows at the selection's center in the 3D view.
Drag the red, green or blue arrow to move the selection along X, Y or Z.
With **Snap** on, the selection's low side along that axis lands on the grid.

## Extrude

Press `E`.

1. Select the brushes to work on.
2. Hover a face to highlight it in red with a yellow outline; it stays highlighted while you drag.
   In the 3D view this is the face under the pointer; in a 2D view it is the face seen edge on nearest the pointer, preferring the one that faces it.
3. Drag to push or pull the face along its normal; the brush grows or shrinks with it.

Faces of other selected brushes that lie in the same plane move along.

| Mouse | Does |
|---|---|
| Drag | Moves the face, resizing its brush |
| Ctrl+drag | Leaves the brush alone and adds a new brush in the dragged volume; dragged inward, it splits the brush in two |
| Ctrl+Alt+drag | Stamps a new brush between the face and a moved copy of it |

The status bar shows how far you have dragged.
With **Snap** on, faces that point straight along an axis land on the grid, and slanted faces move in whole grid steps.
A drag that would change how many faces the brush has is refused.
After a split or stamp the new brushes are selected, ready for the next drag.

## Rotate

Press `R`.

- In a 2D view, drag around the selection's center, marked with an orange circle and cross, to turn it about the view's axis.
- In the 3D view, drag one of the three rings around the selection: red turns about X, green about Y, blue about Z.

With **Snap** on, rotation goes in 15 degree steps; the status bar shows the angle.
For exact quarter turns about Z, use **Tools > Rotate 90° Clockwise** (`Ctrl+R`) and **Tools > Rotate 90° Counter-Clockwise** (`Ctrl+Shift+R`).

## Scale

Press `T`.

- In a 2D view, drag the white handles, as with Select.
- In the 3D view, drag a square handle at the end of the red, green or blue lines to stretch the selection along X, Y or Z.
  Each axis has a bright handle on its positive side and a dim one on its negative side; the side you drag moves and the opposite side stays put.

The selection cannot shrink below one grid step, and with **Snap** on its edge lands on the grid.
While you drag, the side that moves is highlighted in red and labels show the selection's size in units, as they do for resizing with Select.

## Block

Press `B`.

Pick a shape on the top bar, right click the Block icon in the tool column (the small corner mark says it has a menu), or press `Alt+B` to step through the built-in ones.

| Shape | Makes | Options |
|---|---|---|
| Box | A box | |
| Wedge | A ramp: full height at its low X side, sloping down to its high X side | |
| Cylinder | An upright prism inside the box you draw | sides, 3 to 64 |
| Spike | A pyramid or cone with its tip at the top | sides, 3 to 64 |
| Sphere | A sphere, or an egg shape when the box is uneven | sides, 4 to 32 |
| Arch | Blocks in a curve around Z, running **arc** degrees counter-clockwise from **start** degrees off +X, each **wall** units thick | sides, 1 to 64, arc, start, wall |

While you draw, labels beside the box show its size in units along each axis, in every view.
New brushes get the current material (see [Materials](materials.md)), go wherever new objects go (see [Organizing](organizing.md)), and end up selected.

### Drawing in a 2D view

Drag a rectangle.
Its depth, along the axis the view looks down, matches the last block you drew; the first block is 64 units deep.

### Drawing in the 3D view

1. Press on a face to start the base on that face's plane, or on empty ground to start it on the Z = 0 plane.
2. Drag to size the base; the block rises away from the face as far as the last block you drew measured along the same axis.
3. Hold `Shift` while dragging to set the height instead; let go of `Shift` to go back to sizing the base.
4. Release the mouse button to make the block.

### Generator shapes

Plugins can add shapes to the bar, such as Surf Ramp and Bhop Platforms in the SurfsUp SDK (see [Surf mapping](../surf-mapping.md)).

1. Click the shape on the top bar.
   Its parameters show in the inspector, with **Turn** to rotate it (the **90°** button adds a quarter turn counter-clockwise).
2. A yellow wireframe ghost follows the pointer: on the face or Z = 0 ground under it in the 3D view, or at the depth of the last block in a 2D view.
   It stays where the pointer left it, so you can watch inspector changes on it.
3. Click to place it.

A placed generator is a group that stays editable: select it and change any parameter in the inspector, and its brushes are rebuilt.
Each shape remembers the parameters you last used until you quit.
`Alt+B` switches back to the built-in shapes.
The **Plugins** menu runs the same generators from a dialog instead.
To write your own, see the [Plugin API](../plugin-api.md).

## Entity

Press `N`.

Click a point entity class in the Entities panel (`Ctrl+4`); that also switches to this tool.
Then click to place one:

- In a 2D view it lands under the pointer, at the depth of the last block you drew (in the Top view, at that block's bottom).
- In the 3D view it lands on the surface under the pointer, snapped to the grid; clicking empty space places nothing.

The new entity gets the class's default values and is selected.
See [Entities and zones](entities.md) for editing entities and making brush entities.

## Clip

Press `C`.

1. Select the brushes to cut.
2. Drag a line, or click two points, in any view.
   In a 2D view the cut runs straight back through the view.
   In the 3D view, start on a face: the cut runs straight into that face, and the second point slides across the face's plane.
3. Check the preview: yellow parts stay, dark red parts go.
   Press `Alt+C`, or use **Both**, **Front** and **Back** on the top bar, to choose what to keep.
4. Press `Enter` to cut, or `Escape` to cancel and go back to Select.

With two points placed, click a third point on a face in the 3D view to tilt the cut: it then runs through all three points, which lets it cut at any angle.
Clicking again after that, or after two points in a 2D view, starts a new line.
After the cut, the pieces that stay are selected.

## Vertex

Press `V`.

Select brushes, then pick which parts of them to work on, as in Blender's edit mode:

| Mode | Key | Handles |
|---|---|---|
| Vertex | `1` | Yellow squares on the corners |
| Edge | `2` | Light blue squares on the edge midpoints |
| Face | `3` | Green dots on the face centers |

The mode buttons are also on the top bar, with **Merge** and **Bevel**.
These keys only work in the Vertex tool, where they win over the same keys elsewhere.
Switching mode keeps what the selected corners still fully cover, so two selected corners become their edge.

### Selecting

- Click a handle to select it; selected handles turn orange, and selected edges and faces are outlined in orange.
- Shift+click or Ctrl+click adds a handle, or takes it out.
- Drag a box from empty space to select every handle inside it; hold Shift to add to the selection, or Ctrl to take away.
- Click away from the handles to select brushes as usual.

In a 2D view, a click takes everything stacked under the pointer, such as a corner and the corners hidden behind it.
In the 3D view it takes the handle nearest you, along with the same corner of other selected brushes.
Each brush keeps its own selection, which follows the brush through edits and drops whatever no longer exists.

### Moving

Drag any selected handle to move the whole selection; dragging a handle that is not selected selects it first.
In the 3D view the selection moves across the level plane through the handle you grabbed; hold `Shift` while dragging to move it up and down instead.
The brush is always rebuilt as the convex shape around its corners; a move that leaves no valid brush is refused with "Cannot move there" in the status bar.
Dragged points snap to the grid and to other corners, as set by **Snap** and **Corners**.

### Merge

`M` collapses the selected corners of each brush to their center (in edge and face mode, the corners of the selected edges or faces), as Blender's Merge at Center does.
The merged corner stays selected.
A merge that would leave a flat or empty brush is refused with a message in the status bar.
Each merge is one undo step.

### Bevel

`Ctrl+B` bevels the selected edges, rounding them off with new faces the way Blender's bevel does, without clipping by hand.
In vertex and face mode it bevels the edges between selected corners.

1. Move the pointer away from the edges to set the width, how far the cut reaches back along both faces; it snaps to the grid while **Snap** is on.
2. Scroll to add or remove segments; one segment is a plain chamfer.
3. Click or press `Enter` to apply, or press `Esc` or right click to cancel.

The width and segments show next to the edges in every view and in the status bar.
After a bevel, the **Adjust Last Bevel** window changes its width, segments and profile, redoing the bevel in place of the last one as a single undo step; it closes once the brush is edited again.
Profile 0.5 is a round arc, 0.25 or less a flat chamfer, and values toward 1 hug the original corner more closely for a boxier edge.

Each new face takes the material of the face beside the edge it is nearest to, with the texture turned on across the edge, so the pattern runs on around the rounded edge; the two halves of a bevel meet at one seam in the middle.

A bevel keeps every brush one convex solid, so some are refused with a reason in the status bar:

- an edge that is not between two faces of the brush,
- a width larger than a face beside the edge,
- two edges whose bevels would run into each other, such as edges that share a corner.

Edges on opposite sides of a face can bevel together, and meet in the middle when the width is half the face.

## Face

Press `Shift+F`.

Switching to Face turns every face of the selected brushes into a picked face and clears the selection; leaving Face drops the picks, except for the Sculpt tool, which keeps them to limit its strokes.
Faces hidden between touching brushes, such as the joins between the pieces of a ramp, are left out, so texturing or unwrapping a whole ramp only gets the faces you can see; the status bar says how many were left out.

- Click a face in the 3D view to pick just that face.
- Ctrl+click or Shift+click to add a face, or take it out.
- Click empty space to pick nothing.

Picked faces keep their true color, outlined, so you can judge their materials.
`Ctrl+C` copies the active face's material and alignment and `Ctrl+V` pastes them on the picked faces: see [Copying and pasting faces](materials.md#copying-and-pasting-faces).
A view turned into the UV editor shows the picks over their texture, and picking there picks here: see [The UV editor](materials.md#the-uv-editor).
The inspector shows the face panel: see [Materials](materials.md#the-face-panel) for texturing and [Displacements](displacements.md) for terrain.

## Sculpt

Press `D`.

Drag on a displacement in the 3D view to raise, lower, smooth, flatten or roughen it; mode, radius and strength are on the top bar, and `Ctrl` swaps raise and lower.
See [Displacements](displacements.md#the-sculpt-tool).

## Path

Press `P`.

Click in a view to add points of a Bezier path and drag to pull out their handles; sweeps, spins and arrays along a path live under **Tools > Curves**.
See [Paths, sweeps and spins](curves.md).

## Brush operations

These work on the selection and live in the **Tools** menu.

| Command | Key | Does |
|---|---|---|
| **Tools > Carve** | `Ctrl+Shift+C` | Cuts the selected brushes out of every other brush they touch that you can see (hidden, tagged-off, locked and out-of-cordon brushes are left alone); the selected brushes stay |
| **Tools > Hollow** | `Ctrl+H` | Turns each selected brush into walls one grid step thick around an empty inside, and selects the walls |
| **Tools > Merge Brushes** | `Ctrl+J` | Replaces two or more selected brushes with one convex brush around them all |
| **Tools > Flip X** | `Ctrl+L` | Mirrors the selection along X through its center |
| **Tools > Flip Y** | `Ctrl+I` | Mirrors the selection along Y through its center |
| **Tools > Flip Z** | `Ctrl+Alt+L` | Mirrors the selection along Z through its center |
| **Tools > Rotate 90° Clockwise** | `Ctrl+R` | Turns the selection a quarter turn about Z through its center |
| **Tools > Rotate 90° Counter-Clockwise** | `Ctrl+Shift+R` | The same, the other way |
| **Tools > Snap Selection to Grid** | `Ctrl+B` | Moves each selected object, without reshaping it, so its lowest corner (or an entity's or prefab's origin) sits on the nearest grid point |
| **Tools > Transform** | `Ctrl+M` | Opens the Transform dialog |
| **Tools > Array** | `Ctrl+Shift+D` | Opens the Array dialog |

The brush inspector also has **Carve** and **Hollow** buttons.

### The Transform dialog

Pick **Move**, **Rotate** or **Scale**, type X, Y and Z values, and click **Apply**.

- Move: units along each axis.
- Rotate: degrees about each axis through the selection's center, applied about X, then Y, then Z.
- Scale: factors along each axis from the selection's center.

### The Array dialog

The Array dialog makes copies of the selection in one undo step, like Hammer's Paste Special.

- **Copies**: how many, up to 256.
- **Offset**: how far each copy moves from the one before it.
- **Turn**: degrees each copy turns from the one before it, about X, then Y, then Z, through the **Pivot**.
- **Pivot at selection** and **Pivot at origin** fill in the pivot.

A row of pillars is an offset with no turn; a ring of them is a turn about Z with the pivot at the ring's center and no offset; a spiral stair is both.
The copies end up selected.

Moves, turns, flips and scales keep textures on the brushes while texture lock is on; see [Materials](materials.md#texture-lock).
