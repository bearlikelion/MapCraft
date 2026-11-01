# Godot environment rendering in MapCraft

`light_environment` controls the Godot addon's `WorldEnvironment` and the editor's Lit viewport.
The SurfsUp SDK's entity definition exposes the controls implemented below.
The viewport is a native wgpu forward renderer; it does not run Godot's renderer.
Full Godot Environment parity is not implemented.

## Implemented controls

| Controls | Editor Lit viewport | Godot addon |
|---|---|---|
| Sun direction/color/energy | Directional light | DirectionalLight3D |
| Sky top/horizon/ground colors | Procedural sky approximation | ProceduralSkyMaterial |
| `sky_curve`, `ground_curve` | Procedural horizon curves | Matching sky material properties |
| `sky_energy_multiplier`, `ground_energy_multiplier` | Sky and hemispherical ambient brightness | Matching sky material properties |
| `ambient_energy` | Hemispherical ambient approximation | Sky ambient energy |
| `fog_enabled`, `fog_density`, `fog_color` | Exponential distance fog | Exponential fog |
| `fog_light_energy`, `fog_sun_scatter`, `fog_sky_affect` | Fog light and directional scattering approximation | Matching Environment properties |
| `tonemap_mode` 0/linear, 1/reinhardt, 2/filmic | Linear, extended Reinhardt, filmic | Matching Godot tonemappers |
| `tonemap_mode` 3/aces | Fitted ACES preview, not an exact Godot color match | Godot ACES |
| `tonemap_mode` 4/agx | Unsupported warning and filmic fallback | Godot AgX |
| `tonemap_exposure`, `tonemap_white` | Exposure; white normalization for Reinhardt/filmic | Matching Environment properties |
| `adjustment_enabled`, `adjustment_brightness`, `adjustment_contrast`, `adjustment_saturation` | After-tonemap display-space adjustment | Matching Environment properties |

Density is expressed per meter; the viewport converts map inches to meters before fog evaluation.
Fog and color processing apply to Lit geometry, baked geometry, and the procedural background.
Textured, Flat, Wireframe and 2D views retain their editing-oriented rendering.
Editor overlays are not fogged or graded.

Non-finite or out-of-range numeric inputs use documented defaults and produce a warning.
Invalid sun angles and color components also fall back with a warning.
Energy/exposure values are limited to 0 to 64, white point to 0.001 to 64, curves to 0.001 to 1, adjustments to 0 to 8, fog density to 0 to 1, and fog scatter/sky affect to 0 to 1.
`fog_enabled=-1` or `auto` uses whether density is positive; explicit zero disables fog.

## Remaining Godot environment work

The following inventory is the work remaining for the requested full Environment support, not a claim that these effects are rendered.
Godot's own renderer also varies feature availability between Forward+, Mobile and Compatibility.

| Effect family | Missing renderer capability |
|---|---|
| Background modes, panorama skies, sky cover, physical sky, sky rotation/custom FOV and energy/intensity modes | Additional sky resources, panorama sampling, atmosphere model and background selection |
| Independent ambient color/source/sky contribution and reflection sources | Ambient source mixing and image-based irradiance/specular cubemaps |
| Sun shadows and contact shadows | Shadow maps, cascade fitting and shadow filtering |
| Glow/bloom and glow maps/blend modes | HDR intermediate targets, threshold/downsample/upsample passes and compositing |
| SSAO | Resolved depth and normal textures, spatial samples, denoising |
| SSIL | Screen-space indirect lighting, reprojection and temporal history |
| SSR | Depth-pyramid tracing, hit history, rough reflection filtering and fallbacks |
| SDFGI | Voxelization, cascaded distance fields, irradiance propagation and ray tracing |
| Volumetric fog | Froxel volumes, light injection, integration and temporal reprojection |
| Height fog, depth fog and aerial perspective | Matching depth/height integration and radiance sampling |
| AgX and renderer-exact ACES | Matching color-space transforms, exposure calibration and tonemap controls |
| Adjustment color-correction LUTs | Bounded LUT resources and color-grading sampling |
| Auto exposure and camera exposure/DOF | Luminance history, CameraAttributes integration and depth-of-field passes |
| Temporal effects and quality controls | Motion vectors, per-viewport history and renderer quality configuration |

Enabled `glow_enabled`, `ssao_enabled`, `ssil_enabled`, `ssr_enabled`, `sdfgi_enabled`, `volumetric_fog_enabled`, `adjustment_color_correction`, `fog_height_density`, `fog_aerial_perspective`, nonzero `fog_mode` and `auto_exposure_enabled` produce viewport warnings instead of silently pretending to render them.
Advanced effects are not currently configured by the addon either; the new addon setters cover only the implemented controls table.
The editor reports warnings through `SceneData.environment_warnings` and the renderer log.

Reference: [Godot Environment](https://docs.godotengine.org/en/4.6/classes/class_environment.html).
