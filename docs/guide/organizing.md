# Organizing

A map is a tree: the world at the top, then layers, groups, entities, brushes and prefabs.

## Selection modes

The **Groups**, **Objects** and **Solids** buttons on the top bar (a group frame, four shapes and a cube) decide what a click in a view selects.

| Mode | Key | A click selects |
|---|---|---|
| **Groups** | `Alt+1` | The outermost group around what you click, else its entity, else the brush |
| **Objects** | `Alt+2` | The entity or loose brush, ignoring groups |
| **Solids** | `Alt+3` | The one brush |

- `Ctrl+Click` adds an object to the selection, or takes it out; `Shift+Click` does too in a 2D view.
- In the 3D view, `Shift+Click` picks the face under the pointer in the [Face tool](tools.md#face) instead.
- `Ctrl+A` selects everything you can click: hidden, tagged off, out of cordon and locked objects, and everything inside a hidden or locked layer or group, are left out.
- `Ctrl+Shift+A` selects nothing.
- The modes are also under **Edit > Selection Mode**.

![Two copies of the scene tree side by side: in Groups mode a click selects the whole Ramp 3 group, in Solids mode it selects one brush inside it](img/organizing-modes.webp "Left: Groups selects the whole ramp. Right: Solids selects one brush.")

## Drill select

Click the same spot again, in the same view, to select the object behind the one you have.
Keep clicking to cycle through everything under the pointer.

## Groups

1. Select some objects.
2. Press `Ctrl+G` (**Tools > Group**, or the **Group** button above the scene tree).

The new group is named Group, sits where the first selected object was, and ends up selected.
Rename it by typing a new name into the inspector's header.

`Ctrl+U` (**Tools > Ungroup**) breaks the selected groups apart; their contents stay selected.

![The scene tree with a new group in the Course layer, and the inspector showing its name in the header](img/organizing-group.webp)

> [!WARNING]
> Ungrouping a surf ramp or another generator keeps its brushes but loses its settings.
> Ungrouping a [mirror group](mirror-groups.md) loses its mirrored half.

## Linked copies

A linked copy shows another group's contents and follows every change to it, like Blender's linked duplicates.

1. Select a group.
2. Press `Alt+D` (**Edit > Duplicate Linked**).

The copy lands one grid step away and is selected.
A selection that is not a single group is grouped first, and that group becomes the original.

- Move, turn, flip or scale each copy on its own; a flipped copy is a mirror image.
- Edit inside the original (switch to **Objects** or **Solids** to reach it) and every copy changes. Moving the original group itself moves only the original.
- **Edit here** (**Tools > Edit Linked Copy Here**) makes this copy the original, so you can edit through it.
- **Make real** (**Tools > Make Linked Copy Real**) turns a copy into an ordinary group that no longer follows.
- In the inspector's link section, the pointer icon selects the original from a copy (**Select original**), or every copy from an original (**Select copies**).
- Duplicating a copy with `Alt+D` or `Ctrl+D` adds another copy of the same original.

The scene tree marks copies **(linked copy)**.
If the original is deleted, its copies show nothing, and Check for Problems reports them.
The saved map holds each copy's contents too, so the game sees them without knowing about links.

![The scene tree with Ramp 2 (linked copy) selected, and the inspector's Linked copy section with the Select original icon, Edit here and Make real](img/organizing-linked.webp)

## Where new objects go

New brushes, entities, zones, generators and prefabs go into one place: the world, a layer or a group.
Its name shows in orange in the scene tree.

- Right click a layer, group or the world in the scene tree and choose **New objects go here**.
- Or select a layer or group and click **Put new objects here**, the pin icon in the inspector's header; click it again to send new objects back to the world.

![Right clicking the Gameplay layer in the scene tree opens a menu with New objects go here and Delete](img/organizing-new-objects.webp)

## Layers

Layers sit right under the world and hold groups, entities, brushes and prefabs.
They cannot be nested.

- **New Layer**, the **+** above the scene tree (`Ctrl+Shift+N`), adds a numbered layer and makes it where new objects go.
- Rename a layer by typing into the inspector's header.
- Tick **Leave out of exports** to keep a layer out of Godot and the compiles, for reference pieces or work in progress.
- Each import lands in a layer of its own.

![A new layer, Layer 4, selected in the scene tree, with its name in the inspector's header and the Leave out of exports box below](img/organizing-layer.webp)

## The scene tree

The **Scene** tab (`Ctrl+1`) shows the whole map.
Each row has a fold arrow, an icon (entities in their class color), a name and an eye.
Entities with a `targetname` show as `name (class)`.

- Click a row to select it; `Ctrl+Click` adds it or takes it out.
- Double click a row to select it and focus the views on it.
- Right click a row for **New objects go here** and **Delete**.
- Click the eye to hide or show a node and everything in it.

Selecting something in a view unfolds the tree down to it and scrolls to it.

### Search

Type in the **Search** box above the tree to find a brush, entity, group or layer.
`Ctrl+Shift+F` (**View > Search Scene**) jumps to the box from anywhere and selects its text, so typing starts a new search.

- It matches names, entity classes and targetnames, and brush materials, ignoring case.
- The tree shows only the matches and the layers, groups and entities they are in, unfolded.
- The matching text is orange, and a brush found by its material shows that material after its name.
- `Enter` selects the first match and focuses the views on it; press it again for the next match.
- The box keeps the keyboard after `Enter`, so press `Down` to hand the keys back to the editor and keep the filter, for example to delete what you found.
- `Escape` clears the box, hands the keys back and brings the whole tree back, scrolled to the selection.
- Rows cannot be folded while the box has text.

### Drag to reorder or reparent

Drag a row and drop it:

- on the middle of a row to move it inside that layer, group or brush entity;
- near the top or bottom edge of a row to put it before or after that row.

A line or outline shows where it will land.
Dragging a selected row moves the whole selection, and each move is one undo step.
A brush entity only takes brushes, layers stay under the world, and a generator group (such as a surf ramp) refuses drops because it rebuilds its own contents.

![Dragging Ramp 1 onto the Hazards layer: an orange outline around Hazards shows it will land inside](img/organizing-drag.webp)

## Hide, isolate and show all

| Command | Key | Does |
|---|---|---|
| **View > Hide Selected** | `H` | Hides the selection |
| **View > Isolate Selected** | `I` | Hides every brush, entity and prefab outside the selection |
| **View > Show All** | `U` | Shows everything hidden with the eye, Hide or Isolate |

- Hidden objects cannot be clicked, and Select All skips them.
- Hiding can be undone and is saved with the map, but it only affects the editor: hidden objects still go to Godot.
- Show All leaves tags alone, since each tag has its own switch.

![Ramp 3 isolated: the 3D view shows only that ramp and the scene tree shows closed eyes on the hidden objects](img/organizing-isolate.webp)

> [!TIP]
> To keep something out of Godot and the compiles, put it on a layer and tick **Leave out of exports**.

## Lock and unlock

| Command | Key | Does |
|---|---|---|
| **View > Lock Selected** | `Ctrl+Alt+K` | Locks the selection so it cannot be clicked, sculpted or carved |
| **View > Unlock All** | `Ctrl+Alt+Shift+K` | Unlocks everything |

Locking a layer, group or brush entity locks everything inside it too.
Locking can be undone and is saved with the map.
Lock a reference layer, such as an imported map you are zoning, so clicks only reach what you are building.
Locked objects still catch what you place: blocks drawn in the 3D view, new entities and clip points land on them, and Alt+click still samples their materials.

## Tags

Tags are labels on objects, like Hammer's visgroups.
An object can carry several.

1. Select the objects.
2. Open the **Tags** tab (`Ctrl+2`).
3. Type a name into **New tag** and click **Tag selection**. With no name you get Tag 1, Tag 2 and so on.

Each tag's row has:

- an eye that shows or hides everything with the tag;
- its name, which you can edit;
- how many objects carry it;
- **Select** (the pointer), which selects them;
- **Add the selection** and **Take the selection out** (a plus and a minus in a square), which add the selection to the tag or take it out;
- a trash can that deletes the tag (the objects stay).

![The Tags tab with Tag 1 on one object, and the Zones auto tags Timing and Hazards below it](img/organizing-tags.webp)

### Auto tags

When the SDK's entity definitions (its FGD) list `@AutoVisGroup` sets, they show under **Auto tags**, grouped under their parent name, like Hammer's auto visgroups.
The SurfsUp SDK has **Zones** with **Timing** and **Hazards**.
Each set gathers the entities of the classes it lists; hover its name to see them.

- The eye shows or hides every entity in the set.
- **Select** selects them.

Auto tags are not saved in the map: membership follows the entity classes.
Sets with no entities in the map are left out.

## Cordons

A cordon is a box that hides everything outside it, so you can work on one part of a big map.
Cordons sit below the tags in the **Tags** tab.

1. Select the part you want to work on.
2. Click **New cordon around selection**.

The cordon turns on and shows as a blue box; only what touches it stays visible.

- The checkbox turns a cordon on or off.
- **Fit** (arrows pointing in) fits a cordon to the current selection.
- Cordons can be renamed and deleted, and every change can be undone.

![A cordon around Ramp 3: the 3D view shows only the ramp inside a blue box, with Cordon 1 listed in the Tags tab](img/organizing-cordon.webp)

> [!WARNING]
> Cordons are saved with the map, and the Godot addon leaves out everything outside an active cordon.
> Turn cordons off before you build the map for the game.
