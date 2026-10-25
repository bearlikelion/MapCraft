# SDK manifests

An SDK is a folder with an `sdk.toml` file.
It tells MapCraft how to edit maps for one game: which entities exist, where materials live, which plugins to load and how to compile and launch a map.
Game content (textures, models, FGDs a game ships itself) stays in the user's game install; the SDK points at it.

## Example

```toml
id = "surfsup"
name = "SurfsUp"
version = "1.0.0"
plugins = ["godot", "surfsup"]
units_per_meter = 39.37
fgd = ["fgd/surfsup.fgd"]

[textures]
roots = ["${project}/Assets/Textures"]
extensions = ["png", "jpg"]
default = "ramps-orange1"
scale = 1.0

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
| `version` | Free-form version text |
| `plugins` | Ids of the WebAssembly plugins the SDK needs |
| `units_per_meter` | Map units per meter, 39.37 by default |
| `fgd` | Entity definition files, merged in order; later classes replace earlier ones |
| `textures.roots` | Folders searched for materials, first match wins |
| `textures.extensions` | File types that count as materials |
| `textures.default` | Material for new brushes |
| `textures.scale` | Texture scale for new faces |
| `paths.<name>` | A folder or program the user picks once; available as `${name}` |
| `compile` | Compile profiles shown in the Run menu |

Relative paths are relative to the SDK folder.
Unknown keys are an error, so typos are caught when the SDK loads.

## Variables

Any string in `fgd`, `textures.roots` and compile steps can use `${name}`.
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

Steps run in order and the profile stops at the first failure.
MapCraft asks before running an SDK's programs the first time.
