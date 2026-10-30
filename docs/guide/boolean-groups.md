# Boolean groups

A boolean group cuts holes and adds shapes the way Godot's CSG nodes do, and keeps them live.
Put a cylinder through a wall, mark it **Subtract**, and the wall gets a round hole.
Move or resize the cylinder later and the hole follows; the wall and the cylinder stay ordinary brushes you can keep editing.
Games only ever get the result, cut into plain convex brushes.

Carve does the same cut once and for all: it replaces the wall with the pieces and forgets the cylinder.
Use a boolean group when you may still move the hole, and Carve when you are sure.

## Make a hole

1. Build the wall, then a brush where the hole goes, such as a Block tool cylinder pushed through the wall.
2. Select the cylinder.
3. Press `Ctrl+Shift+B` (**Tools > Boolean > Subtract**).

The cylinder and every brush it touches go into a new group named Boolean.
The cylinder becomes a **Subtract** brush and the brushes it touched become **Add** brushes.
The cylinder stays selected, so you can move it straight away.

- The result is every Add brush less every Subtract brush; the order of the brushes in the group does not matter.
- A Subtract brush that touches a brush already in a boolean group joins that group instead, so a second hole goes in the same wall.
- With a boolean group selected too, the selected brushes join that group, whatever they touch.
- Brushes inside an entity, such as a `func_detail`, are left out.

## Add and Subtract

Each brush in a boolean group adds or subtracts.
Select one brush of the group (switch to **Solids** with `Alt+3`, or pick it in the Scene tree) and set **Boolean** in the inspector to **Add** or **Subtract**.

| Command | Key | Does |
|---|---|---|
| **Subtract** | `Ctrl+Shift+B` | Makes the selected brushes Subtract brushes, putting them in a boolean group as above |
| **Add** | `Ctrl+Alt+B` | Makes the selected brushes Add brushes; brushes not yet in a boolean group join the selected one, else the one holding a brush they touch, else a new one around them |
| **Release from Boolean** | none | Moves the selected brushes out of their boolean group, just after it, as ordinary brushes |
| **Apply Boolean** | none | Turns the selected boolean group into a plain group holding the result as ordinary brushes |

All four are under **Tools > Boolean** and in the right click menu, and each is one undo step.

## In the views

- The result draws as ordinary brushes, with the materials of the Add brushes.
- The faces a cut opens take the material of the Subtract brush's faces, so texture the cylinder to texture the inside of the hole.
- Subtract brushes show as orange outlines in every view.
  Add brushes show an outline only while selected, since the result shows their shape.
- Clicking a piece of the result reaches the Add brush it came from, and clicking inside a hole reaches the Subtract brush that made it.
  With the **Groups** selection mode either click selects the group.
- With the Face tool, clicking a face of the result picks the face it came from: a face of an Add brush, or for the inside of a hole the face of the Subtract brush.
- Moving, turning or duplicating the group carries everything in it, as with any group.

## Apply

When the shape is final, **Apply Boolean** (or **Apply** in the group's inspector) replaces the group's brushes with the pieces of the result.
The views do not change.
It is like having used Carve.

## In the game

The saved map holds the result as ordinary brushes in the group, so Godot, exports such as `.vmf` and `.map`, glTF and compiles get only the pieces, never the cutting brushes.
MapCraft keeps the group's own brushes beside them in the file and rebuilds the result each time it opens the map.

For importer authors: [the .mcmap format](../mcmap-format.md#boolean-groups).

## Limits

- A cut makes several pieces per Add brush, more for a round Subtract brush with many sides; keep cylinders to the sides you need.
- One boolean group stops cutting past 4096 pieces.
- Only Add and Subtract exist; there is no Intersect.
- A group inside a boolean group, and anything that is not a brush, is left as it is and neither adds nor cuts.
- A hidden Subtract brush still cuts.
- The views rebuild every boolean group's result after each edit, so a map with very many of them can feel slower; apply the ones you are done with.
