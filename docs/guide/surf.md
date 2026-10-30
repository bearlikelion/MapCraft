# Surf ramps and bhop

Surf ramps are the steep slopes players slide along, and bhop (bunny hop) platforms are the blocks they jump between.
The SurfsUp SDK adds both to the Block tool as two shapes: **Surf Ramp** and **Bhop Platforms**.
Both stay editable after you place them, so you can keep tuning a ramp until it plays right.

## Place a ramp

1. Press `B` for the Block tool, then click **Surf Ramp** in the top bar.
2. Set **Facing** in the inspector, or click the turn icon next to it to add a quarter turn.
3. Click in a view to place it: on the ground or a face in the 3D view, or at the last block's depth in a 2D view.

A yellow ghost shows where the ramp will land.
It stays where the pointer left it while you change settings.
`Alt+B` switches back to the plain shapes.

![Surf Ramp picked in the top bar (1), the quarter-turn icon next to Facing (2) and the yellow ghost in the Top view (3)](img/first-ramp.webp)

> [!TIP]
> The **Plugins** menu makes the same ramps from a window instead.

## Edit a placed ramp

A placed ramp is a group that remembers its settings.
Select it with the Select tool (`Q`) and change its settings in the inspector.
The ramp rebuilds as you type, all in one undo step.
Each shape also remembers the settings you last used, until you quit.

You can also resize it in a view, with the Select tool's handles or the Scale tool.
When you let go, the settings take the new size, in the same undo step:

- **Length or radius**, **Gap**, **Lead in**, **Lead out** and **End slope** follow the ramp's length.
- **Width** and **Ridge** follow its width.
- **Height**, **Tip** and **Drop** follow its height, and **Rise** does on a spiral.
- **Thickness** follows the slab on a thin ramp.
- **Trim top** and **Trim bottom** follow the length of the surf face, and **Trim depth** scales like **Thickness**.

