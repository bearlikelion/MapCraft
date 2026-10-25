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
2. Hover a face to outline it in yellow.
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
- In the 3D view, drag the square handle at the end of the red, green or blue line to stretch the selection along X, Y or Z.
  Its low side stays put, so it grows or shrinks toward the positive end of the axis.

The selection cannot shrink below one grid step, and with **Snap** on its edge lands on the grid.

## Block

Press `B`.

Pick a shape on the top bar, or press `Alt+B` to step through the built-in ones.

| Shape | Makes | Options |
|---|---|---|
| Box | A box | |
| Wedge | A ramp: full height at its low X side, sloping down to its high X side | |
| Cylinder | An upright prism inside the box you draw | sides, 3 to 64 |
| Spike | A pyramid or cone with its tip at the top | sides, 3 to 64 |
| Sphere | A sphere, or an egg shape when the box is uneven | sides, 4 to 32 |
| Arch | Blocks in a curve around Z, running **arc** degrees counter-clockwise from **start** degrees off +X, each **wall** units thick | sides, 1 to 64, arc, start, wall |

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

Clicking again after two points starts a new line.
After the cut, the pieces that stay are selected.

## Vertex

Press `V`.

Select brushes, then work in a 2D view, where their handles show:

| Handle | Moves |
|---|---|
| Yellow square | A corner |
| Light blue square | An edge's two corners |
| Green dot | All of a face's corners |

Drag a handle to move it.
Everything stacked under the pointer in that view moves together: a corner takes the corners hidden behind it, and an edge handle can take a whole side.
Corners win over edges and edges over faces when handles overlap.
The brush is always rebuilt as the convex shape around its corners; a move that leaves no valid brush is refused with "Cannot move there" in the status bar.
Dragged points snap to the grid and to other corners, as set by **Snap** and **Corners**.

## Face

Press `Shift+F`.

Switching to Face turns every face of the selected brushes into a picked face and clears the selection; leaving Face drops the picks, except for the Sculpt tool, which keeps them to limit its strokes.

- Click a face in the 3D view to pick just that face.
- Ctrl+click or Shift+click to add a face, or take it out.
- Click empty space to pick nothing.

Picked faces keep their true color, outlined, so you can judge their materials.
The inspector shows the face panel: see [Materials](materials.md#the-face-panel) for texturing and [Displacements](displacements.md) for terrain.

## Sculpt

Press `D`.

Drag on a displacement in the 3D view to raise, lower, smooth, flatten or roughen it; mode, radius and strength are on the top bar, and `Ctrl` swaps raise and lower.
See [Displacements](displacements.md#the-sculpt-tool).

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

The brush inspector also has **Carve** and **Hollow** buttons.

### The Transform dialog

Pick **Move**, **Rotate** or **Scale**, type X, Y and Z values, and click **Apply**.

- Move: units along each axis.
- Rotate: degrees about each axis through the selection's center, applied about X, then Y, then Z.
- Scale: factors along each axis from the selection's center.

Moves, turns, flips and scales keep textures on the brushes while texture lock is on; see [Materials](materials.md#texture-lock).
