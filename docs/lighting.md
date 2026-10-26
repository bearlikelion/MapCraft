# Lit preview

The Lit render mode (`Ctrl+Alt+4`) previews lighting and materials in the 3D view without compiling.
This page lists what it reads, for SDK and material authors; mappers only need [Getting around](guide/views.md#lit).

## Shading

Faces are shaded the way game engines such as Godot shade them: metallic and roughness maps, normal maps for surface detail, and ambient occlusion that darkens ambient light in creases.
Each face takes light from the map's point lights, spotlights and sun, with distance falloff, plus a faint ambient so dark corners stay readable.

- Source lights fall off with the inverse square of distance.
- Quake lights shade the way the game's compiled lightmaps look (see [Quake lights](#quake-lights)).
- Only the 64 lights nearest the camera shine at once.
- Nothing casts shadows, so a sun also lights faces indoors.
- Entity boxes stay unlit.

## Quake lights

Quake light values are lightmap levels, not physical intensities, so Lit adds Quake lights up the way ericw-tools' light compiler fills a lightmap and a Quake engine shows it.
A light is a Quake light when it has a lone brightness rather than Source's `r g b brightness`.

- A face right next to a `light 255` shows its texture as is; where lights overlap it gets up to twice as bright, as Quake's overbright lighting does.
- Falloff follows `delay`: linear over `light / wait` units by default, and the inverse and inverse square falloffs (`delay` 1, 2 and 5) are scaled by 128 units divided by `wait`.
- A face lit at an angle keeps at least half the light (`_anglescale`, 0.5 by default), so floors under wall torches stay bright.
- Worldspawn `light` or `_minlight` sets the minimum light level, and Lit never shows a Quake map darker than a faint minimum of its own, so unlit corners stay readable.
- Worldspawn `_sunlight` adds ericw-tools' sun; `_sunlight2`, `_sunlight3`, bounce and dirt are not previewed.

Without shadows, light from neighboring rooms shines through walls and floors, so a map whose long inverse falloffs are stopped by walls in the game looks brighter in Lit.

## The stand-in rig

A map without light entities gets a stand-in rig, so materials can still be judged:

- a warm sun from high above at +X +Y, where the Textured mode's light comes from;
- a weaker cool fill from the opposite side;
- sky and ground ambient, bluish from above and brownish from below.

Once the map has a light, Lit uses only the map's lights.

## Light keys

Lit reads the usual Source and Quake light keys, so it works with any SDK whose lights follow either convention.

| Key | Read as |
|---|---|
| classname | `light` or `light_*` is a light, `light_spot` a spotlight and `light_environment` the sun; any entity with `_light` is a light too |
| `_light` | Source color and brightness, `r g b brightness`; a lone number is a Quake brightness |
| `light`, `_color` | Quake brightness (300 when missing) and color, `r g b` from 0 to 255 or 0 to 1 |
| `wait`, `delay` | Quake falloff distance scale and falloff curve (0 linear, 1 inverse, 2 inverse square, 3 none, 5 softened inverse square) |
| `_anglescale` | How much a Quake light dims on faces it strikes at an angle, 0 to 1 |
| `angles`, `pitch` | Source spotlight and sun aim; pitch -90 points straight down |
| `_cone`, `_inner_cone` | Source spotlight cone, in degrees from its axis |
| `mangle`, `angle`, `_softangle` | Quake spotlight aim (yaw, pitch, roll) and cone |
| `_ambient` | Ambient color and brightness, `r g b brightness` |

Worldspawn keys, as ericw-tools reads them:

| Key | Read as |
|---|---|
| `light` or `_minlight`, `_minlight_color` | Minimum Quake light level and its color |
| `_sunlight`, `_sunlight_color` | Sun brightness and color |
| `_sunlight_mangle` or `_sun_mangle` | Sunlight direction (yaw, pitch, roll); straight down when missing |
| `_anglescale` | The sun's angle scale |

## Godot sky and sun

A `light_environment` without `_light` is read the way the MapCraft Godot addon builds its sky and sun (see the [Godot addon](../godot/README.md#entity-maps)), so tuning its keys in the inspector previews the look the game gets.

- Lit draws Godot's procedural sky behind the map from `sky_top_color`, `sky_horizon_color` and `ground_color`, with a glow around the sun.
- The sun shines along `angles`, pitch in degrees below the horizon, with `sun_color` and `sun_energy`.
- Surfaces take light from the sky scaled by `ambient_energy`.
- The picture goes through Godot's filmic tonemapper.

Unset keys take the addon's defaults; fog and shadows are not previewed.

## Material maps

The maps come from the SDK's material plugin (see [Materials](plugin-api.md#materials) in the plugin API) and are read when a map first uses the material.

- **Godot materials** (`.tres`): `normal_texture` when `normal_enabled` is on, `roughness_texture` and `metallic_texture` with their channels, `ao_texture` when `ao_enabled` is on, and the `roughness` and `metallic` values, which multiply their maps and default to 1 and 0 as in Godot.
- **Loose image sets** with no `.tres`, named as ambientCG names them: next to `Bricks_Color.jpg`, the files `Bricks_NormalGL`, `Bricks_NormalDX`, `Bricks_Roughness`, `Bricks_Metalness`, `Bricks_AmbientOcclusion`, or a packed `Bricks_ORM` or `Bricks_ARM` (occlusion, roughness and metallic in red, green and blue).
- Normal maps are OpenGL style, green pointing up the picture, as Godot expects; a DirectX map (`NormalDX`) is flipped, and `NormalGL` wins when a set has both.
- Source and Quake materials have no such maps and draw fully rough and not metallic.

Normal and roughness maps are loaded at up to 1024 pixels on a side.
