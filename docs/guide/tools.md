# Tools

This page is a reference for every tool in the tool column, the right click menu, and the brush commands in the **Tools** menu.
The tools sit in the column left of the views: click one or press its key.
Keys here are from the MapCraft preset (see [Keyboard shortcuts](shortcuts.md)).

## The tool column

1. **Select** `Q`
2. **Move** `W`
3. **Extrude** `E`
4. **Rotate** `R`
5. **Scale** `T`
6. **Block** `B`
7. **Entity** `N`
8. **Clip** `C`
9. **Vertex** `V`
10. **Face** `Shift+F`
11. **Sculpt** `D`
12. **Path** `P`

Hover a tool to see its key, and read the status bar for a hint on using it.
`Escape` goes back to Select from any other tool, and in Select it clears the selection.
A whole drag is one undo step, so `Ctrl+Z` takes all of it back.

![The tool column with its twelve tools numbered from Select at the top to Path at the bottom](img/tools-column.webp)

## The right click menu

The right click menu puts the commands for what you have selected right under the pointer, as in Blender.
Right click in any view, without dragging, to open it.
What it holds depends on what is selected:

- **Objects selected:** Duplicate, Duplicate Linked and Delete, Apply Current Material and Clear Material, then Hollow, Carve, the **Boolean** submenu, Merge and Clip, then the **Brush Type**, **Transform** (which holds Drop to Floor and Randomize too), **UV**, **Organize** and **Curves** submenus, then Focus, Hide, Isolate and Lock.
  A linked copy adds its linked copy commands.
- **Faces picked in the Face tool:** Apply Current Material, Clear Material, Copy Face, Paste Face, the **UV** submenu, Sweep Along Path and Spin.
- **In the Vertex tool:** the select modes, Merge at Center, Bevel Edges and Delete Vertices (Dissolve Faces in face mode) come first, with Set Origin to Selected Corners when a brush entity is selected.
- **Nothing selected:** **Add** (every Block shape and generator, Entity and Path), Select All, Show All, Unlock All, Frame All and Go to Origin.

Each entry shows its shortcut, and entries that cannot run are grayed out.

![The right click menu over a selected surf ramp, from Duplicate down to Lock](img/tools-right-click.webp)

![The right click menu with nothing selected: Add, Select All, Show All, Unlock All, Frame All and Go to Origin](img/tools-right-click-empty.webp)

