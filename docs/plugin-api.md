# Plugin API

MapCraft plugins are WebAssembly components built against [`wit/plugin.wit`](../wit/plugin.wit).
Any language that can target the component model works: Rust, C and C++, C#, Go, JavaScript, Python and Zig.
The core editor has no game-specific code; plugins provide it.

## What a plugin can do

A plugin exports eight functions:

| Export | Purpose |
|---|---|
| `manifest` | Its id, name, version, commands, importers, exporters, the model file types it reads, and whether it lists materials or validates maps |
| `run-command` | Runs one of its commands with parameter values and returns brushes and entities |
| `list-materials` | Lists the SDK's materials for the browser, with a preview image or color each |
| `load-materials` | Returns full-size pictures of materials it listed smaller, for the textures a map uses |
| `import-file` | Reads a file the user picked for one of its importers and returns brushes and entities |
| `export-file` | Writes the map in one of its export formats and returns the file's bytes |
| `validate` | Checks the map and returns problems for Check for Problems |
| `load-model` | Reads a model a point entity names, for the views |

It can call five host functions:

| Import | Purpose |
|---|---|
| `log` | Writes a line to the Output panel |
| `list-files` | Lists files in the SDK's asset folders by extension |
| `read-file` | Reads a file from the SDK's asset folders |
| `read-range` | Reads part of a file from them, for archives too big to read whole |
| `file-path` | Gives where one of those files is on disk, for an export that a compiler outside the editor reads |

Plugins get nothing else: no other files, no network, no processes, no environment.
Each call has a fuel budget of a few seconds of work and each plugin a memory cap, so a plugin that loops or runs away is stopped and reported instead of freezing the editor.
A plugin that traps is switched off until the SDK is reloaded.

## Commands

A command declares its parameters, and the editor builds the form for them: numbers and integers with ranges, toggles, choices, text and materials.
Values arrive filled in and clamped to their ranges, so a plugin never sees a missing or out of range value.

A command makes its output around the origin.
The editor places it at the selection's center, or where the cursor last was in a 2D view.

Brushes come in two forms:

- `faces`: three points per face plane, clockwise seen from outside the brush, the same convention as `.map` and `.vmf` files.
- `hull`: the convex solid around a list of points.
  Faces get the hull's material, except those whose outward normal is within about 8 degrees of a `surface` hint, which get that hint's material.
  It is the simpler form for generators: list the corners and say which way the special faces point.

An empty material means the material picked in the browser.
Every command, import and export also gets a `context`: the grid size, the material picked in the browser, and `game`, the SDK's name, which the Quake plugin writes as the game a `.map` is for.

A `faces` face can also carry `attrs`, the game attributes MapCraft keeps per face (such as `lightmapscale`, `smoothing_groups` or `anti_tile`), and a `disp`, a displacement:

| Field | Meaning |
|---|---|
| `power` | 1 to 4; the grid has 2^power + 1 vertices a side |
| `corners` | The face's four corners in grid order: the start corner, then counter-clockwise seen from outside; coming back from a plugin only the first is read, as the start corner |
| `heights` | Each vertex's height along the face's outward normal, row by row: a row runs from the start corner toward the second corner, and rows step toward the fourth |
| `blend` | Each vertex's share of the second layer, 0 to 1, in the same order; empty is 0 everywhere |
| `blend-material` | The second layer's material, when the face has one |

The editor keeps at most 64 attributes a face, each key and value at most 1024 bytes.
A displacement whose power is not 1 to 4, whose heights or blend do not fill the grid, or that holds a height that is not a number comes in flat, and the Output panel counts it.
A displacement on a face without exactly four corners is kept but draws and exports flat.

### Generator groups

A command with `generator: true` puts its output in a group that remembers the plugin, the command, the parameters and where the group sits.
Selecting the group shows the parameters in the inspector; changing one runs the command again and replaces the group's contents, as one undo step per edit.
Moving or turning the group moves the generator with it.
Exporters and engine importers see ordinary brushes, so a map made with a generator does not need the plugin to load.

## Importers

