# Surf mapping with the SurfsUp SDK

This page covers the surf-specific tools in the SurfsUp SDK and the geometry rules behind them.

## Making things

Pick the Block tool (B); the shape bar lists the plugin generators next to the plain shapes.

| Shape | Makes |
|---|---|
| Surf Ramp | Straight, curved, spiral, S-curve, dip and arc ramps, solid or thin |
| Bhop Platforms | Lines, grids and spirals of box, triangle or round platforms |

While a generator is picked, its parameters are in the inspector and a ghost follows the pointer; click to place it.
Placed generators stay editable: select the group and change any parameter, and the brushes are rebuilt live.

## The surf face

A surf face is surfable when it is steeper than 45.573 degrees; anything flatter is walkable floor.
Its angle is `atan((height - tip) / width)`, and the generator warns when a ramp falls under the limit.
The default ramp is a 5:4 slope: a rise of 480 over a run of 384, about 51.3 degrees.

The tip is a base under the surf face's low edge, so the face never thins to a knife edge players can slip through.

## Ramp parameters

| Parameter | Effect |
|---|---|
| Drop | The ramp loses this much height over its length; on a straight ramp it makes a slide |
| Ridge | A flat top on the peak, or the floor of an Inside ramp |
| Surf side | Left, Right, Both, or Inside for a trough of two ramps facing each other |
| Gap | Breaks the ramp into pieces this far apart, a broken ramp to strafe across |
| End slope | Straight ramps only: the base reaches past the top at each end, a negative value undercuts, and length 0 makes a pyramid |
| Lead in, Lead out | Straight runs before and after a curve |
| Bank | Leans a curve into its turn |

## Curves and seams

Curves are built from straight pieces, and a seam between pieces can catch a player and stop them dead.
Keep each piece to 5 degrees of turn or less; the generator warns above that, and a 90 degree turn needs at least 18 segments.
MapCraft joins pieces exactly, with no grid rounding, so seams only come from the game's collision.
In Godot, turn on Trimesh collision in the MapCraft import options or on a `MapCraftMap` node: the whole world then collides as one triangle mesh instead of one convex shape per brush.

## Zones

- To zone a platform, click the zone class (such as `zone_start`) in the Entities tab, select the platform, and use Tools > Zone from Selection: the zone covers the platform's top and stands 256 units tall.
- Make zones at least 256 units tall so players cannot jump over them.
- Make start zones big enough to strafe in before leaving.
- Space checkpoints so each stage takes about ten seconds or more.
- Point every `teleport_destination` the way the stage continues, so players arrive facing forward.
- Put a kill zone under every stage.

## Symmetric maps

Build one half, select it, and use Tools > Make Mirror Group (Ctrl+Shift+M).
The other half follows every edit, and the saved map holds both halves so the game sees a whole map.

## Terrain

Turn the top faces of floor brushes into displacements with the Face tool's Displacement section, then shape them with the Sculpt tool (D) or a heightmap image.
