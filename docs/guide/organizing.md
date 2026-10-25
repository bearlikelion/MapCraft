# Organizing

A map is a tree: the world at the top, then layers, groups, entities, brushes and prefab instances.
This page covers selecting within that tree and keeping a large map tidy.

## Selection modes

The selection mode decides what a click in a view selects.
Switch it with the **Groups**, **Objects** and **Solids** buttons on the top bar, the same items in the Edit menu, or the keys below.

| Mode | Key | A click selects |
|---|---|---|
| Groups | `Alt+1` | The outermost group around the brush, else its entity, else the brush |
| Objects | `Alt+2` | The brush's entity, else the brush; groups are ignored |
| Solids | `Alt+3` | The brush itself |

- Ctrl+click or Shift+click adds an object to the selection, or takes it out.
- `Ctrl+A` selects everything at the top of the map and inside layers, skipping hidden objects.
- `Ctrl+Shift+A` selects nothing.

## Drill select

Clicking the same spot again, in the same view, while one object is selected selects the next object behind it.
Keep clicking to cycle through everything under the pointer.

## Groups

- **Tools > Group** (`Ctrl+G`, or **Group** above the scene tree) puts the selection in a new group named Group, inside the first selected object's parent, and selects the group.
- **Tools > Ungroup** (`Ctrl+U`) dissolves the selected groups: their contents move up into the group's place and stay selected.

Rename a group in the inspector.
Ungrouping a generator group keeps its brushes but drops its parameters, and ungrouping a [mirror group](mirror-groups.md) drops its mirrored half.

## Where new objects go

New brushes, entities, zones, generators and inserted prefabs go into one container: the world, a layer or a group.
Its name shows in the accent color in the scene tree.

- Right-click a layer, group or the world in the scene tree and choose **New objects go here**.
- Or select a layer or group and click **Put new objects here** in the inspector.

## Layers

Layers sit directly under the world and hold groups, entities, brushes and prefab instances; they cannot be nested.

- **New Layer** above the scene tree (`Ctrl+Shift+N`) adds a layer named Layer 1, Layer 2 and so on, and makes it where new objects go.
- Rename a layer in the inspector.
- Each import goes into a layer of its own.
- Opening a map unfolds its layers in the scene tree.

## The scene tree

The Scene tab (`Ctrl+1`) shows the whole map.
Each row has an arrow to fold it, an icon for its kind (entities in their class color), its name and an eye.
Entities with a `targetname` show as `name (class)`.

- Click a row to select it, Ctrl+click to add it to the selection or take it out.
- Double-click a row to select it and focus the views on it.
- Right-click a row for **New objects go here** and **Delete**.
- Click the eye to hide or show a node and everything in it.

Selecting something in a view unfolds its parents in the tree and scrolls to it.

### Drag to reorder or reparent

Drag a row and drop it:

- on the middle of a row to move it inside that layer, group or brush entity;
- near the top or bottom edge of a row to put it before or after that row.

A line or outline shows where it will land.
Dragging a selected row moves the whole selection.
A brush entity only takes brushes, layers stay under the world, and a generator group refuses drops because it rebuilds its own contents.
Each move is one undo step.

## Hide, isolate and show all

| Command | Key | Effect |
|---|---|---|
| **View > Hide Selected** | `H` | Hides the selection |
| **View > Isolate Selected** | `I` | Hides every brush, entity and prefab instance outside the selection |
| **View > Show All** | `U` | Shows everything hidden by the eye, Hide or Isolate |

Hidden objects cannot be clicked and Select All skips them.
Hiding is undoable and is saved with the map, but it only affects the editor: hidden objects still go to Godot.
Show All leaves tags alone, since each tag has its own switch.
To keep something out of Godot and the compiles, put it on a layer and tick **Leave out of exports** in the layer's inspector.

## Lock and unlock

| Command | Key | Effect |
|---|---|---|
| **View > Lock Selected** | `Ctrl+Alt+K` | Locks the selection so it cannot be clicked, sculpted or carved |
| **View > Unlock All** | `Ctrl+Alt+Shift+K` | Unlocks everything |

Locking is undoable and is saved with the map.
Lock a reference layer, such as an imported map you are zoning, so clicks only reach what you are building.

## Tags

Tags are labels you put on any objects, like Hammer's visgroups; an object can carry several.
Open the Tags tab with `Ctrl+2`.

1. Select the objects.
2. Type a name into **New tag** and click **Tag selection**; an empty name gives Tag 1, Tag 2 and so on.

Each tag's row has:

- a checkbox that shows or hides everything with the tag;
- its name, which you can edit;
- how many objects carry it;
- **Select**, which selects them;
- **+** and **−**, which add the selection to the tag or take it out;
- a button that deletes the tag (the objects stay).

## Cordons

Cordons sit below the tags in the Tags tab.
While a cordon is on, only what touches it is shown, so you can work on one part of a large map.

- **New cordon around selection** adds a cordon fitted to the selection and turns it on.
- The checkbox turns a cordon on or off.
- **Fit** fits a cordon to the current selection.
- Cordons can be renamed and deleted, and every change is undoable.

Active cordons are drawn as blue boxes.
They are saved with the map, and the Godot extension leaves out everything outside an active cordon, so turn cordons off before you build the map for the game.
