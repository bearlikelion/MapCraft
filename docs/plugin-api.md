# Plugin API

MapCraft plugins are WebAssembly components built against [`wit/plugin.wit`](../wit/plugin.wit).
Any language that can target the component model works: Rust, C and C++, C#, Go, JavaScript, Python and Zig.
The core editor knows no game; plugins teach it how to work with one.

## What a plugin can do

A plugin exports five functions:

| Export | Purpose |
|---|---|
| `manifest` | Its id, name, version, commands, importers, exporters, and whether it lists materials |
| `run-command` | Runs one of its commands with parameter values and returns brushes and entities |
| `list-materials` | Lists the SDK's materials for the browser, with a preview image or color each |
| `import-file` | Reads a file the user picked for one of its importers and returns brushes and entities |
| `export-file` | Writes the map in one of its export formats and returns the file's bytes |

It can call four host functions:

| Import | Purpose |
|---|---|
| `log` | Writes a line to the Output panel |
| `list-files` | Lists files in the SDK's asset folders by extension |
| `read-file` | Reads a file from the SDK's asset folders |
| `read-range` | Reads part of a file from them, for archives too big to read whole |

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
- `hull`: the convex solid around a list of points. Faces get the hull's material, except those whose outward normal is within about 8 degrees of a `surface` hint, which get that hint's material. This is the easy form for generators: list the corners and say which way the special faces point.

An empty material means the material picked in the browser.

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
A plugin with no importers returns an empty list and an error from `import-file`.

## Exporters

A manifest's `exporters` list file formats the plugin writes, each with an id, a label and its extensions.
They appear under File > Export, and compile profiles run them with an `export` step (see [`sdk-format.md`](sdk-format.md)).
`export-file` gets the map as `map-data`: worldspawn's key values, then a flat list of nodes (layers, groups, entities and brushes) where each node names its parent by index and parents always come first.
Every brush comes with its faces (three points, material and texture axes) and its corners.
The editor has already resolved prefabs and mirror copies and removed layers left out of exports and everything outside an active cordon, so an exporter writes what it gets.
It returns the file's bytes, and the editor saves them.

## Materials

A plugin that sets `materials` in its manifest replaces the plain image listing of the SDK's texture folders.
Each material has a name, which is what faces store and what engine importers look up, and a preview: an image path in the asset folders, or an sRGB color.

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

The WIT package is `mapcraft:plugin@0.1.0`.
Until 1.0 the API may change between releases; a plugin built against an older version fails to load with a message naming what is missing.