**Angle**, **Bank**, **Shape** and **Segments** stay as you set them.
A taller or narrower box makes the surf face steeper, so check [Keep it surfable](#keep-it-surfable) after a big change.
A curve stays round: it grows by whichever side you stretched more, so stretching a 90 degree turn longer also makes it wider.
Moving and turning a ramp keep its settings as they are.

A ramp that is mirrored, tilted, or resized at an angle to its own direction keeps that change on top of its settings, and the inspector says so; changing a setting keeps it too.
Bhop platforms work the same way: a resized group keeps its new size when you change a setting.
Vertex edits, clips and face drags on a ramp's brushes are replaced the next time you change a setting, so ungroup the ramp first to keep them.

![A curved ramp selected in the Top and Side views, with its Shape, Turn, Length, Angle, Rise, Bank and Segments outlined in the inspector](img/surf-ramp-settings.webp)

## Pick a shape

**Shape** sets the path the ramp follows.

| Shape | Follows |
|---|---|
| Straight | A straight line |
| Left, Right | A flat curve turning left or right |
| Up, Down | A curve that bends upward or downward along its length |
| Dip | A valley: down, then back up |
| Arc | A hump: up, then back down |
| Spiral | A curve that keeps turning and climbs by **Rise** |
| S-Curve | A turn one way, then the other |

**Length or radius** is the length of a straight ramp, or the radius of a curve.
**Angle** is how far a curve turns, in degrees.

![Nine orange ramps, one for each shape: Straight, Left, Right, Up, Down, Dip, Arc, Spiral and S-Curve](img/surf-shapes.webp)

## Shape the cross-section

The cross-section is the ramp's profile, what you would see if you cut straight across it.
These settings shape it:

| Setting | Does |
|---|---|
| **Style** | **Wedge** is a solid ramp; **Thin** is a slab under the surf face |
| **Surf side** | The side players surf: Left, Right, Both, or Inside for a trough of two ramps facing each other |
| **Width**, **Height** | The size of the cross-section |
| **Tip** | A base under a wedge's low edge, so the face never thins to a knife edge players can slip through |
| **Thickness** | How thick a thin ramp's slab is |
| **Ridge** | A flat top on the peak, or the floor of an Inside trough |
| **Bank** | Leans a curve into its turn |
| **Drop** | Height the ramp loses along its length; on a straight ramp it makes a slide |
| **Gap** | Breaks the ramp into pieces this far apart |
| **End slope** | Straight only: the base reaches past the top at each end; negative undercuts, and length 0 makes a pyramid |
| **Lead in**, **Lead out** | Straight runs before and after a curve |
| **Surf**, **Side** and **Bottom material** | Empty uses the current material; the bucket (**Use current material**) fills one in |
| **UV scale** | Map units per texel; the texture runs along the ramp and down its faces, as SURGE lays it out |

## Add trim

Trim is a strip of another material along the top or bottom of each surf face, such as a metal edge on a pointy ramp.
The ramp underneath still does the colliding, so trim never changes how the ramp surfs.

| Setting | Does |
|---|---|
| **Trim top** | How far down each surf face the top trim reaches; it also caps a pointy peak, and covers a flat ridge whole |
| **Trim bottom** | How far up each surf face the bottom trim reaches from its low edge; it covers an Inside trough's floor whole |
| **Trim depth** | How far the trim stands off the surf face |
| **Trim material** | Empty uses the side material |

**Trim top** and **Trim bottom** are 0 by default, which means no trim; **Trim depth** is 1.
The trim follows curves, gaps and both sides of a ramp.
It is a `func_illusionary` in the ramp's group, so players see it but pass through it, and SurfsUp draws it without collision.
Keep **Trim depth** small: players surf on the ramp underneath, so their feet sit that far inside the trim.

## Keep it surfable

A face is surfable when it is steeper than 45.573 degrees.
Anything flatter is floor that players can walk on.
The default ramp is 544 tall and 384 wide with a 64 tip, about 51 degrees.

If a ramp comes out too flat, the Output panel (`Ctrl+5`) says so.
Make it taller or narrower to steepen it.

> [!NOTE]
> The angle is `atan((height - tip) / width)`.
> [Surf mapping](../surf-mapping.md) has the geometry behind it.

## Smooth curves

Curves are built from straight pieces, and a seam between pieces can stop a player dead.
Keep each piece to 5 degrees of turn or less: **Segments** should be at least the **Angle** divided by 5.
A 90 degree turn needs 18 segments or more.
The Output panel warns when a curve has too few.

> [!TIP]
> In Godot, the SurfsUp entity map turns on trimesh collision, so the seams between pieces cannot catch players.
> See [Play it in Godot](godot.md#import-options).

![A curved ramp in the Top view built from 14 pieces, with Segments outlined in the inspector](img/surf-segments.webp)

## Bhop platforms

1. Click **Bhop Platforms** in the Block tool's top bar.
2. Pick a **Pattern**: **Linear**, **Grid** or **Spiral**.
3. Click in a view to place them.

**Platforms** sets how many there are.
The inspector lists every pattern's settings together in the **Bhop Platforms** section; hover one to see which pattern it belongs to.
Linear uses **Spacing** and **Rise**, for example, and Spiral uses **Radius** and **Rise per turn**.

To vary them:

- **Shape** makes them boxes, triangles or cylinders.
- **Size variation** and **Jitter** make them uneven.
- **Face the path** turns each one toward the next.

![Thirty orange platforms climbing in a spiral, with the Spiral pattern outlined in the inspector](img/surf-bhop.webp)

## Zones

Zones are see-through boxes that the game watches: start and finish timers, checkpoints, teleports and kill boxes.
[Zone from Selection](entities.md#zone-from-selection) fits one to a platform, and [Entities and zones](entities.md) lists every zone class.

A few rules keep them working:

- Make zones at least 256 units tall, so players cannot jump over them.
- Make them at least 64 units thick, so fast players cannot skip through them between ticks.
- Make start zones big enough to strafe in before leaving.
- Space checkpoints so each stage takes about ten seconds or more.
- Put a kill zone under every stage.

**Check** (`Alt+P`) warns about zones that are too short or too thin, and about a player start outside the start zone.

![A zone_start over a platform with the player start inside it, and its settings and description in the inspector](img/surf-zone.webp)

## Teleports between stages

1. Put a `teleport_destination` where the next stage starts, and give it a name.
2. Make a `teleport_zone` where the stage ends.
3. Set the zone's **Destination** to that name.

A yellow line joins the selected zone to its destination.
Point each destination the way the stage continues, so players arrive facing forward.

![A teleport_zone selected, with a yellow line to its destination and Destination set to stage2 in the inspector](img/surf-teleport.webp)

## Symmetric maps and terrain

- Build one half of a map and mirror it: [Mirror groups](mirror-groups.md).
- Sculpt rocky ground around the course: [Terrain](displacements.md).
