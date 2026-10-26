# SDK manifests

An SDK is a folder with an `sdk.toml` file.
It tells MapCraft how to edit maps for one game: which entities exist, where materials live, which plugins to load and how to compile and launch a map.
Game content (textures, models, FGDs a game ships itself) stays in the user's game install; the SDK points at it.

## Folder layout

```
my-game/
  sdk.toml          the manifest below
  fgd/              entity definitions the manifest lists
  plugins/<id>.wasm one file per id in `plugins`
  samples/*.mcmap   example maps; the SDK Manager's Test button opens the first one
  README.md         what the SDK is for and how to set it up
```

File > SDK Manager… lists every SDK MapCraft found, checks them, installs more from registries and makes new ones from a template.
`mapcraft-cli sdk new <dir> --id <id> --name <name>` writes the same template.
Publishing an SDK to a registry is described in [`sdk-registry.md`](sdk-registry.md).

## Example

```toml
id = "surfsup"
name = "SurfsUp"
version = "1.0.0"
description = "Surf maps for SurfsUp."
author = "MapCraft"
license = "MIT"
homepage = "https://example.com/surfsup"
plugins = ["godot", "surfsup"]
units_per_meter = 39.37
fgd = ["fgd/surfsup.fgd"]

[textures]
roots = ["${project}/Assets/Textures"]
extensions = ["png", "jpg"]
default = "ramps-orange1"
scale = 1.0

[models]
roots = ["${project}"]
class = "prop_static"

[paths.project]
prompt = "SurfsUp Godot project folder"

[paths.godot]
prompt = "Godot editor executable"
kind = "file"

[[compile]]
name = "Build and play"
steps = [
    { copy = "${map}", to = "${project}/maps/${map_name}.mcmap" },
    { run = "${godot}", args = ["--path", "${project}", "--", "+map", "${map_name}"] },
]
```

## Keys

| Key | Meaning |
|---|---|
| `id` | Lowercase letters, digits, `_` and `-`; stored in maps made with this SDK |
| `name` | Display name |
| `version` | Version text; dotted numbers such as `1.2.0` let the SDK Manager tell which copy is newer |
| `description` | One or two sentences, shown in the SDK Manager and copied into registry entries |
| `author` | Who maintains the SDK |
| `license` | SPDX expression for the SDK's own files, such as `MIT` |
| `homepage` | Where to read more or report problems |
| `plugins` | Ids of the WebAssembly plugins the SDK needs |
| `units_per_meter` | Map units per meter, 39.37 by default |
| `map_size` | The lowest and highest coordinate maps may use on every axis, such as `[-4096, 4096]` for Quake; unset, an FGD's `@mapsize` gives it, and without either maps are unbounded |
| `fgd` | Entity definition files, merged in order; later classes replace earlier ones |
| `textures.roots` | Folders searched for materials, first match wins |
| `textures.extensions` | File types that count as materials |
| `textures.default` | Material for new brushes |
| `textures.scale` | Texture scale for new faces |
| `models.roots` | Folders glTF model paths are relative to; empty uses `textures.roots` |
| `models.class` | Point entity class the Models tab places models with; unset, models can be previewed but not placed |
| `models.key` | The key that holds a model's path, `model` by default |
| `files.roots` | More folders plugins may read, such as a game install; ones whose paths are unset are skipped |
| `paths.<name>` | A folder or program the user picks once; available as `${name}` |
| `compile` | Compile profiles shown in the Run menu |

Relative paths are relative to the SDK folder.
Unknown keys are an error, so typos are caught when the SDK loads.

## Variables

Any string in `fgd`, `textures.roots`, `models.roots` and compile steps can use `${name}`.
Besides the `paths` the user set, these are always available:

| Variable | Value |
|---|---|
| `${map}` | Full path of the map being compiled |
| `${map_dir}` | Its folder |
| `${map_name}` | Its file name without extension |
| `${sdk_dir}` | The SDK folder |
| `${cpu_count}` | Number of CPU threads |

An unknown variable stops the compile with an error instead of expanding to nothing.

## Compile steps

- `{ run = "...", args = [...], cwd = "...", wine = true }` runs a program. With `wine = true`, a Windows tool runs through Wine on Linux and macOS.
- `{ copy = "...", to = "..." }` copies a file, creating the destination folder.
- `{ export = "id", to = "..." }` writes the map with the exporter `id` of one of the SDK's plugins, such as a Quake `.map` before `qbsp` runs. Prefabs and mirror copies are resolved first, and layers left out of exports and anything outside an active cordon are dropped.

Export steps run first, before the other steps start, because plugins run in the editor; the other steps then run in order and the profile stops at the first failure.
MapCraft asks before running an SDK's programs the first time.

## Where SDKs are found

MapCraft searches these folders in order and uses the first SDK with a given id:

1. `sdks` in the user's data folder, where the SDK Manager installs and creates SDKs.
2. An `sdks` folder beside the executable or up to four folders above it, and `../share/mapcraft/sdks`.
3. The `sdks` folder of the source tree MapCraft was built from.
4. Folders in `MAPCRAFT_SDK_PATH`.
5. Extra SDK folders from Preferences.
