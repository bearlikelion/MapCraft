# Mirror groups

A mirror group keeps a map symmetric: you build one half and MapCraft draws the other half, following every edit.

## Make a mirror group

1. Build one half of the map, up to the middle line.
2. Select it.
3. Choose **Tools > Make Mirror Group** (`Ctrl+Shift+M`).

- The selection goes into a new group named Mirror, mirrored left and right across a plane at a fixed X.
- If the selection is a single plain group, that group becomes the mirror group instead.
- The plane goes through the point of the selection nearest the map's origin: at 0 when the selection spans the origin, else at the selection's edge nearest it.
- The group's texture lock starts as the top bar's **Texture lock** setting.

Selecting a mirror group and making a mirror group again wraps it in a new one, so mirrors can nest: build a quarter, mirror it, mirror the result and set the outer group's axis to Y.

## How the mirror follows

- The mirrored half is drawn live, and every edit to the originals shows on it at once.
- Clicking the mirrored half selects the original it copies (or its group, depending on the selection mode).
- Moving the mirror group moves its plane with it.
- While the mirror group, or anything in it, is selected, the plane is outlined in the views.

Select the group to change the mirror in the inspector:

| Setting | Effect |
|---|---|
| **Axis** | X mirrors left and right, Y front and back, Z up and down; changing it moves the plane to that axis's default spot |
| **Plane at** | The plane's position along the axis |
| **Texture lock** | Copies carry their textures across the plane; off, they keep the originals' texture alignment |
| **Make real** | See below |

How the copy is made:

- A brush touching the plane meets its copy with no gap.
- A brush crossing the plane adds only the part its reflection does not already fill, and a brush centered on the plane adds nothing.
- Point entities get the mirrored position and turn to face the way the reflection does; a point entity or prefab instance lying on the plane is not doubled.
- Prefab instances are copied as instances, turned rather than flipped.

Check for Problems checks the mirrored half too and reports its problems on the originals.
Ungrouping a mirror group drops the mirrored half; use Make Mirror Real first to keep it.

## Make Mirror Real

**Tools > Make Mirror Real**, or **Make real** in the inspector, turns the mirrored half into ordinary objects inside the group and stops mirroring.
It is enabled when a mirror group is selected, changes nothing in the views, and is one undo step.

## What the file holds

- The `.mcmap` file stores the group with its mirror (axis, plane position and texture lock) and the half you built as its children.
- The mirrored half is written too, as extra children flagged `derived`.
  Importers such as the Godot extension get the whole symmetric map without knowing about mirroring.
- MapCraft drops the derived children when it opens the file and builds them again from the originals.
- A prefab instance whose file holds a mirror group comes in with that mirror made real.

The [file format reference](../mcmap-format.md#mirror-groups) has the details for importer authors.
