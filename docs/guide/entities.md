# Entities and zones

Entities are the things a game reads besides geometry: spawns, zones, teleport destinations, lights.
The SDK's FGD files define which classes exist and what keys each one has.

## Point and brush entities

- A **point entity** sits at one spot, its `origin`, and is drawn as a box with its class's size and color, or as its model when it has one (see [Models](#models)).
- A **brush entity** owns brushes; zones and triggers are brush entities.
  Its brushes with `tools/` materials, such as the `tools/trigger` that zones use, are drawn see-through in the class's color.

Check for Problems flags a brush entity with no brushes and a point entity that holds brushes.

## The Entities tab

Open it with `Ctrl+4`.
It lists every class from the SDK's FGDs: ■ marks brush classes and ○ point classes.
Type in the search box to filter, and hover a class to read its description.

- Clicking a **point class** makes it the Entity tool's class and switches to the Entity tool.
- Clicking a **brush class** makes it the class that **Tie to Entity** and **Zone from Selection** use; it never changes the selection by itself.

The tab shows nothing when the SDK has no FGD classes.

## The Entity tool

Press `N`, then click in a view.

- In the 3D view the entity lands on the face under the pointer, snapped to the grid; clicking empty sky places nothing.
- In a 2D view it lands on the grid at the depth of the last brush you drew.

The new entity gets its class's default key values, goes where new objects go (see [Organizing](organizing.md#where-new-objects-go)) and is selected.
The tool starts out placing `info_player_start`.

## Models

Point entities can show glTF 2.0 models (`.glb` and `.gltf`), the format Godot projects use.
An entity shows a model when its class names one in the FGD, as `studio("props/crate.glb")` or TrenchBroom's `model({ "path": "props/crate.glb" })`, or else when its `model` key names a `.glb` or `.gltf` file.
Paths are relative to the SDK's model folders (`[models] roots` in its `sdk.toml`, else its texture folders); a leading `res://` is ignored, so Godot paths work as they are.

- The model is placed by the entity's `origin`, its `angles` (pitch, yaw, roll) or `angle` (yaw), and a `scale` or `modelscale` key.
- glTF is Y up in meters; MapCraft turns it Z up and scales it by the SDK's units per meter, the same way the Godot importer places it.
- Models draw in the Textured, Flat and Lit modes with their base color texture and color, and as wireframes in the 2D views and the Wireframe mode.
- Clicking anywhere in a model's box selects its entity; a selected model is tinted and boxed in red.
- Models load in the background the first time a map uses them.
  One that is missing, unreadable or larger than about a million triangles shows as the entity's box, with a line in the Output panel saying why.

Only static meshes are drawn: positions, normals, texture coordinates and the base color.
Skinned models show in their bind pose, and animations, morph targets and other material maps are ignored.
Source and Quake `.mdl` models are not drawn; their entities stay boxes.

### The Models tab

Open it with `Ctrl+6`.
It lists every `.glb` and `.gltf` file in the SDK's model folders, skipping hidden folders such as `.godot`; type in the search box to filter.

Click a model to load it: the tab shows its triangle count and size in units.
When the SDK names a prop class (`[models] class` in its `sdk.toml`), clicking a model also switches to the Entity tool with that class, and each click in a view places a prop with its model key set to the model's path.
Without a prop class the tab can still preview models but cannot place them.

The SurfsUp SDK's prop class is `prop_static`, with `model`, `angles` and `scale` keys; its model paths are relative to the SurfsUp project folder.

## The inspector

Select one entity to edit it in the inspector on the right.
With nothing selected, the inspector shows the world's (`worldspawn`) key values.

The class dropdown at the top lists the FGD classes of the same kind, point or brush, so you can turn a `trigger_teleport` into a `teleport_zone` without redrawing it.
Its key values stay; keys the new class does not know show in italics.

A brush entity also shows its brushes' material and alignment controls below the key values.

### Keyvalues

With **Smart edit** on (the toggle at the top right, `Ctrl+Alt+E`):

- Each key the FGD defines has a row with its friendly name.
  A dimmed name means the key is not set and the field shows the class's default.