> [!TIP]
> The **UV** submenu also works on selected brushes: right click a ramp and choose **UV > Unwrap**.
> **Edit UVs in This View** turns the view you clicked into the [UV editor](materials.md#the-uv-editor).

> [!NOTE]
> Right dragging still flies the 3D camera and pans a 2D view.
> Alt+right click still applies the current material to a face, and a right click still cancels a bevel being sized.

## Select

Select picks the objects that the other tools and commands work on.
Press `Q`.

- Click an object to select it.
- Click the same spot again to select the object behind it.
- Ctrl+click adds an object, or takes it out; Shift+click does too in a 2D view.
- Shift+click a face in the 3D view to pick only that face in the [Face](#face) tool.
- Click empty space to select nothing.

Labels show the size of the selection in every view; with several objects selected, they show the size of the box around them all.

What a click selects follows the selection mode, set by the three buttons on the top bar: **Groups** (`Alt+1`), **Objects** (`Alt+2`) or **Solids** (`Alt+3`).
See [Organizing](organizing.md#selection-modes).

### Select in a 2D view

In a 2D view, Select also moves and resizes what you have selected.

- Drag an object to move the selection.
  Dragging over the selection moves it even when another object is nearer in that view, such as a wall above a selected group.
- Drag from empty space to draw a box; it selects everything fully inside.
- Drag a white handle to resize the selection.
  The opposite side stays put, the moving sides turn red, and labels show the size.
- Ctrl+drag moves a copy and leaves the original, in one undo step.
- The arrow keys nudge the selection one grid step, Shift+arrow keys one unit, and Ctrl+arrow keys nudge a copy.

A selection cannot shrink below one grid step.

![Resizing a ramp in the Top view: the moving side is red and labels show its size in every view](img/tools-select-resize.webp)

Shift+drag a face of a selected brush to extrude it, as the [Extrude](#extrude) tool does.
Ctrl+Shift+drag splits off a new brush, Ctrl+Alt+Shift+drag stamps one, and Alt+Shift+drag slides the face.

### Select in the 3D view

In the 3D view, drag an object to move the selection, with no handles.

- A drag slides the selection across the level of the point you grabbed, and that point follows the pointer.
- Alt+drag moves the selection straight up and down.
  Press or let go of Alt partway through to switch, and the selection stays where it is.
- Shift does the same as Alt once the drag has started, for desktops that move windows on Alt+drag.
- Dragging an object that is not selected selects it and moves it.
  Dragging over the selection moves it even when another object is nearer, so an object picked by clicking again drags from the same spot.
- Ctrl+drag moves a copy and leaves the original.
- With **Snap** on, the selection's corner nearest the grab point lands on the grid.

Each drag is one undo step, and the status bar shows how far it has moved.

## Move

Move drags the selection along one axis in the 3D view.
Press `W`.

Move works like Select, and adds three arrows at the selection's center in the 3D view.
Drag the red, green or blue arrow to move along X, Y or Z, or Ctrl+drag it to move a copy.
With **Snap** on, the selection's low side along that axis lands on the grid.
Drag the selection itself to move it as [Select](#select-in-the-3d-view) does.

![The Move tool's red, green and blue arrows on a selected ramp in the 3D view](img/tools-move.webp)

## Extrude

Extrude pushes or pulls one face of a brush, and the brush grows or shrinks to follow it.
Press `E`.

1. Select the brushes to work on.
2. Hover a face: it turns red with a yellow outline.
3. Drag to push or pull the face; the brush grows or shrinks with it.

| Mouse | Does |
|---|---|
| Drag | Moves the face, resizing its brush |
| Alt+drag | Slides the face: it keeps its shape and the faces around it tilt to follow; in a 2D view it moves freely, shearing the brush |
| Ctrl+drag | Adds a new brush in the dragged volume instead; dragged inward, it cuts the brush in two |
| Ctrl+Alt+drag | Stamps a new brush between the face and a moved copy of it |

With the Select and Move tools, hold Shift as well.

![Extrude: the hovered top face of a platform in red, then the platform pulled taller](img/tools-extrude.webp)

### Stretch or along normal

Two buttons on the top bar pick how a drag moves the face:

- **Stretch** (an up and down arrow): the face's plane moves, and the faces around it stretch to meet it, as in TrenchBroom.
  This is the default.
- **Along normal** (an arrow rising from a line): the face moves straight out and keeps its shape, and new sides stand square to it.

On a box both do the same; they differ on slanted faces, such as the slope of a ramp.
MapCraft remembers the pick, and Shift+drag with the Select and Move tools uses it too.

### Which face a drag takes

- In the 3D view, it is the face under the pointer.
  Away from the brushes, the nearest outline edge takes the face hidden behind it, so you can pull a back or an underside without turning the camera.
- In a 2D view, it is the face seen edge on nearest the pointer.
  Away from the selection, the Extrude tool takes the nearest one however far off it is.
- Faces of other selected brushes that lie in the same plane move along.
- Where two selected brushes touch, the faces on both sides of the seam move together, so one brush grows as the other shrinks.
  In the 3D view, point at the edge where the seam meets the surface to take it.

### Extrude details

- The status bar shows how far you have dragged, and labels show the size of the selection.
- With **Snap** on, the drag stops where a corner of the face lands on the grid, so straight faces land on grid lines.
  A slide moves in whole grid steps along each axis.
- Faces that stop bounding the brush drop out, and come back if you drag back before letting go.
  A drag that would leave no volume is refused.
- Stamps only go outward, and their sides keep the textures of the walls they continue.
- Across a seam, Ctrl+drag moves the seam like a plain drag, and stamps are refused.
- After an outward split or a stamp, the new brushes are selected.
  After an inward split, the slab by the dragged face is a new brush and both pieces stay selected.
- `Escape` cancels a drag and puts everything back.
  A drag that ends where it started, or a split dragged past the whole brush, leaves nothing to undo.

## Rotate

Rotate turns the selection around its center.
Press `R`.

- In a 2D view, drag around the selection's center (the orange circle and cross) to turn it.
- In the 3D view, drag a ring: red turns about X, green about Y, blue about Z.

With **Snap** on, rotation goes in 15 degree steps, and the status bar shows the angle.
For exact quarter turns about Z, use **Tools > Rotate 90° Clockwise** (`Ctrl+R`) or **Tools > Rotate 90° Counter-Clockwise** (`Ctrl+Shift+R`).
The TrenchBroom shortcut preset adds TrenchBroom's quarter turns about the view under the pointer: roll (`Alt+Up` and `Alt+Down`) about the way the view looks, pitch (`Alt+Page Up` and `Alt+Page Down`) about its right side, and yaw (`Alt+Left` and `Alt+Right`) about Z in the 3D view or the view's up axis in a 2D view.

![The Rotate tool's red, green and blue rings around a selected ramp](img/tools-rotate.webp)

## Scale

Scale stretches or squashes the selection by dragging one of its sides.
Press `T`.

- In a 2D view, drag the white handles, as with Select.
- In the 3D view, drag a square handle at the end of the red, green or blue line.

Each axis has a bright handle on its positive side and a dim one on its negative side.
The side you drag moves and the opposite side stays put.
The moving side turns red and labels show the size.
A selection cannot shrink below one grid step, and with **Snap** on its edge lands on the grid.

![The Scale tool's handles on a selected ramp in the 3D view](img/tools-scale.webp)

## Block

Block draws new brushes, from plain boxes to cylinders, arches and generated shapes such as surf ramps.
Press `B`.

1. Pick the Block tool.
2. Pick a shape from the icons on the top bar (hover one for its name), or right click the Block icon, or press `Alt+B` to step through the shapes.
3. Drag a rectangle in a 2D view.

Labels show the block's size in every view while you draw.
New brushes get the current [material](materials.md), go [where new objects go](organizing.md#where-new-objects-go) and end up selected.

![Drawing a block: the Block tool, the shape icons with Surf Ramp and Bhop Platforms on the top bar, and a new box being dragged in the Top view with its size labels](img/tools-block.webp)

| Shape | Makes | Options |
|---|---|---|
| Box | A box | |
| Wedge | A ramp, full height at its low X side and sloping down to its high X side | |
| Cylinder | An upright prism inside the box you draw | sides, 3 to 64 |
| Spike | A pyramid or cone with its tip at the top | sides, 3 to 64 |
| Sphere | A sphere, or an egg shape when the box is uneven | sides, 4 to 32 |
| Arch | Blocks in a curve around Z, running **arc** degrees counter-clockwise from **start** degrees off +X, each **wall** units thick | sides 1 to 64, arc, start, wall |

The options show on the top bar next to the shapes.

### Drawing in a 2D view

Drag a rectangle.
Its depth matches the last block you drew; the first block is 64 units deep.

### Drawing in the 3D view

1. Press on a face to start on that face, or on empty ground to start at height 0.
2. Drag to size the base; the block rises as far as the last block you drew.
3. Hold `Shift` or `Alt` while dragging to set the height instead; let go to size the base again.
4. Release the mouse button to make the block.

### Generator shapes

Plugins add shapes to the bar, such as **Surf Ramp** and **Bhop Platforms** in the SurfsUp SDK.
These are generators: you give them a few settings and they build the brushes for you.
They show as named buttons with a puzzle piece, after the shape icons.

1. Click the shape.
   Its settings show in the inspector, with **Facing** to rotate it (the curved arrow next to it, **Turn a quarter turn counter-clockwise**, adds a quarter turn).
2. A yellow ghost follows the pointer: on the face or ground under it in the 3D view, or at the last block's depth in a 2D view.
3. Click to place it.

The ghost stays where the pointer left it and updates as you edit its settings.

- A placed generator is a group that stays editable: select it, change any setting in the inspector, and it rebuilds.
- Each shape remembers the settings you last used until you quit.
- `Alt+B` switches back to the built-in shapes.
- The **Plugins** menu runs the same generators from a dialog.

See [Surf ramps and bhop](surf.md), or the [Plugin API](../plugin-api.md) to write your own.

## Entity

Entity places point entities: things that sit at one spot in the map, such as spawn points, lights and props.
Press `N`.

1. Click a point entity class in the **Entities** tab (`Ctrl+4`); that also picks this tool.
2. Click in a view to place one.

Where it lands depends on the view:

- In the 3D view it lands on the surface under the pointer, snapped to the grid; clicking empty space places nothing.
- In a 2D view it lands under the pointer at the depth of the last block you drew (in the Top view, at that block's bottom).

The new entity gets its class's default values and is selected.
The tool starts out placing `info_player_start`.
See [Entities and zones](entities.md).

![An info_player_start placed on the start platform in the 3D view, with its keys in the inspector](img/tools-entity.webp)

## Clip

Clip cuts brushes in two along a line you draw, and keeps one side or both.
Press `C`.

1. Select the brushes to cut.
2. Click two points, or press and drag a line, in any view.
   In a 2D view the cut runs straight back through the view; in the 3D view, start on a face and the cut runs straight into it.
3. The points show as yellow squares, with their coordinates in the 2D views.
   Drag a point to move it.
4. Check the preview: yellow parts stay, red parts go.
   The bigger part stays at first; press `Ctrl+Enter` to cycle Front, Back and Both, as in TrenchBroom, or pick one on the top bar.
5. Press `Enter` to cut.
   `Escape` or `Delete` takes back the last point; with no points left, `Escape` leaves the tool.

After the cut, the pieces that stay are selected, and the Clip tool stays on for the next cut.
`Delete` never deletes the selected brushes while the Clip tool is on; use **Edit > Delete** for that.

![Clipping a ramp diagonally in the Top view: two yellow points with their coordinates, the part that stays outlined in yellow, the part that goes in red, and Both, Front and Back on the top bar](img/tools-clip.webp)

> [!TIP]
> To cut at any angle, place two points, then click a third point in another view: two points in the Top view and one in the Front or Side view make a tilted cut.
> A point placed in a 2D view sits halfway through the selection along that view's depth; in the 3D view it goes on the face under the pointer.
> A third point in line with the first two is refused, and clicking again after three points, or after two in the 2D view that placed them, starts a new line.

### Clip along a face

Double-click a face in the 3D view to put the cut exactly on that face's plane.
The cut uses the face's own plane, even an off-grid or slanted one, so a brush cut against another meets it with no gap.
Check the preview, pick the side to keep and press `Enter` as usual.

## Vertex

Vertex reshapes brushes by moving, merging, deleting or beveling their corners, edges and faces.
Press `V`.

Select brushes, then pick what to work on, as in Blender's edit mode:

| Mode | Key | Handles |
|---|---|---|
| Vertex | `1` | Yellow squares on the corners |
| Edge | `2` | Light blue squares on the edge middles |
| Face | `3` | Green dots on the face centers |

The top bar has the three mode buttons too (**Select corners**, **Select edges** and **Select faces**), then **Merge** (two arrows joining), **Bevel**, and **Delete** (a trash can).
These keys only work in the Vertex tool, where they win over the same keys elsewhere.
Switching mode keeps what the selected corners still fully cover, so two selected corners become their edge.

![A platform in the Vertex tool: corner handles in vertex mode, and edge handles in edge mode](img/tools-vertex.webp)

### Selecting

- Click a handle to select it; it turns orange.
- Shift+click or Ctrl+click adds a handle, or takes it out.
- Drag a box from empty space to select every handle inside; hold `Shift` to add or `Ctrl` to take away.
- Click away from the handles to select brushes as usual.

In a 2D view, a click takes everything stacked under the pointer, such as a corner and the corners hidden behind it.
In the 3D view it takes the nearest handle, plus the same corner of other selected brushes.
Each brush keeps its own selection, which follows the brush through edits.

### Moving

Drag any selected handle to move the whole selection.
In the 3D view the handles slide across the level plane; hold `Shift` or `Alt` to move them up and down.
For exact moves, hover a 2D view and press the arrow keys: the selected handles move one grid step at a time.

- The brush always stays a solid convex shape around its corners, with no dents.
- A move that would break the brush is refused with "Cannot move there" in the status bar.
- So is an edge or face move that would merge the edge or face into the faces around it, which would lose corners.
- Corners snap to the grid and to other corners when **Snap** and **Corners** are on.
- Edges and faces move in whole grid steps, so corners that start on the grid stay on it.

### Delete

`Delete` takes off the selected corners, or the corners of the selected edges, and the brush closes up around the corners left.
In face mode it dissolves the selected faces instead: the faces around them grow to close the gap, so dissolving a bevel's faces brings back the sharp edge.
A delete that would leave a flat or open brush is refused.
In the Vertex tool `Delete` never deletes whole brushes; use **Edit > Delete** for that.

### Merge

`M` collapses the selected corners of each brush to their center, as Blender's Merge at Center does.
In edge and face mode, it merges the corners of the selected edges or faces.
The merged corner stays selected, and each merge is one undo step.
A merge that would leave a flat or empty brush is refused.

### Bevel

`Ctrl+B` rounds off the selected edges with new faces, the way Blender's bevel does.
In vertex and face mode it bevels the edges between the selected corners.

1. Move the pointer away from the edges to set the width; it snaps to the grid while **Snap** is on.
2. Scroll to add or remove segments; one segment is a plain chamfer.
3. Click or press `Enter` to apply.
   `Esc` or a right click cancels.

The width and segments show next to the edges and in the status bar.
Afterwards, the **Adjust Last Bevel** window changes the width, segments and profile as one undo step, until you edit the brush again.
A profile of 0.5 is round, 0.25 or less is a flat chamfer, and values toward 1 make a boxier edge.

![A beveled platform edge, with the Adjust Last Bevel window showing width 16, one segment and profile 0.5](img/tools-bevel.webp)

New faces take the material of the face beside them, with the texture turned to run on around the rounded edge.

Some bevels are refused, with the reason in the status bar:

- an edge that is not between two faces of the brush;
- a width larger than a face beside the edge;
- two edges whose bevels would run into each other, such as edges that share a corner.

Edges on opposite sides of a face can bevel together, and meet in the middle when the width is half the face.

## Face

Face picks single faces of brushes, so you can texture them or turn them into terrain.
Press `Shift+F`.

Switching to Face picks every face of the selected brushes, so you can texture them all at once.
Faces hidden between touching brushes, such as the joins between ramp pieces, are left out, and the status bar says how many.

- Click a face in the 3D view to pick only that face.
  Shift+click in the Select or Move tool does the same and switches here.
- Ctrl+click or Shift+click adds a face, or takes it out.
- Click empty space to pick nothing.

Picked faces keep their real color, outlined, so you can judge their materials.
The inspector shows the face panel: see [Materials](materials.md#the-face-panel) for texturing and [Terrain](displacements.md) for displacements.

![A rock face picked with the Face tool, outlined in cyan, and the face panel in the inspector with its UV editor at the top](img/tools-face.webp)

- `Ctrl+C` copies the active face's material and alignment, and `Ctrl+V` pastes them on the picked faces (see [Copying and pasting faces](materials.md#copying-and-pasting-faces)).
- Alt+click, Alt+drag or Alt+double-click in the 3D view gives faces the active face's material and alignment; Alt+Shift turns it around corners and Ctrl+Alt carries only the material (see [Transferring a face](materials.md#transferring-a-face)).
- The [UV editor](materials.md#the-uv-editor) at the top of the face panel, and any view turned into one, shares the picks.
- Leaving the Face tool drops the picks, except for the Sculpt tool, which keeps them to limit its strokes.

## Sculpt

Sculpt shapes and paints terrain made from displacements, the faces that bend into hills and banks.
Press `D`.

Drag on a displacement in any view to raise, lower, smooth, flatten or roughen it, or to paint a second material into it and blur the blend.
The mode, radius, falloff and strength are on the top bar, and `Ctrl` swaps raise and lower.
See [Terrain](displacements.md#the-sculpt-tool).

## Path

Path draws curved paths, the guides for roads, rails and anything else that follows a curve.
Press `P`.

Click in a view to add the points of a curved path, and drag to pull out their handles.
Sweeps, spins and copies along a path are under **Tools > Curves**.
See [Paths, sweeps and spins](curves.md).

![A three point path drawn with the Path tool, curving through its middle point, with its length in the inspector](img/tools-path.webp)

## Brush operations

These commands reshape, mirror, turn or copy the selection.
They are in the **Tools** menu.

| Command | Key | Does |
|---|---|---|
| **Carve** | `Ctrl+Shift+C` | Cuts the selected brushes out of every other brush they touch |
| **Boolean > Subtract** | `Ctrl+Shift+B` | Cuts the selected brushes out of the brushes they touch and keeps the cut live; see [Boolean groups](boolean-groups.md) |
| **Hollow** | `Ctrl+H` | Turns each selected brush into walls one grid step thick around an empty inside, and selects the walls |
| **Merge Brushes** | `Ctrl+J` | Joins two or more selected brushes into one when together they make one convex shape |
| **Flip X**, **Flip Y**, **Flip Z** | `Ctrl+L`, `Ctrl+I`, `Ctrl+Alt+L` | Mirrors the selection through its center |
| **Rotate 90° Clockwise** | `Ctrl+R` | Turns the selection a quarter turn about Z through its center |
| **Rotate 90° Counter-Clockwise** | `Ctrl+Shift+R` | The same, the other way |
| **Snap Selection to Grid** | `Ctrl+B` | Moves each selected object, without reshaping it, so its lowest corner (or an entity's or prefab's origin) sits on the nearest grid point |
| **Drop to Floor** | `End` | Moves each selected object straight down until it rests on the first solid brush below it |
| **Transform** | `Ctrl+M` | Opens the Transform dialog |
| **Array** | `Ctrl+Shift+D` | Opens the Array dialog |
| **Randomize Transform** | none | Opens the Randomize Transform dialog |

A few of these have limits worth knowing:

- **Carve** leaves hidden, tagged off, locked and out of cordon brushes alone, and the selected brushes stay.
- **Merge Brushes** refuses anything that is not one convex shape (no dents or hollows), such as a curve.
- **Drop to Floor** passes through triggers, clips and other `tools/` brushes.
  An object sunk into its floor rises onto it.
  Objects with nothing below stay put, and the status bar says so.

While texture lock is on, moves, turns, flips and scales keep textures stuck to the brushes (see [Texture lock](materials.md#texture-lock)).

### The Transform dialog

The Transform dialog moves, turns or scales the selection by exact amounts.
Pick **Move**, **Rotate** or **Scale**, type X, Y and Z, and click **Apply**.

- **Move**: units along each axis.
- **Rotate**: degrees about each axis through the selection's center, applied X, then Y, then Z.
- **Scale**: factors along each axis from the selection's center.

![The Transform dialog with Move, Rotate and Scale and the X, Y and Z fields](img/tools-transform.webp)

### The Array dialog

The Array dialog makes copies of the selection in one undo step, like Hammer's Paste Special.

- **Copies**: how many, up to 256.
- **Offset**: how far each copy moves from the one before it.
- **Turn**: degrees each copy turns from the one before it, about X, then Y, then Z, through the **Pivot**.
- **Pivot at selection** and **Pivot at origin** fill in the pivot.

A row of pillars is an offset with no turn.
A ring of them is a turn about Z with the pivot at the ring's center.
A spiral stair is both.
The copies end up selected.

![The Array dialog with Copies, Offset, Turn and Pivot](img/tools-array.webp)

### The Randomize Transform dialog

Randomize Transform breaks up rows of props, rocks or crates so they stop looking copied.

- **Turn up to**: the most each object turns about Z, either way.
- **Tilt up to**: the most it tips about X and Y, either way.
- **Scale up to**: the most a brush or group grows or shrinks, as a percentage.
- **Seed**: the same seed gives the same result.

Click **Randomize** (with the dice icon) to turn and scale each selected object about its own center, all in one undo step.
While the dialog stays open, click it again for another roll, or change a value to redo the last roll with it.
Point entities such as props get new `angles` rather than moved origins, so they stay where they stand.