A manifest's `importers` list file formats the plugin reads, each with an id, a label and its extensions.
They appear under File > Import.
The editor reads the picked file and passes its bytes and file name to `import-file`, so an importer needs no file access of its own.
The output is placed as it is, in map space, in a new layer named after the file.
A point entity named `worldspawn` in the output is not placed: its keys go on the map's world, filling in those the world does not have yet, so a file's level settings (such as Quake's `_sunlight` or `message`) come along without replacing the open map's own.
An entity's `connections` are its outputs (Hammer's I/O), each wiring an output to an input of the entities named `target`, with a parameter, a delay in seconds and how many times it fires (-1 for every time); they show in the inspector's Outputs tab.
A plugin with no importers returns an empty list and an error from `import-file`.

## Exporters

A manifest's `exporters` list file formats the plugin writes, each with an id, a label and its extensions.
They appear under File > Export, and compile profiles run them with an `export` step (see [`sdk-format.md`](sdk-format.md)).
`export-file` gets the map as `map-data`: worldspawn's key values, then a flat list of nodes (layers, groups, entities and brushes) where each node names its parent by index and parents always come first.
Every brush comes with its faces (three points, material, texture axes, `attrs` and `disp`) and its corners, and every entity with its key values and `connections`.
A face's `disp` always has a power of 1 to 4 and a full grid of heights, with `corners` filled in; a displacement that does not fit its face (one that is not four-cornered, or whose heights do not fill the grid) goes out flat, and the Output panel counts it.
`blend-material` is set only when it is a real second layer: not empty, not the face's own material and not a `tools/` material.
The editor has already resolved prefabs and mirror copies and removed layers left out of exports and everything outside an active cordon, so an exporter writes what it gets.
It returns the file's bytes, and the editor saves them.

## Validators

A plugin that sets `validates` in its manifest is asked to check the map whenever Check for Problems runs.
`validate` gets the same `map-data` an exporter gets and returns a list of issues, each a warning or an error with a message and, when one node is at fault, that node's index in `map-data.nodes`.
The editor lists them with its own problems and selects the node when the user clicks the issue.
A plugin that does not validate returns an empty list.

## Materials

A plugin that sets `materials` in its manifest replaces the plain image listing of the SDK's texture folders.
Each material has a name, which is what faces store and what engine importers look up, and a preview: an image path in the asset folders, or an sRGB color.
A plugin that returns its own pictures (`pixels`) may shrink them to save memory; it then sets `size` to the texture's real width and height, because texture scales and offsets count real texels.
Importers do the same for the materials they return.
When a map uses a material whose listed picture is smaller than its `size`, the editor asks the plugin's `load-materials` for the full picture of every such material at once, so a big library can list small previews and the views still draw the textures sharp.
A plugin that never shrinks its pictures returns `none` for each name.

`pbr` gives the maps the Lit view shades with, as paths in the asset folders; the editor reads them only when a map uses the material, so listing stays cheap.
Leave it out (`none`) for materials without such maps; they draw fully rough and not metallic.

| Field | Meaning |
|---|---|
| `normal` | Tangent space normal map, green pointing up the picture (OpenGL, as Godot uses) |
| `normal-dx` | The normal map is DirectX style, green pointing down; the editor flips it |
| `roughness`, `metallic`, `ao` | A `channel-map`: a picture's path and the channel (`red`, `green`, `blue`, `alpha` or `gray`) holding the value |
| `roughness-value`, `metallic-value` | Multiply their map, as in Godot, or stand alone without one; 0 to 1 |

Ambient occlusion darkens only ambient light.
A packed occlusion, roughness and metallic picture is one path named three times with the channels `red`, `green` and `blue`.

Three more fields say how the game draws the material:

| Field | Meaning |
|---|---|
| `uv-scale` | How many times the texture repeats across its own texel size, as Godot's `uv1_scale`; the views tile it as densely, ignoring the sign; `none` is once |
| `pixelated` | The game draws it with nearest filtering, so the views do too |
| `blend` | `layer` for a material that can be either layer of a blended displacement, `own` for one that holds two textures and mixes them by vertex weight itself (such as Source's WorldVertexTransition), `unblendable` for one painting refuses |

## Models

The editor reads glTF models itself; a plugin reads the game's own model formats.
A plugin lists the extensions it reads in its manifest's `models`, such as `mdl` and `bsp` for Quake, and the editor then asks it, through `load-model`, for every model path with one of them that an entity's class or `model` key names.
The model comes back as parts, each a list of positions in map units around the entity's origin and facing +X, one UV per position (0 to 1 across the texture, V down), three indices per triangle and an index into the model's `textures`, which are RGBA pictures.
The editor works out the normals and places the model by the entity's `origin`, `angles` or `angle`, and `scale`.
A model that fails to load shows as the entity's box, with the plugin's message in the Output panel.
A plugin that reads no models leaves `models` empty and returns an error from `load-model`.

`load-model` also reads the file an SDK's bake profile leaves (see [`sdk-format.md`](sdk-format.md)), such as a compiled `.bsp`, which the Lit view draws in place of the brushes.
For that, each part names its SDK `material` and can carry a `lightmap`, an index into `textures`, with one `lightmap-uvs` coordinate per position.
The material's sRGB color is multiplied by twice the lightmap's value and clamped, as Quake draws lightmaps, so 128 shows the material as it is; a part without a lightmap shows the material unlit, and sky materials show the sky.
The Quake plugin colors a `.bsp`'s lightmaps from its BSPX `RGBLIGHTING` lump (ericw-tools `light -bspxlit`), else from a `.lit` beside it, else in gray.
It places them by the BSPX `LMSHIFT`, `LMOFFSET`, `LMSTYLE` and `DECOUPLED_LM` lumps when the `.bsp` has them, so lightmaps at other scales than 16 units a luxel line up.

## Where plugins come from

- An SDK lists the plugin ids it needs in `sdk.toml` (`plugins = ["godot", "surfsup"]`), and MapCraft loads `plugins/<id>.wasm` from the SDK folder.
- Every `.wasm` file in the user's plugins folder is loaded too, whatever the SDK: `~/.local/share/mapcraft/plugins` on Linux, `%LOCALAPPDATA%\mapcraft\plugins` on Windows, `~/Library/Application Support/mapcraft/plugins` on macOS.

## Writing a plugin in Rust

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.63"
```

```rust
wit_bindgen::generate!({ path: "wit", world: "plugin" });

struct Mine;

impl Guest for Mine {
    fn manifest() -> Manifest { /* ... */ }
    fn run_command(id: String, params: Vec<Param>, context: Context) -> Result<Output, String> { /* ... */ }
    fn list_materials() -> Result<Vec<MaterialInfo>, String> { Ok(Vec::new()) }

    fn import_file(_importer: String, _name: String, _data: Vec<u8>, _context: Context) -> Result<Output, String> {
        Err("this plugin imports no files".into())
    }

    fn export_file(_exporter: String, _map: MapData, _context: Context) -> Result<Vec<u8>, String> {
        Err("this plugin exports no files".into())
    }

    fn validate(_map: MapData) -> Vec<Issue> {
        Vec::new()
    }

    fn load_materials(names: Vec<String>) -> Vec<Option<Image>> {
        names.iter().map(|_| None).collect()
    }

    fn load_model(path: String) -> Result<Model, String> {
        Err(format!("this plugin reads no models: {path}"))
    }
}

export!(Mine);
```

Build with `cargo build --target wasm32-wasip2 --release`.
The first-party plugins in [`plugins/`](../plugins) are complete examples; `cargo xtask plugins` builds them and copies each into the SDKs that list it.

## Writing a plugin in Python

[`plugins/stairs-py/app.py`](../plugins/stairs-py/app.py) is a staircase generator written in Python.
Build it with [componentize-py](https://github.com/bytecodealliance/componentize-py):

```sh
pip install componentize-py
cd plugins/stairs-py
componentize-py -d ../../wit -w plugin componentize app -o stairs.wasm
```

Then copy `stairs.wasm` into your user plugins folder.
A Python plugin carries its own interpreter, so it is about 18 MB and takes a couple of seconds to load.

## Versioning

The WIT package is `mapcraft:plugin@0.2.0`.
Until 1.0 the API may change between releases, and a plugin built for another API version is not loaded and the Output panel asks for it to be rebuilt.
Version 0.2.0 added face `attrs` and `disp` and material `uv-scale`, `pixelated` and `blend`, so a plugin built for 0.1, in an SDK or in the user's plugins folder, is not loaded and the Output panel asks for it to be rebuilt against 0.2.
