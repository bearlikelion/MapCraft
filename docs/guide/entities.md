# Entities and zones

Entities are everything a game reads besides walls and floors: spawns, zones, teleport destinations, lights and props.
The SDK decides which entity classes exist and what settings each one has.

## Point and brush entities

There are two kinds:

- A **point entity** sits at one spot. It shows as a box in its class's color, or as its model when it has one (see [Models](#models)).
- A **brush entity** is made of brushes. Zones and triggers are brush entities, drawn see-through in the class's color.

In this sample course, the start zone is green, the split zones are blue and the finish zone is red.

![A surf course in the 3D view with a green start zone, blue split zones and a red finish zone](img/entities-zones.webp)

## The Entities tab

Open it with `Ctrl+4`.
It lists every class the SDK defines: ■ marks brush classes and ○ point classes.
Type in the search box to filter, and hover a class to read what it does.

- Click a **point class** to place it with the Entity tool.
- Click a **brush class** to choose what **Tie to Entity** and **Zone from Selection** make. This never changes the selection.

The tab is empty when the SDK defines no classes.

![The Entities tab listing SurfsUp's brush classes (zone_start, zone_finish, kill_box and more) and point classes (info_player_start, teleport_destination, prop_static and more)](img/entities-tab.webp)

## The Entity tool

1. Press `N`, or click a point class in the Entities tab.
2. Click in a view to place one.

- In the 3D view it lands on the face under the pointer, snapped to the grid. Clicking empty sky places nothing.
- In a 2D view it lands on the grid, at the depth of the last brush you drew.

The new entity gets its class's default settings, goes where new objects go (see [Organizing](organizing.md#where-new-objects-go)) and is selected.
The tool starts out placing `info_player_start`, the player spawn.

![A newly placed info_player_start selected in the 3D view, with its origin in the inspector](img/entities-place.webp#wide)

## Models

Point entities can show 3D models in `.glb` or `.gltf` files, the formats Godot projects use.
An entity shows a model when its class names one, or when its `model` key names a `.glb` or `.gltf` file.

- Models move, turn and scale with the entity's `origin`, `angles` (or `angle`) and `scale` keys.
- They appear at the same size and facing as they will in Godot.
- They draw with their colors and textures in the 3D view, and as wireframes in the 2D views.
- Click anywhere in a model's box to select its entity.

Models load in the background the first time a map uses them.
A model that is missing, broken or huge (over about a million triangles) shows as a plain box, and the Output panel says why.

> [!NOTE]
> Only still meshes are drawn: no animation. Source and Quake `.mdl` models are not drawn; their entities stay boxes.
>
> For SDK authors: a class names its model in the FGD with `studio("props/crate.glb")` or TrenchBroom's `model({ "path": "props/crate.glb" })`. Paths are relative to the SDK's model folders, and a leading `res://` is ignored. See [SDK manifests](../sdk-format.md).

### The Models tab

Open it with `Ctrl+6`.
It lists every model in the SDK's model folders; type in the search box to filter.

1. Click a model.
   The tab shows its triangle count and size.
2. Click in a view to place it as a prop.

In the SurfsUp SDK the prop class is `prop_static`, with **Model**, **Pitch Yaw Roll** and **Scale** settings.
An SDK without a prop class can preview models but not place them.

![The Models tab with Archer.glb picked, the archer model standing on the lava floor in the 3D view and drawn as wireframes in the 2D views, and its prop_static settings in the inspector](img/entities-models.webp#wide)

## The inspector

Select one entity to edit it in the inspector on the right.
With nothing selected, the inspector shows the whole map's settings (`worldspawn`).

1. The class dropdown turns the entity into another class of the same kind, for example a `trigger_teleport` into a `teleport_zone`, without redrawing it. Its settings stay; ones the new class does not know show in italics.
2. **Smart edit** (`Ctrl+Alt+E`) switches between friendly fields and the raw key and value pairs.
3. The help box describes the setting under the pointer, or the class when nothing is hovered.

A brush entity also shows its brushes' material buttons below its settings.

![The inspector for a teleport_zone with numbered markers on the class dropdown, the Smart edit toggle and the help box](img/entities-inspector.webp)

### Keyvalues

With **Smart edit** on:

- Each setting the class defines gets a row with a friendly name. A dimmed name means it is not set and shows the default.
- Choices get a dropdown, flags and yes or no settings a checkbox, and colors a color picker.
- Everything else is a text field.
- Settings the class does not define show in italics with a delete button.
- Type a name into **new key** and click **Add** to add one.

Turn Smart edit off to see and rename the raw keys.
Typing into one field is a single undo step.

### Outputs

Outputs make one entity tell another to do something.
The tab title shows how many the entity has.

1. Click **Add output**.
2. Fill in the output name, the target entity's name, the input to fire on it and a parameter.
3. Set a **delay** in seconds, and tick **once** to fire only the first time.

The output name is a dropdown when the SDK lists outputs for the class.
The delete button removes an output.

![The Outputs tab with one new output row: output, target, input, parameter, delay and once](img/entities-outputs.webp)

### Inputs

This tab lists the outputs of other entities that target this entity's name (`targetname`).
Click one to select the entity that sends it.
An entity with no name has no inputs.

## Tie to Entity and Move to World

**Tools > Tie to Entity** (`Ctrl+T`, or **Tie to entity** in the inspector) moves the selected world brushes into a new brush entity.

- The class is the brush class you last clicked in the Entities tab (`func_detail` until you click one).
- Brushes already in an entity are left alone.
- The new entity ends up selected.

**Tools > Move to World** (`Ctrl+Shift+W`) does the reverse: the entity goes away and its brushes stay, selected.

### Brush type

The inspector's **Type** row switches selected brushes in one click:

- **World** makes them plain world brushes.
- **detail**, **illusionary** and **wall** tie them to `func_detail`, `func_illusionary` or `func_wall`, if the SDK has that class.
- **Group** groups them, like **Tools > Group**.

Empty entities left behind are removed, and the switch is one undo step.
In the SurfsUp SDK, `func_detail` merges into the world and `func_illusionary` is drawn but cannot be touched.

![The Type row in the inspector: World, detail, illusionary and Group](img/entities-brush-type.webp)

## Zone from Selection

**Tools > Zone from Selection** builds a zone that fits a platform.

1. Click a zone class in the Entities tab, such as `zone_start`.
2. Select the platform's brushes.
3. Choose **Tools > Zone from Selection**.

The zone covers the platform's footprint and stands 256 units tall above its highest point, with the `tools/trigger` material.
It goes where new objects go and ends up selected.
While the brush class is still `func_detail`, MapCraft asks you to pick a trigger class first.

![Zone from Selection: zone_start clicked in the Entities tab, and the new zone standing on the start platform in the 3D view](img/entities-zone-from-selection.webp)

### SurfsUp zones

| Class | What it does in the game |
|---|---|
| `zone_start` | Starts the run timer when the player leaves it |
| `zone_finish` | Stops the run timer |
| `zone_split` | Records a split time |
| `zone_checkpoint` | A jump area: players can bunny hop inside without the floor resetting their run |
| `kill_box` | Sends the player back to their respawn point |
| `teleport_zone` | Teleports the player to a `teleport_destination` |
| `zone_point_tp` | Teleports the player to a point |
| `zone_booster` | Pushes the player |
| `zone_respawn` | Changes where the player respawns, and counts as a checkpoint on maps with no splits |
| `bhop_zone` | The same jump area as `zone_checkpoint` |

Each zone becomes the matching SurfsUp zone in the game, and every setting on it arrives on that zone with the same name and type.
`starting_platform` and `finish_line` place SurfsUp's own starting and landing platforms.
[Surf ramps and bhop](surf.md) has tips on sizing and placing zones.

## Link lines

Select an entity and lines run from it to every entity it names: through settings that point at a target (such as a teleport zone's **Destination**) and through its outputs.
For a class the SDK does not define, the keys `target`, `parentname`, `respawn` and `landmark` count.

![A selected teleport_zone in the Top view with a line running to its teleport destination](img/entities-link-lines.webp)

## Find Entities

**View > Find Entities** (`Ctrl+F`) opens the Find tab below the views.

- Type a class name or any value to list the entities that contain it, ignoring case.
- Type `key=value` to list the entities whose `key` contains `value`.
- Click a result to select it and frame it; **Select all** selects every result.

To replace text, type the new text into **replace with** and click **Replace in values**.
It changes the found text in settings and in output targets and parameters, in the whole map or, with **In selection**, only the selected objects.
It does not work with a `key=value` search, and it is one undo step.

![The Find tab below the views listing info_player_start, zone_start and the zone_split entities, with Select all and Replace in values](img/entities-find.webp#wide)

## Entity problems

**Tools > Check for Problems** (`Alt+P`) reports:

- A class the SDK does not define.
- A brush entity with no brushes, which **Fix** removes.
- A point entity that holds brushes.
- A target or output naming an entity that does not exist; **Fix** clears the setting or removes the output.
