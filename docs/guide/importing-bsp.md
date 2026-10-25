# Importing BSP maps

The Source BSP plugin reads a compiled Source map, so you can bring an existing Counter-Strike: Source surf map into MapCraft and zone it for SurfsUp.

## Before you import

- Use an SDK that loads the `source-bsp` plugin; the SurfsUp SDK does.
  With no importing plugin, **File > Import** is grayed out.
- For the map's stock textures, set **Counter-Strike: Source game folder** (the `cstrike` folder) in **File > SDK Settings**.
  The plugin then reads `cstrike_pak_dir.vpk` from that folder and `hl2_textures_dir.vpk` and `hl2_misc_dir.vpk` from the `hl2` folder beside it.
  Textures packed into the map itself load without it.

## Import

Choose **File > Import > Source BSP** and pick a `.bsp` file.

- Everything goes into a new layer named after the file, as one undo step, and the views frame it.
- The Output panel (`Ctrl+5`) gets a summary: world brushes and how many are ramps, entities, brush entities, static props, dropped brushes, skipped displacements, and how many textures came from the map, from the game or were missing.
- It reminds you to set the Counter-Strike: Source folder when textures are missing because of it.

## What comes in

- **Versions:** BSP versions 19 to 21 (Counter-Strike: Source, Half-Life 2, Garry's Mod); other versions are refused with a message naming the version.
  Compressed (LZMA) lumps are read.
- **Brushes:** rebuilt from the brush planes the compiler kept, so you get solids, not the compiled faces, with their materials and texture alignment.
  Sides with no texture get `tools/toolsnodraw`.
- **Brush entities:** their brushes, placed by the entity's origin and angles, with every key value except `model`.
- **Point entities:** with all their key values.
- **Static props:** as `prop_static` point entities with `model`, `origin`, `angles` and `skin`.
- **Materials:** faces keep the Source material names (lower case, without `materials/`), and patched cubemap copies go back to their base material.
  Pictures come from the map's own packed files first, then from the game's packages, shrunk to fit a memory budget.

The pictures are saved as PNG files in the `materials` folder of MapCraft's data folder (`~/.local/share/mapcraft/materials` on Linux) and show in the Materials tab, in later sessions too.

The layer holds a group per kind of content; empty groups are left out:

| Group | Holds |
|---|---|
| Ramps | World brushes with a surfable face: too steep to stand on, not a wall, at least 64 by 64 units |
| World | The other world brushes |
| Triggers | `trigger_` entities |
| Spawns | `info_player_` entities |
| Teleport destinations | `info_teleport_destination` |
| Lights | `light` entities and `env_sun` |
| Props | `prop_` entities, static props included |
| Brush entities | `func_` entities |
| Other | Everything else |

## Limits

- Displacements are skipped.
- Brushes with no volume, and brushes with more than 2048 sides, are dropped.
- Overlays are not read.
- Models are not drawn: props are only points.
- Textures that were not found show as MapCraft's missing texture checkerboard.
- Godot looks for materials under its material root, not in MapCraft's data folder; copy the PNG files from there into that folder, keeping their subfolders, to see the textures in Godot.

## Zone a CS:S surf map for SurfsUp

1. Import the `.bsp` as above, then save it as a `.mcmap`.
2. Switch to **Objects** (`Alt+2`) or **Solids** (`Alt+3`) selection mode: in Groups mode a click selects a whole import group.
3. Run **Tools > Check for Problems** (`Alt+P`): it lists every Counter-Strike class the SurfsUp FGD does not define.
4. **Spawn:** select one of the spawns in the Spawns group and pick `info_player_start` in the inspector's class dropdown, or place a new one with the Entity tool (`N`), and delete the rest.
5. **Start and finish:** click `zone_start` in the Entities tab, select the start platform and choose **Tools > Zone from Selection**; do the same with `zone_finish`.
6. **Stage teleports:** find them with **View > Find Entities** (`Ctrl+F`) and `trigger_teleport`.
   For each one, pick `teleport_zone` in the class dropdown and set **Destination** to the old `target` value, then delete the old `target` key.
   Turn each `info_teleport_destination` into a `teleport_destination` the same way.
7. **Brush entity geometry:** in Godot, brush entity classes the SurfsUp entity map does not know become invisible, non-solid areas.
   Select geometry such as `func_brush` or `func_wall` and use **Tools > Move to World** (`Ctrl+Shift+W`), or pick `func_detail` as its class, so it stays solid and visible.
8. Run Check for Problems again and fix what is left.

In Godot, turn on **Trimesh Collision** for the map so seams between ramp brushes cannot catch players; see [The Godot workflow](godot.md#import-options).
[Entities and zones](entities.md) covers the inspector and Zone from Selection in more detail.

## Save zones for SurfsUp's own BSP maps

SurfsUp v3 can also keep playing the original `.bsp` and read just its zones from a `.zon` file.

1. Save the map under the BSP's file name, such as `surf_emerald.mcmap`: SurfsUp looks the zone file up by that name.
2. Place the zones:
   - `zone_start` and `zone_finish` with **Course** 0 for the main course; both are required.
   - More `zone_finish` zones for extra finishes.
   - `zone_checkpoint` at each stage start, and `zone_split` for split lines.
   - `zone_start` and `zone_finish` with **Course** above 0 for bonuses.
3. Choose **Run > Export zones to SurfsUp**.
   It writes `Resources/SurfZones/<map>.zon` in the SurfsUp project folder; if a different file was there, it is kept beside it as `<map>.zon.bak`.
4. Import the BSP again in SurfsUp v3: zones are built into the level when the BSP is imported, not when the game runs.

Kill boxes, teleports, boosters and respawn zones have no place in a `.zon` file; the Output panel lists every zone it skipped.
**File > Export > SurfsUp zones** writes the same file anywhere you choose.