- Keys with choices get a dropdown, flags a checkbox per flag, booleans a checkbox, and `color255` and `color1` keys a color picker next to the text.
- Everything else is a text field.
- Keys the FGD does not define show in italics with a delete button.
- Type a name into **new key** and click **Add** to add a key.

Turn Smart edit off to see the raw key value pairs, where keys can be renamed too.

Below the keys, a help box shows the hovered key's name and description, or the class's description when nothing is hovered.
Typing into one field is a single undo step.

### Outputs

The tab title shows how many outputs the entity has.
Each output has:

- the output name, a dropdown when the FGD lists outputs for the class;
- the target entity's name, the input to fire on it and a parameter;
- a delay in seconds;
- **once**, to fire only the first time instead of every time.

Click **Add output** to add one, and the delete button to remove it.

### Inputs

This tab lists the outputs of other entities whose target is this entity's `targetname`.
Click one to select the entity that sends it.
An entity with no `targetname` has no inputs.

## Tie to Entity and Move to World

**Tools > Tie to Entity** (`Ctrl+T`, or **Tie to entity** in the inspector) moves the selected world brushes into a new brush entity.

- The class is the brush class last clicked in the Entities tab, `func_detail` until you click one.
- Brushes that already belong to an entity are left alone.
- The entity goes where the first brush was, and ends up selected.

**Tools > Move to World** (`Ctrl+Shift+W`) does the reverse: the selected brush entities are removed and their brushes stay where the entity was, selected.

### Brush type

The brush inspector starts with a **Type** row for switching selected brushes in one click.
**World** makes them plain world brushes; **detail**, **illusionary** and **wall** tie them to `func_detail`, `func_illusionary` or `func_wall`, whichever the SDK's FGD defines.
The selected brushes leave whatever entity they were in, empty entities are removed, and the switch is one undo step.
**Group** groups the selection, as **Tools > Group** does.
In the SurfsUp SDK, `func_detail` merges into the world and `func_illusionary` is drawn but has no collision.

## Zone from Selection

**Tools > Zone from Selection** builds a zone fitted to a platform.

1. Click a zone class in the Entities tab, such as `zone_start`.
2. Select the platform's brushes.
3. Choose **Tools > Zone from Selection**.

The zone is a box covering the selected brushes' footprint, from their highest point up 256 units, with the `tools/trigger` material.
Its class is the brush class last clicked in the Entities tab; while that is still `func_detail`, MapCraft refuses and asks you to pick a trigger class first.
The zone goes where new objects go and ends up selected.

The SurfsUp SDK's zone classes:

| Class | What it does |
|---|---|
| `zone_start` | Starts the run timer when the player leaves it |
| `zone_finish` | Stops the run timer |
| `zone_split` | Records a split time |
| `zone_checkpoint` | Saves the player's progress |
| `kill_box` | Sends the player back to their respawn point |
| `teleport_zone` | Teleports the player to a `teleport_destination` |
| `zone_point_tp` | Teleports the player to a point |
| `zone_booster` | Pushes the player |
| `zone_respawn` | Changes where the player respawns |
| `bhop_zone` | Marks a bunny hop area |

[Surf mapping](../surf-mapping.md#zones) has advice on sizing and placing zones.

## Link lines

When an entity is selected, lines run from it to every entity it names: through keys the FGD marks as a target (such as a teleport zone's **Destination**) and through its outputs.
For a class the FGD does not define, the keys `target`, `parentname`, `respawn` and `landmark` count.

## Find Entities

**View > Find Entities** (`Ctrl+F`) opens the Find tab below the views.

- Type a class name or any value to list the entities whose class name or values contain it.
- Type `key=value` to list the entities whose `key` contains `value`.
- Case does not matter.
- Click a result to select it and focus the views on it; **Select all** selects every result.

To replace text, type the new text into **replace with** and click **Replace in values**.
It replaces the found text in key values and in output targets and parameters, in the whole map or, with **In selection**, only in the selected objects.
Like the search, it ignores case; it does not work with a `key=value` search, and it is one undo step.

## Entity problems

**Tools > Check for Problems** (`Alt+P`) reports these entity problems:

- A class the SDK's FGDs do not define.
- A brush entity with no brushes, which **Fix** removes.
- A point entity that holds brushes.
- A target key or output naming an entity that does not exist; **Fix** clears the key or removes the output.
