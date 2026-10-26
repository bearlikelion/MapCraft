# TrenchBroom and func_godot import

**File > SDK Manager > Create > Import from TrenchBroom / func_godot** and `mapcraft-cli sdk import-trenchbroom <folder> [--out <dir>] [--data <dir>]` build an SDK from a TrenchBroom game config, as func_godot's `TrenchBroomGameConfig` resource exports it, and from the func_godot map settings of the Godot project behind it.
The editor writes the SDK into the user SDK folder; the CLI does too unless `--out` names the SDK folder.
The code is `crates/sdk-manager/src/trenchbroom.rs`; the user guide is [TrenchBroom and func_godot](guide/trenchbroom.md).

## Inputs

The folder is either kind, and the importer looks for the other:

| Picked | Holds | Finds the other through |
|---|---|---|
| TrenchBroom game folder | `GameConfig.cfg` and the FGDs it lists | TrenchBroom's `Preferences.json` two folders up, whose `Games/<name>/Path` is the game path; the project is that folder or the nearest one above it holding `project.godot` |
| Godot project | `project.godot` | `TRENCHBROOM_GAME_CONFIG_FOLDER` in `func_godot_config.json` (or the older `<Name>_FuncGodotConfig.json`), which func_godot saves in the project's user data folder, `app_userdata/<config/name>` under Godot's data folder |

The map settings are the first `FuncGodotMapSettings` resource in the project outside `addons`, else the addon's `func_godot_default_map_settings.tres`, else func_godot's defaults.
`GameConfig.cfg` is read as JSON with `//` and `/* */` comments allowed; versions 9 (`materials`), 8 (`textures`) and 4 to 7 (`textures.package` and `textures.format`) are understood.

## What goes where

| `sdk.toml` | From |
|---|---|
| `id` | The game name in lowercase, other characters as `-`, else `trenchbroom-game` |
| `name` | `name` in `GameConfig.cfg` |
| `plugins` | `godot` for the material list and `quake` for `.map` import and export, copied from the first installed SDK that ships each |
| `units_per_meter` | `inverse_scale_factor` of the map settings, 32 by default |
| `fgd` | `entities.definitions`, copied into `fgd/` with the files they `@include` |
| `textures.roots` | `base_texture_dir` with `res://` as `${project}/`, plus `base_material_dir` when that folder exists; without map settings, the config's material root under TrenchBroom's game path |
| `textures.extensions` | `texture_file_extensions` of the map settings, else the config's material extensions, keeping the types MapCraft can show |
| `textures.default` | The texture on the most brush faces of the project's `.map` files, else the first image in the texture folder; names matching the config's brush face tag patterns or `excludes`, and TrenchBroom's `__TB_empty`, are skipped |
| `textures.scale` | `faceattribs.defaults.scale` |
| `models.roots` | TrenchBroom's game path, which FGD `model(...)` paths are relative to |
| `paths.project` | The Godot project folder, defaulting to the project found |
| `compile` | **Export .map to Godot project**: `{ export = "map", to = "${project}/<maps>/${map_name}.map" }`, where `<maps>` is the project folder holding the most `.map` files outside `addons` and TrenchBroom's `autosave` folders, else `maps` |

The SDK also gets a `README.md` saying where it came from.

## Why a Valve220 .map

func_godot users already build `.map` files with a FuncGodotMap node, so the SDK exports one instead of switching them to MapCraft's own Godot addon.
The `quake` plugin's exporter writes Valve220 with layers and groups as TrenchBroom's `func_group` entities (`_tb_type`, `_tb_id`, `_tb_layer`, `_tb_group`), which func_godot reads for its groups hierarchy, and leaves out layers excluded from exports.
Material names are paths under the texture folder without the extension, which is how func_godot looks textures and materials up under `base_texture_dir`.
The exporter warns about texture names longer than Quake's 15 characters; func_godot has no such limit.

## Not carried over

- `FuncGodotFGDFile` resources are not read: the exported FGD already holds what they generate, and building one from the resources would mean reimplementing func_godot's exporter and following its changes.
- `.def` and `.ent` definitions, `.wal` textures, palettes, and texture WADs listed in the map settings; WADs inside the texture folder still list through the `quake` plugin.
- Brush and face tags, content and surface flags, the entity scale expression, the icon, and the map settings' clip, skip and origin textures.
- Godot projects with a custom user data folder (`use_custom_user_dir`), and NetRadiant Custom gamepacks.

## Hostile input

- `GameConfig.cfg`, `project.godot`, `Preferences.json` and `func_godot_config.json` are read up to 1 MiB, FGDs up to 16 MiB, and `.map` files up to 16 MiB each and 64 MiB in all.
- The project walk skips hidden folders, never follows links, and stops 16 folders deep or after 100000 entries.
- FGDs and `@include`s outside the game folder are refused; a damaged FGD is copied with its problems listed, and one that cannot be read is left out.
- A config that is not JSON or has no name is refused with the reason; folders containing `${` are not written into the manifest.
- The import never replaces an existing folder, loads the written manifest before reporting success, and removes the folder when anything fails.
