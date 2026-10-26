# Paths, sweeps and spins

Brushes are convex, so curved shapes such as winding roads, rails, trims, pipes, arches and round towers are built from many convex pieces.
MapCraft builds those pieces for you from a Bezier path or a turn about an axis, and rebuilds them whenever you edit what they came from.

## Paths

A path is a curve through points, each with two handles, as in Blender.
It builds nothing by itself: sweeps and arrays follow it.

Draw one with the **Path** tool (`P`, the last icon in the tool palette):

- Click in any view to add a point; in a 2D view it sits at the depth of the last brush you drew, in the 3D view on the ground or the face under the pointer.
- Press and drag to add a point and pull out its handles, which makes the curve run smoothly through it.
- Click the first point (green) to close the path into a loop.
- `Enter` finishes the path, so the next click starts a new one; `Esc` returns to the Select tool.
- Each point is its own undo step, so `Ctrl+Z` takes back the last one.

Edit a selected path with the Select, Move or Path tool:

- Drag a point (a white square) to move it with its handles.
- Drag a handle (a dot) to bend the curve; the handle across the point turns with it.
  `Alt` drags a handle on its own, for a sharp bend.
- `Alt` drag a point to pull fresh handles out of it, turning a corner into a smooth point.
- Drags snap to the grid and to brush corners like every other drag; in the 3D view they slide across the point's height, and `Shift` moves up and down.
- Click a path's line to select it, and drag the line in a 2D view to move the whole path.

The inspector shows the path's length and a **Closed** box.
Selected paths are drawn red with their handles; others are drawn pink in every view.

## Sweep along a path

A sweep moves faces along a path and fills the space they pass through with convex brushes.

1. Build the cross-section as brushes at the start of the path, for example a road deck with a curb on each side.
   Each face you sweep must be convex; split a concave cross-section into several brushes.
2. With the **Face** tool (`Shift+F`), pick the faces that look along the path.
3. Choose **Tools > Curves > Sweep Along Path** (`Ctrl+Alt+S`).

The sweep follows the selected path, or the newest one when none is selected.
The result is a group named Sweep, selected and shown in the inspector, and the brushes you picked faces of stay as they are.

| Setting | Effect |
|---|---|
| **Segments** | Brushes per curved span; a straight span always takes one |
| **Twist** | Degrees the cross-section turns by the end of the path; on a closed path it rounds to whole turns |
| **End scale** | Size of the cross-section at the end of an open path, for tapering |

- The cross-section stays upright as the path turns, climbs and falls; a path that goes straight up somewhere, such as a loop, carries it along with as little twist as possible and never flips it.
- Neighboring pieces share their faces exactly, so there are no gaps; the faces between pieces are marked hidden in the file.
- Textures follow the brushes you swept: each side carries on the texture of the face beside that edge on the profile brush, unrolled along the path, so a trim keeps running around every bend.
  Align the texture on the profile brush with the Face tool and the whole sweep follows.
- The ends keep the swept face's own texture.

## Spin

A spin turns faces about an axis, like a lathe, for arches, rings, round towers and spiral pieces.

1. Pick the faces with the Face tool.
2. Hover a 2D view where the axis should go and press `Ctrl+Alt+N`, or choose **Tools > Curves > Spin**.

The axis runs through the point under the pointer (else the selection's center, else the origin): upright for a standing face, and level for a face lying flat, so a floor tile beside the axis swings up into an arch.

| Setting | Effect |
|---|---|
| **Angle** | Degrees to turn; 360 closes the ring |
| **Steps** | Brushes along the turn |
| **Axis** | X, Y or Z |
| **Pivot** | A point the axis runs through |

The axis is drawn through the views while the spin is selected.

## Array along a path

1. Select what to copy, such as a lamp post, and the path; without a selected path the newest one is used.
2. Choose **Tools > Curves > Array Along Path** (`Ctrl+Alt+D`).

Copies are placed one step apart along the path, the last at its end, each turned to the path's direction as the original sits at its start.

| Setting | Effect |
|---|---|
| **Count** | How many copies, spread evenly |
| **Spacing** | Units between copies instead of a count; 0 uses Count |
| **Turn** | Follow turns copies with the path; Yaw only turns them about Z so they stay upright |

## Live results

- Sweeps, spins and arrays are curve groups: changing a setting, editing the path, or editing a picked face or copied object rebuilds the group in the same undo step.
- Moving a path carries its sweeps and arrays along; moving the profile brushes or copied objects on their own leaves the result on the path.
- Deleting a path, a profile brush or a copied object leaves the result as it is, no longer following.
- Brushes inside a curve group are rebuilt, so edits made to them directly are lost on the next rebuild; nothing can be dragged into the group.
- **Make real** in the inspector, or **Tools > Curves > Make Real**, keeps the brushes as a plain group that no longer follows.
- Copying or duplicating a curve group gives a plain group.

Large settings are refused rather than slowing the editor: a sweep or spin makes at most 4096 brushes, and an array at most 256 copies.

## In the file

Paths and curve groups are stored in the `.mcmap` file.
A curve group's brushes are written as ordinary children, so importers such as the Godot extension use them without knowing about curves, and skip paths.
The [file format reference](../mcmap-format.md#paths-and-curve-groups) has the details.
